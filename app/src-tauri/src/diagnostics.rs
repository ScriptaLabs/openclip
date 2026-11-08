//! Anonymous problem reports.
//!
//! When capture goes wrong, the recorder keeps a short list of what happened. After the match (never
//! during it) the app offers to send that list, with an anonymized excerpt of the log, to help fix the
//! problem. Nothing leaves the machine unless the user presses Send, and they can read exactly what
//! would be sent first.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use openclip_capture::{GpuAdapter, RecorderService, describe_system};
use openclip_core::{Incident, IncidentKind, MatchRepository, Settings};
use openclip_storage::SqliteRepository;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::anonymize::{Identity, MAX_LOG_CHARS, fingerprint, scrub};

const POLL: Duration = Duration::from_secs(10);
/// After offering a report (sent or not), wait this long before offering another.
const COOLDOWN: Duration = Duration::from_mins(20);
/// How much of the end of the log file is read.
const LOG_TAIL_BYTES: u64 = 1_500_000;
/// Log lines this far before the first problem are still relevant to it.
const LOG_LEAD: ChronoDuration = ChronoDuration::minutes(20);
const MAX_NOTE_CHARS: usize = 1_000;
const MAX_SUMMARY_CHARS: usize = 500;
const MAX_BACKLOG: usize = 60;

/// Exactly what is sent. The field names are the database column names.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReportPayload {
    pub app_version: String,
    pub os: String,
    pub gpus: Vec<GpuAdapter>,
    pub encoder: Option<String>,
    pub settings: serde_json::Value,
    pub kinds: Vec<IncidentKind>,
    pub fingerprint: String,
    pub summary: String,
    pub log: String,
    pub user_note: Option<String>,
}

/// A report waiting for the user's decision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingReport {
    pub id: String,
    pub incident_count: usize,
    pub payload: ReportPayload,
}

struct Inner {
    app: AppHandle,
    recorder: RecorderService,
    repository: Arc<Mutex<SqliteRepository>>,
    match_active: Box<dyn Fn() -> bool + Send + Sync>,
    backlog: Mutex<Vec<Incident>>,
    pending: Mutex<Option<PendingReport>>,
    quiet_until: Mutex<Option<Instant>>,
}

#[derive(Clone)]
pub(crate) struct DiagnosticsService {
    inner: Arc<Inner>,
}

impl DiagnosticsService {
    pub(crate) fn start(
        app: AppHandle,
        recorder: RecorderService,
        repository: Arc<Mutex<SqliteRepository>>,
        match_active: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> std::io::Result<Self> {
        let service = Self {
            inner: Arc::new(Inner {
                app,
                recorder,
                repository,
                match_active,
                backlog: Mutex::new(Vec::new()),
                pending: Mutex::new(None),
                quiet_until: Mutex::new(None),
            }),
        };
        let worker = service.clone();
        thread::Builder::new()
            .name("openclip-diagnostics".to_owned())
            .spawn(move || {
                loop {
                    thread::sleep(POLL);
                    worker.tick();
                }
            })?;
        Ok(service)
    }

    pub(crate) fn pending(&self) -> Option<PendingReport> {
        lock(&self.inner.pending).clone()
    }

    /// The user declined: forget this batch and stay quiet for a while.
    pub(crate) fn dismiss(&self) {
        *lock(&self.inner.pending) = None;
        lock(&self.inner.backlog).clear();
        self.start_cooldown();
    }

    /// Sends the pending report with the user's optional note. A failure keeps the report so the
    /// user can try again.
    pub(crate) async fn submit(&self, note: Option<String>) -> Result<(), String> {
        let mut report = self.pending().ok_or("there is no report to send")?;
        let identity = Identity::current();
        report.payload.user_note = note
            .map(|note| scrub(note.trim(), &identity))
            .filter(|note| !note.is_empty())
            .map(|note| note.chars().take(MAX_NOTE_CHARS).collect());
        send(&report.payload).await?;
        tracing::info!(kinds = ?report.payload.kinds, "anonymous problem report sent");
        *lock(&self.inner.pending) = None;
        lock(&self.inner.backlog).clear();
        self.start_cooldown();
        Ok(())
    }

    fn start_cooldown(&self) {
        *lock(&self.inner.quiet_until) = Some(Instant::now() + COOLDOWN);
    }

    fn tick(&self) {
        let fresh = self.inner.recorder.take_incidents();
        let wants_prompt = self
            .load_settings()
            .is_some_and(|settings| settings.diagnostics_prompt);
        let mut backlog = lock(&self.inner.backlog);
        if !wants_prompt {
            backlog.clear();
            return;
        }
        backlog.extend(fresh);
        let excess = backlog.len().saturating_sub(MAX_BACKLOG);
        backlog.drain(..excess);
        // Never interrupt a match, and never nag.
        if backlog.is_empty()
            || (self.inner.match_active)()
            || lock(&self.inner.pending).is_some()
            || lock(&self.inner.quiet_until).is_some_and(|until| Instant::now() < until)
            || endpoint().is_none()
        {
            return;
        }
        let incidents = backlog.clone();
        drop(backlog);
        let Some(settings) = self.load_settings() else {
            return;
        };
        let payload = build_report(
            &incidents,
            &settings,
            &Identity::current(),
            &read_log_tail().unwrap_or_default(),
        );
        let report = PendingReport {
            id: uuid::Uuid::new_v4().to_string(),
            incident_count: incidents.len(),
            payload,
        };
        *lock(&self.inner.pending) = Some(report.clone());
        self.start_cooldown();
        tracing::info!(
            incidents = report.incident_count,
            "offering an anonymous problem report"
        );
        if let Err(error) = self.inner.app.emit("diagnostics_pending", report) {
            tracing::warn!(%error, "could not announce the problem report");
        }
    }

    fn load_settings(&self) -> Option<Settings> {
        self.inner
            .repository
            .lock()
            .ok()
            .and_then(|repository| repository.load_settings().ok())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where reports go: the project's REST endpoint and its public (insert-only) key, both baked in by
/// CI like the update channel. Builds without them never offer a report.
fn endpoint() -> Option<(String, &'static str)> {
    let configured = option_env!("OPENCLIP_REALTIME_URL")?;
    let key = option_env!("OPENCLIP_SUPABASE_ANON_KEY")?;
    let host = url::Url::parse(configured).ok()?.host_str()?.to_owned();
    Some((format!("https://{host}/rest/v1/diagnostic_report"), key))
}

async fn send(payload: &ReportPayload) -> Result<(), String> {
    let (url, key) = endpoint().ok_or("reporting is not set up in this build")?;
    let response = reqwest::Client::new()
        .post(url)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .header("Prefer", "return=minimal")
        .timeout(Duration::from_secs(20))
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("could not reach the server: {error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "the server refused the report ({})",
            response.status()
        ))
    }
}

fn log_path() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("APPDATA")?).join("dev.openclip.app");
    Some(directory.join("openclip.log"))
}

