//! Silent background updates.
//!
//! New versions are found at startup, every few hours, and whenever the release channel nudges
//! (see `realtime`). A found update is downloaded in the background and verified against the
//! public key baked into the app by the updater plugin. It is then installed only when that cannot
//! hurt a recording: never while a match is running or a clip is being saved.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::mpsc::UnboundedReceiver;

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(15);
const CHECK_INTERVAL: Duration = Duration::from_hours(6);
const SAFE_POLL: Duration = Duration::from_secs(20);
/// How often a waiting update looks for a newer release even without an announcement.
const RECHECK_WHILE_READY: Duration = Duration::from_mins(5);
const PROGRESS_STEP: f32 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Idle,
    Checking,
    /// Found, but automatic updates are off. Waiting for the user.
    Available,
    Downloading,
    /// Downloaded and verified; waiting for a safe moment to install.
    Ready,
    Installing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateStatus {
    pub phase: Phase,
    pub current_version: String,
    pub available_version: Option<String>,
    pub notes: Option<String>,
    /// 0 to 1 while downloading.
    pub progress: f32,
    pub last_checked: Option<String>,
    pub error: Option<String>,
}

/// Decisions that depend on the rest of the app, supplied by the host.
pub(crate) struct Policy {
    pub auto_update: Box<dyn Fn() -> bool + Send + Sync>,
    /// True when restarting now cannot interrupt a recording or a clip being saved.
    pub safe_to_install: Box<dyn Fn() -> bool + Send + Sync>,
}

struct Staged {
    update: Update,
    bytes: Vec<u8>,
}

struct Inner {
    status: UpdateStatus,
    staged: Option<Staged>,
}

#[derive(Clone)]
pub(crate) struct UpdateService {
    app: AppHandle,
    inner: Arc<Mutex<Inner>>,
    policy: Arc<Policy>,
}

fn emit_progress(previous: f32, next: f32) -> bool {
    next >= 1.0 || next - previous >= PROGRESS_STEP
}

impl UpdateService {
    pub(crate) fn new(app: AppHandle, policy: Policy) -> Self {
        let status = UpdateStatus {
            phase: Phase::Idle,
            current_version: app.package_info().version.to_string(),
            available_version: None,
            notes: None,
            progress: 0.0,
            last_checked: None,
            error: None,
        };
        Self {
            app,
            inner: Arc::new(Mutex::new(Inner {
                status,
                staged: None,
            })),
            policy: Arc::new(policy),
        }
    }

    pub(crate) fn status(&self) -> UpdateStatus {
        self.inner.lock().map_or_else(
            |poisoned| poisoned.into_inner().status.clone(),
            |inner| inner.status.clone(),
        )
    }