fn read_log_tail() -> Option<String> {
    let mut file = std::fs::File::open(log_path()?).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(LOG_TAIL_BYTES)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn build_report(
    incidents: &[Incident],
    settings: &Settings,
    identity: &Identity,
    raw_log: &str,
) -> ReportPayload {
    let system = describe_system();
    let first = incidents.first();
    let mut kinds: Vec<IncidentKind> = Vec::new();
    for incident in incidents {
        if !kinds.contains(&incident.kind) {
            kinds.push(incident.kind);
        }
    }
    let kind_name = |kind: IncidentKind| {
        serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    let first_message = first
        .map(|incident| scrub(&incident.message, identity))
        .unwrap_or_default();
    let more = incidents.len().saturating_sub(1);
    let mut summary: String = first_message.chars().take(MAX_SUMMARY_CHARS).collect();
    if more > 0 {
        summary = format!("{summary} (+{more} more)");
    }
    ReportPayload {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        os: system.windows,
        gpus: dedupe(system.adapters),
        encoder: incidents
            .iter()
            .rev()
            .find_map(|incident| incident.encoder.clone())
            .map(|encoder| scrub(&encoder, identity)),
        settings: settings_summary(settings),
        fingerprint: fingerprint(
            &first
                .map(|incident| kind_name(incident.kind))
                .unwrap_or_default(),
            &first_message,
        ),
        kinds,
        summary,
        log: select_log(raw_log, first.map(|incident| incident.at), identity),
        user_note: None,
    }
}

fn dedupe(adapters: Vec<GpuAdapter>) -> Vec<GpuAdapter> {
    let mut unique: Vec<GpuAdapter> = Vec::new();
    for adapter in adapters {
        if !unique.contains(&adapter) {
            unique.push(adapter);
        }
    }
    unique
}

/// The capture settings that affect whether recording works. No paths, names or keys.
fn settings_summary(settings: &Settings) -> serde_json::Value {
    serde_json::json!({
        "videoEncoder": settings.video_encoder,
        "quality": settings.quality,
        "resolution": settings.resolution,
        "framesPerSecond": settings.frames_per_second,
        "captureGameAudio": settings.capture_game_audio,
        "captureMicrophone": settings.capture_microphone,
        "skipWorkshopMaps": settings.skip_workshop_maps,
        "skipCommunityServers": settings.skip_community_servers,
    })
}

/// The part of the log that explains the problem: warnings and errors, how the encoder started, and a
/// few frame-rate samples, from shortly before the first problem. Every line is anonymized.
fn select_log(raw: &str, first_problem: Option<DateTime<Utc>>, identity: &Identity) -> String {
    let since = first_problem.map(|at| at - LOG_LEAD);
    let mut kept: Vec<(String, u32)> = Vec::new();
    let mut samples: Vec<String> = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let (Some(stamp), Some(rest)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(at) = DateTime::parse_from_rfc3339(stamp) else {
            continue;
        };
        if since.is_some_and(|since| at.with_timezone(&Utc) < since) {
            continue;
        }
        let level = rest;
        let is_problem = level == "WARN" || level == "ERROR";
        let is_context = line.contains("encoder initialized")
            || line.contains("capture target detected")
            || line.contains("capture session failed");
        if line.contains("capture diagnostics") {
            samples.push(scrub(line, identity));
            if samples.len() > 3 {
                samples.remove(0);
            }
            continue;
        }
        if !is_problem && !is_context {
            continue;
        }
        let cleaned = scrub(line, identity);
        // A failure that repeats every few seconds is shown once, with a count.
        let body = cleaned
            .split_once(' ')
            .map_or(cleaned.as_str(), |(_, body)| body);
        if let Some((last, count)) = kept.last_mut()
            && last.split_once(' ').map_or(last.as_str(), |(_, body)| body) == body
        {
            *count += 1;
            continue;
        }
        kept.push((cleaned, 1));
    }
    let mut lines: Vec<String> = kept
        .into_iter()
        .map(|(line, count)| {
            if count > 1 {
                format!("{line}  (x{count})")
            } else {
                line
            }
        })
        .collect();
    lines.extend(samples);
    let text = lines.join("\n");
    let excess = text.chars().count().saturating_sub(MAX_LOG_CHARS);
    text.chars().skip(excess).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity::from_terms(["alexandra".to_owned()])
    }

    #[test]
    fn the_log_excerpt_keeps_problems_and_hides_who_you_are() {
        let raw = "\
2026-11-02T20:00:00.000000Z  INFO openclip_cs2::adapter: kill detected match_id=9f22559a-9d08-48a2-9b09-9e02856b03e8 round=3
2026-11-02T20:01:00.000000Z  INFO openclip_capture::windows: continuous encoder initialized width=1920 backend=AMD H.264 Encoder MFT hardware=true
2026-11-02T20:02:00.000000Z  WARN openclip_capture::windows: encoder failed error=AMD returned no video path=C:\\Users\\alexandra\\Videos\\x.mp4
2026-11-02T20:02:05.000000Z  WARN openclip_capture::windows: encoder failed error=AMD returned no video path=C:\\Users\\alexandra\\Videos\\x.mp4
2026-11-02T20:02:10.000000Z ERROR openclip_capture::windows: capture session failed error=boom
";
        let first = DateTime::parse_from_rfc3339("2026-11-02T20:02:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let log = select_log(raw, Some(first), &identity());
        assert!(
            !log.contains("kill detected"),
            "routine lines are dropped:\n{log}"
        );
        assert!(log.contains("encoder initialized"), "{log}");
        assert!(log.contains("(x2)"), "repeats are collapsed:\n{log}");
        assert!(log.contains("capture session failed"), "{log}");
        assert!(!log.to_lowercase().contains("alexandra"), "{log}");
        assert!(!log.contains("C:\\"), "{log}");
    }

    #[test]
    fn lines_long_before_the_problem_are_left_out() {
        let raw = "2026-11-01T01:00:00.000000Z  WARN openclip_capture::windows: old problem\n\
                   2026-11-02T20:02:00.000000Z  WARN openclip_capture::windows: new problem\n";
        let first = DateTime::parse_from_rfc3339("2026-11-02T20:02:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let log = select_log(raw, Some(first), &identity());
        assert!(
            log.contains("new problem") && !log.contains("old problem"),
            "{log}"
        );
    }

    #[test]
    fn a_report_has_a_summary_a_fingerprint_and_no_note_until_the_user_adds_one() {
        let incidents = vec![
            Incident {
                at: Utc::now(),
                kind: IncidentKind::EncoderFailed,
                message: r"AMD H.264 Encoder MFT returned no video after 180 pictures at C:\Users\alexandra\x".to_owned(),
                encoder: Some("AMD H.264 Encoder MFT".to_owned()),
            },
            Incident {
                at: Utc::now(),
                kind: IncidentKind::ClipSaveFailed,
                message: "the replay buffer has no finalized segments".to_owned(),
                encoder: None,
            },
        ];
        let report = build_report(&incidents, &Settings::default(), &identity(), "");
        assert_eq!(
            report.kinds,
            vec![IncidentKind::EncoderFailed, IncidentKind::ClipSaveFailed]
        );
        assert!(
            report.summary.contains("returned no video") && report.summary.ends_with("(+1 more)")
        );
        assert!(!report.summary.to_lowercase().contains("alexandra"));
        assert_eq!(report.encoder.as_deref(), Some("AMD H.264 Encoder MFT"));
        assert_eq!(report.fingerprint.len(), 16);
        assert!(report.user_note.is_none());
        assert_eq!(report.settings["videoEncoder"], "auto");
    }

    /// Prints a real report as JSON, to check it against the database: `cargo test -p openclip-app
    /// print_sample_report -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints a sample"]
    fn print_sample_report() {
        let incidents = vec![Incident {
            at: Utc::now(),
            kind: IncidentKind::EncoderFailed,
            message: "probe from the OpenClip test suite".to_owned(),
            encoder: Some("Probe encoder".to_owned()),
        }];
        let report = build_report(&incidents, &Settings::default(), &identity(), "");
        println!("REPORT_JSON={}", serde_json::to_string(&report).unwrap());
    }
}