    fn update_status(&self, change: impl FnOnce(&mut UpdateStatus)) {
        let snapshot = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            change(&mut inner.status);
            inner.status.clone()
        };
        if let Err(error) = self.app.emit("update_status", snapshot) {
            tracing::debug!(%error, "could not emit update status");
        }
    }

    fn fail(&self, error: impl std::fmt::Display, quiet: bool) {
        tracing::warn!(%error, "update check failed");
        self.update_status(|status| {
            status.phase = Phase::Idle;
            status.error = (!quiet).then(|| error.to_string());
        });
    }

    /// Looks for a newer version and, unless the user turned automatic updates off, stages it.
    /// `manual` surfaces errors in the UI; background checks stay quiet.
    pub(crate) async fn check(&self, manual: bool) {
        if matches!(
            self.status().phase,
            Phase::Checking | Phase::Downloading | Phase::Installing | Phase::Ready
        ) {
            return;
        }
        self.update_status(|status| {
            status.phase = Phase::Checking;
            status.error = None;
        });
        let updater = match self.app.updater() {
            Ok(updater) => updater,
            Err(error) => return self.fail(error, !manual),
        };
        let checked_at = chrono::Utc::now().to_rfc3339();
        let found = match updater.check().await {
            Ok(found) => found,
            Err(error) => return self.fail(error, !manual),
        };
        let Some(update) = found else {
            self.update_status(|status| {
                status.phase = Phase::Idle;
                status.available_version = None;
                status.notes = None;
                status.last_checked = Some(checked_at);
            });
            return;
        };
        let version = update.version.clone();
        let notes = update.body.clone();
        tracing::info!(%version, "update found");
        self.update_status(|status| {
            status.available_version = Some(version);
            status.notes = notes;
            status.last_checked = Some(checked_at);
            status.phase = Phase::Available;
        });
        if (self.policy.auto_update)() || manual {
            self.stage(update).await;
        }
    }

    /// A newer release can be announced while an older one waits for a safe moment. Download the
    /// newer one in its place, so a single restart reaches the latest version.
    async fn replace_if_newer(&self) {
        let Ok(updater) = self.app.updater() else {
            return;
        };
        let found = match updater.check().await {
            Ok(Some(update)) => update,
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(%error, "could not look for a newer update while one is waiting");
                return;
            }
        };
        let waiting = self.status().available_version;
        if !replaces_staged(waiting.as_deref(), &found.version) {
            return;
        }
        tracing::info!(version = %found.version, "newer update found while another is waiting");
        self.stage(found).await;
    }

    async fn stage(&self, update: Update) {
        let version = update.version.clone();
        let notes = update.body.clone();
        self.update_status(|status| {
            status.available_version = Some(version);
            status.notes = notes;
            status.phase = Phase::Downloading;
            status.progress = 0.0;
        });
        let mut received = 0_u64;
        let mut reported = 0.0_f32;
        let service = self.clone();
        let result = update
            .download(
                move |chunk, total| {
                    received += chunk as u64;
                    if let Some(total) = total.filter(|total| *total > 0) {
                        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                        let fraction = (received as f64 / total as f64).min(1.0) as f32;
                        if emit_progress(reported, fraction) {
                            reported = fraction;
                            service.update_status(|status| status.progress = fraction);
                        }
                    }
                },
                || {},
            )
            .await;
        match result {
            Ok(bytes) => {
                self.inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .staged = Some(Staged { update, bytes });
                self.update_status(|status| {
                    status.phase = Phase::Ready;
                    status.progress = 1.0;
                });
            }
            Err(error) => self.fail(error, false),
        }
    }

    /// Installs the staged update and relaunches. On Windows this exits the process.
    pub(crate) fn install(&self) {
        let staged = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .staged
            .take();
        let Some(Staged { update, bytes }) = staged else {
            return;
        };
        self.update_status(|status| status.phase = Phase::Installing);
        tracing::info!(version = %update.version, "installing update");
        if let Err(error) = update.install(bytes) {
            self.fail(error, false);
        }
    }

    /// The user asked to update right now.
    pub(crate) async fn update_now(&self) {
        if self.status().phase != Phase::Ready {
            self.check(true).await;
        }
        if self.status().phase == Phase::Ready {
            self.install();
        }
    }

    /// Runs for the life of the app.
    pub(crate) async fn run(self, mut nudges: UnboundedReceiver<()>) {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        let mut announced = false;
        let mut last_recheck = Instant::now();
        loop {
            match self.status().phase {
                Phase::Ready => {
                    // Never install a release that has already been superseded: when a new one is
                    // announced (or every few minutes while waiting), fetch the newest first.
                    if announced || last_recheck.elapsed() >= RECHECK_WHILE_READY {
                        self.replace_if_newer().await;
                        last_recheck = Instant::now();
                    }
                    if self.status().phase == Phase::Ready
                        && (self.policy.auto_update)()
                        && (self.policy.safe_to_install)()
                    {
                        self.install();
                    }
                }
                _ => self.check(false).await,
            }
            let wait = if self.status().phase == Phase::Ready {
                SAFE_POLL
            } else {
                CHECK_INTERVAL
            };
            // A release being announced wakes this loop right away, whatever the timer said.
            announced = tokio::select! {
                () = tokio::time::sleep(wait) => false,
                nudge = nudges.recv() => {
                    if nudge.is_none() {
                        return;
                    }
                    true
                }
            };
        }
    }
}

/// Whether a release found while another one waits should take its place. The updater only
/// reports releases newer than the installed version, so any different version is a newer one.
fn replaces_staged(staged: Option<&str>, found: &str) -> bool {
    staged != Some(found)
}

#[cfg(test)]
mod tests {
    use super::{emit_progress, replaces_staged};

    #[test]
    fn a_newer_release_replaces_the_one_that_is_waiting() {
        assert!(replaces_staged(Some("0.1.3"), "0.1.4"));
        assert!(replaces_staged(None, "0.1.4"));
    }

    #[test]
    fn the_release_that_is_already_waiting_is_not_downloaded_again() {
        assert!(!replaces_staged(Some("0.1.4"), "0.1.4"));
    }

    #[test]
    fn progress_is_reported_in_steps_and_always_at_the_end() {
        assert!(!emit_progress(0.0, 0.01));
        assert!(emit_progress(0.0, 0.02));
        assert!(!emit_progress(0.5, 0.51));
        assert!(emit_progress(0.99, 1.0));
    }
}
