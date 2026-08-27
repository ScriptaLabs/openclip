// Tauri command extraction requires owned `State` wrappers and deserialized command arguments.
#![allow(clippy::needless_pass_by_value)]

mod automatic;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use openclip_capture::{AudioDevice, RecorderService, list_microphone_devices};
use openclip_core::{
    Clip, GameEventType, GameStateUpdate, Match, MatchId, MatchRepository, Moment, RecordingStatus,
    Settings,
};
use openclip_cs2::{
    ConfigInstallResult, Cs2Install, Cs2Status, GSI_CONFIG_FILE_NAME, GsiEndpoint, GsiService,
    SteamLocator, install_gsi_config, render_gsi_config, serve,
};
use openclip_storage::SqliteRepository;
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

use automatic::{AutomaticMomentResult, AutomaticMomentService};

type SharedService = Arc<GsiService<SqliteRepository>>;

struct AppState {
    repository: Arc<Mutex<SqliteRepository>>,
    gsi: SharedService,
    recorder: RecorderService,
    automatic: AutomaticMomentService,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Cs2SetupStatus {
    found: bool,
    configured: bool,
    install: Option<Cs2Install>,
    message: String,
}

#[tauri::command]
fn get_recording_status(state: State<'_, AppState>) -> RecordingStatus {
    state.recorder.status()
}

fn save_and_persist_replay(state: &AppState) -> Result<Clip, String> {
    let clip = state
        .recorder
        .save_replay(state.gsi.status().active_match_id)
        .map_err(|error| error.to_string())?;
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .save_clip(&clip)
        .map_err(|error| error.to_string())?;
    Ok(clip)
}

#[tauri::command]
fn save_replay(state: State<'_, AppState>) -> Result<Clip, String> {
    save_and_persist_replay(&state)
}

#[tauri::command]
fn get_clips(state: State<'_, AppState>) -> Result<Vec<Clip>, String> {
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .list_clips()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_microphone_devices() -> Result<Vec<AudioDevice>, String> {
    list_microphone_devices().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_cs2_status(state: State<'_, AppState>) -> Cs2Status {
    state.gsi.status()
}

#[tauri::command]
fn get_cs2_setup_status() -> Cs2SetupStatus {
    match SteamLocator::discover() {
        Ok(install) => {
            let path = install.cfg_dir.join(GSI_CONFIG_FILE_NAME);
            let configured = std::fs::read_to_string(&path)
                .is_ok_and(|contents| contents == render_gsi_config(GsiEndpoint::default()));
            Cs2SetupStatus {
                found: true,
                configured,
                install: Some(install),
                message: if configured {
                    "CS2 GSI is configured for OpenClip".to_owned()
                } else {
                    "CS2 was found; install the OpenClip GSI configuration".to_owned()
                },
            }
        }
        Err(error) => Cs2SetupStatus {
            found: false,
            configured: false,
            install: None,
            message: error.to_string(),
        },
    }
}

#[tauri::command]
fn install_cs2_gsi_config() -> Result<ConfigInstallResult, String> {
    let install = SteamLocator::discover().map_err(|error| error.to_string())?;
    install_gsi_config(&install, GsiEndpoint::default()).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_games(state: State<'_, AppState>) -> Result<Vec<Match>, String> {
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .list_matches()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_game(id: MatchId, state: State<'_, AppState>) -> Result<Option<Match>, String> {
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_game_moments(id: MatchId, state: State<'_, AppState>) -> Result<Vec<Moment>, String> {
    Ok(state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(id)
        .map_err(|error| error.to_string())?
        .map_or_else(Vec::new, |game_match| game_match.moments))
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .load_settings()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_settings(
    mut settings: Settings,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manual_replay_shortcut = settings.manual_replay_shortcut.trim().to_owned();
    settings.manual_replay_shortcut = manual_replay_shortcut;
    if settings.manual_replay_shortcut.is_empty() {
        return Err("choose a manual replay shortcut".to_owned());
    }
    let mut repository = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?;
    let previous = repository
        .load_settings()
        .map_err(|error| error.to_string())?;
    if previous.manual_replay_shortcut != settings.manual_replay_shortcut {
        replace_manual_replay_shortcut(
            &app,
            &previous.manual_replay_shortcut,
            &settings.manual_replay_shortcut,
        )?;
    }
    if let Err(error) = repository.save_settings(&settings) {
        if previous.manual_replay_shortcut != settings.manual_replay_shortcut {
            let _ = replace_manual_replay_shortcut(
                &app,
                &settings.manual_replay_shortcut,
                &previous.manual_replay_shortcut,
            );
        }
        return Err(error.to_string());
    }
    drop(repository);
    state.recorder.update_settings(settings.clone());
    state.automatic.update_settings(settings);
    Ok(())
}

fn replace_manual_replay_shortcut(
    app: &tauri::AppHandle,
    previous: &str,
    next: &str,
) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    shortcuts
        .register(next)
        .map_err(|error| format!("could not register {next}: {error}"))?;
    if let Err(error) = shortcuts.unregister(previous) {
        let _ = shortcuts.unregister(next);
        return Err(format!("could not replace {previous}: {error}"));
    }
    Ok(())
}

#[tauri::command]
fn set_storage_path(path: PathBuf, state: State<'_, AppState>) -> Result<Settings, String> {
    let path = validate_storage_directory(&path)?;
    let mut repository = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?;
    let mut settings = repository
        .load_settings()
        .map_err(|error| error.to_string())?;
    settings.storage_path = path;
    repository
        .save_settings(&settings)
        .map_err(|error| error.to_string())?;
    drop(repository);
    state.recorder.update_settings(settings.clone());
    state.automatic.update_settings(settings.clone());
    Ok(settings)
}

#[tauri::command]
fn open_moment_clip(
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let game_match = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(match_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "match was not found".to_owned())?;
    let path = game_match
        .moments
        .iter()
        .find(|moment| moment.id == moment_id)
        .and_then(|moment| moment.clip_path.as_ref())
        .ok_or_else(|| "this moment has no captured video".to_owned())?;
    if !path.is_file() {
        return Err("the captured video file is no longer available".to_owned());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_moment_thumbnail(
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<u8>, String> {
    let game_match = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(match_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "match was not found".to_owned())?;
    let clip = game_match
        .moments
        .iter()
        .find(|moment| moment.id == moment_id)
        .and_then(|moment| moment.clip_path.as_ref())
        .ok_or_else(|| "this moment has no captured video".to_owned())?;
    if !clip.is_file() {
        return Err("the captured video file is no longer available".to_owned());
    }
    let thumbnail = jpeg_thumbnail_path(clip);
    if !thumbnail.is_file() {
        generate_jpeg_thumbnail(&app, clip, &thumbnail).map_err(|error| {
            tracing::warn!(%error, %moment_id, path = %clip.display(), "could not generate requested clip thumbnail");
            error
        })?;
    }
    let bytes = std::fs::read(&thumbnail).map_err(|error| error.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("the generated thumbnail exceeded the safety limit".to_owned());
    }
    Ok(bytes)
}

fn jpeg_thumbnail_path(clip: &Path) -> PathBuf {
    let mut name = clip.file_stem().unwrap_or_default().to_os_string();
    name.push(".thumbnail.jpg");
    clip.with_file_name(name)
}

fn generate_jpeg_thumbnail(
    app: &tauri::AppHandle,
    clip: &Path,
    thumbnail: &Path,
) -> Result<(), String> {
    let partial = thumbnail.with_extension("jpg.partial");
    let ffmpeg = bundled_ffmpeg_path(app);
    let mut command = Command::new(&ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "1"])
        .arg("-i")
        .arg(clip)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=480:-2",
            "-c:v",
            "mjpeg",
            "-q:v",
            "3",
            "-f",
            "image2",
        ])
        .arg(&partial)
        .output()
        .map_err(|error| {
            format!(
                "could not start bundled FFmpeg at {}: {error}",
                ffmpeg.display()
            )
        })?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "thumbnail generation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    std::fs::rename(&partial, thumbnail).map_err(|error| error.to_string())
}

#[tauri::command]
fn export_interesting_moments(
    match_id: MatchId,
    destination: PathBuf,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PathBuf>, String> {
    let destination = validate_storage_directory(&destination)?;
    let game_match = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(match_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "match was not found".to_owned())?;
    let watermark_enabled = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .load_settings()
        .map_err(|error| error.to_string())?
        .watermark_enabled;
    let watermark = watermark_enabled
        .then(|| watermark_asset_path(&app))
        .transpose()?;
    let mut exported = Vec::new();
    for moment in game_match.moments.iter().filter(|moment| {
        moment.clip_path.is_some() && (moment.importance >= 10.0 || moment.is_favorite)
    }) {
        let source = moment
            .clip_path
            .as_ref()
            .expect("filtered captured moments");
        if !source.is_file() {
            tracing::warn!(path = %source.display(), moment_id = %moment.id, "export source is missing");
            continue;
        }
        let round = moment
            .round_id
            .and_then(|id| game_match.rounds.iter().find(|round| round.id == id))
            .map_or(0, |round| round.number);
        let map = sanitize_filename_component(
            game_match
                .map
                .as_deref()
                .unwrap_or("Unknown")
                .trim_start_matches("de_"),
        );
        let classification = format!("{:?}", moment.classification);
        let occurred_at = moment
            .events
            .first()
            .map_or(game_match.started_at, |event| event.occurred_at);
        let base = format!(
            "{}_R{round:02}_{}_{}",
            title_case(&map),
            split_classification(&classification),
            occurred_at.format("%Y-%m-%d_%H%M%S")
        );
        let target = available_export_path(&destination, &base, "mp4");
        if let Some(watermark) = &watermark {
            render_watermarked_export(&app, source, &target, watermark)?;
        } else {
            std::fs::copy(source, &target).map_err(|error| error.to_string())?;
        }
        exported.push(target);
    }
    Ok(exported)
}

fn watermark_asset_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let packaged = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?
        .join("assets/branding/watermark.mov");
    let development =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/branding/watermark.mov");
    [packaged, development]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "the OpenClip watermark asset is missing".to_owned())
}

fn render_watermarked_export(
    app: &tauri::AppHandle,
    source: &Path,
    target: &Path,
    watermark: &Path,
) -> Result<(), String> {
    let media =
        openclip_capture::validate_finalized_segment(source).map_err(|error| error.to_string())?;
    // The final logo occupies only part of the transparent 400 px canvas, so a 25% canvas
    // width produces a clearly visible (but still restrained) mark at 1080p.
    let watermark_width = watermark_canvas_width(media.width);
    let filter = format!(
        "[1:v]format=rgba,scale={watermark_width}:-2,tpad=stop_mode=clone:stop_duration=86400[wm];[0:v][wm]overlay=W-w-W*0.025:H*0.025:shortest=1[v]"
    );
    let partial = target.with_extension("watermarked.mp4.partial");
    let ffmpeg = bundled_ffmpeg_path(app);
    let mut command = Command::new(&ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .arg("-i")
        .arg(watermark)
        .args(["-filter_complex", &filter, "-map", "[v]", "-map", "0:a?"])
        .args([
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p5",
            "-cq",
            "18",
            "-b:v",
            "0",
        ])
        .args([
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
            "-shortest",
            "-f",
            "mp4",
        ])
        .arg(&partial)
        .output()
        .map_err(|error| {
            format!(
                "could not start bundled FFmpeg at {}: {error}",
                ffmpeg.display()
            )
        })?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "watermark export failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    std::fs::rename(&partial, target).map_err(|error| error.to_string())?;
    Ok(())
}

fn bundled_ffmpeg_path(app: &tauri::AppHandle) -> PathBuf {
    if let Some(override_path) = std::env::var_os("OPENCLIP_FFMPEG") {
        return PathBuf::from(override_path);
    }
    let packaged = app
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("bin/ffmpeg.exe"));
    let development =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dependencies/ffmpeg/ffmpeg.exe");
    packaged
        .into_iter()
        .chain([development])
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn watermark_canvas_width(video_width: u32) -> u32 {
    (video_width.saturating_mul(25) / 100).max(2) & !1
}

fn validate_storage_directory(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("choose a storage folder".to_owned());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let probe = path.join(format!(".openclip-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| format!("OpenClip cannot write to {}: {error}", path.display()))?;
    std::fs::remove_file(&probe)
        .map_err(|error| format!("could not finish validating {}: {error}", path.display()))?;
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("could not resolve {}: {error}", path.display()))
    }
}

fn sanitize_filename_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ if character.is_control() => '_',
            _ => character,
        })
        .collect::<String>();
    sanitized.trim().trim_matches('.').to_owned()
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    characters.next().map_or_else(
        || "Unknown".to_owned(),
        |first| first.to_uppercase().collect::<String>() + characters.as_str(),
    )
}

fn split_classification(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .fold(String::new(), |mut output, (index, character)| {
            if index > 0 && character.is_uppercase() {
                output.push('-');
            }
            output.push(character);
            output
        })
}

fn available_export_path(directory: &Path, base: &str, extension: &str) -> PathBuf {
    let initial = directory.join(format!("{base}.{extension}"));
    if !initial.exists() {
        return initial;
    }
    for suffix in 2_u32.. {
        let candidate = directory.join(format!("{base} ({suffix}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the finite filesystem cannot contain every numeric suffix")
}

fn emit_game_updates(app: &tauri::AppHandle, update: &GameStateUpdate) {
    if let Err(error) = app.emit("gsi_update", update.clone()) {
        tracing::warn!(%error, "could not emit GSI update");
    }
    if !update.sessions.is_empty()
        && let Err(error) = app.emit("game_detected", update.sessions[0].clone())
    {
        tracing::warn!(%error, "could not emit game detection event");
    }
    for game_match in &update.matches {
        if let Err(error) = app.emit("match_updated", game_match.clone()) {
            tracing::warn!(%error, "could not emit match update");
        }
    }
    for event in &update.events {
        let name = match event.event_type {
            GameEventType::MatchStart => Some("match_started"),
            GameEventType::MatchEnd => Some("match_finished"),
            GameEventType::RoundStart => Some("round_started"),
            GameEventType::RoundEnd => Some("round_finished"),
            _ => None,
        };
        if let Some(name) = name
            && let Err(error) = app.emit(name, event.clone())
        {
            tracing::warn!(%error, event = name, "could not emit lifecycle event");
        }
    }
}

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let repository = Arc::new(Mutex::new(
        SqliteRepository::open(data_dir.join("openclip.db"))
            .map_err(|error| std::io::Error::other(error.to_string()))?,
    ));
    let settings = repository
        .lock()
        .map_err(|_| std::io::Error::other("database lock was poisoned"))?
        .load_settings()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let recorder = RecorderService::new(&data_dir, settings.clone())
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let app_handle = app.handle().clone();
    let notification_app = app.handle().clone();
    let notify = Arc::new(move |result: AutomaticMomentResult| {
        if let Err(error) = notification_app.emit("automatic_moment_finalized", result) {
            tracing::warn!(%error, "could not emit automatic moment result");
        }
    });
    let automatic = AutomaticMomentService::start(
        repository.clone(),
        recorder.clone(),
        settings.clone(),
        notify,
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    let automatic_observer = automatic.clone();
    let observer = Arc::new(move |update: &GameStateUpdate| {
        emit_game_updates(&app_handle, update);
        automatic_observer.observe(update);
    });
    let gsi = Arc::new(GsiService::with_observer(repository.clone(), observer));
    app.manage(AppState {
        repository,
        gsi: gsi.clone(),
        recorder: recorder.clone(),
        automatic,
    });

    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_shortcuts([settings.manual_replay_shortcut.as_str()])?
            .with_handler(|app, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let app = app.clone();
                let shortcut = format!("{shortcut:?}");
                tauri::async_runtime::spawn_blocking(move || {
                    let state = app.state::<AppState>();
                    match save_and_persist_replay(&state) {
                        Ok(clip) => {
                            tracing::info!(path = %clip.path.display(), %shortcut, "manual replay save completed");
                            if let Err(error) = app.emit("replay_saved", clip) {
                                tracing::warn!(%error, "could not emit replay saved event");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, %shortcut, "manual replay save was not available");
                        }
                    }
                });
            })
            .build(),
    )?;
    recorder
        .start()
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    let handler: Arc<dyn openclip_cs2::GsiPayloadHandler> = gsi.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(GsiEndpoint::default(), handler).await {
            tracing::error!(%error, "CS2 GSI listener stopped");
            gsi.set_listener_error(error.to_string());
        }
    });
    tracing::info!(path = %data_dir.display(), "OpenClip metadata store ready");
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openclip=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(setup_app)
        .invoke_handler(tauri::generate_handler![
            get_recording_status,
            save_replay,
            get_clips,
            get_microphone_devices,
            get_cs2_status,
            get_cs2_setup_status,
            install_cs2_gsi_config,
            get_games,
            get_game,
            get_game_moments,
            get_settings,
            update_settings,
            set_storage_path,
            open_moment_clip,
            get_moment_thumbnail,
            export_interesting_moments
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run OpenClip: {error}");
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_are_windows_safe() {
        assert_eq!(
            sanitize_filename_component("de:mirage/one?"),
            "de_mirage_one_"
        );
        assert_eq!(split_classification("TripleKill"), "Triple-Kill");
    }

    #[test]
    fn duplicate_exports_receive_deterministic_suffixes() {
        let directory = tempfile::tempdir().expect("temp directory");
        std::fs::write(directory.path().join("Mirage_R07_Triple-Kill.mp4"), b"one").unwrap();
        std::fs::write(
            directory.path().join("Mirage_R07_Triple-Kill (2).mp4"),
            b"two",
        )
        .unwrap();
        assert_eq!(
            available_export_path(directory.path(), "Mirage_R07_Triple-Kill", "mp4"),
            directory.path().join("Mirage_R07_Triple-Kill (3).mp4")
        );
    }

    #[test]
    fn storage_validation_creates_and_probes_directory() {
        let parent = tempfile::tempdir().expect("temp directory");
        let requested = parent.path().join("clips");
        let validated = validate_storage_directory(&requested).expect("writable path");
        assert!(validated.is_dir());
        assert!(validated.read_dir().unwrap().next().is_none());
    }

    #[test]
    fn watermark_canvas_scales_to_one_quarter_of_the_clip() {
        assert_eq!(watermark_canvas_width(1_920), 480);
        assert_eq!(watermark_canvas_width(1_280), 320);
    }

    #[test]
    fn jpeg_thumbnail_is_adjacent_to_its_clip() {
        assert_eq!(
            jpeg_thumbnail_path(Path::new(r"C:\Clips\ace.mp4")),
            PathBuf::from(r"C:\Clips\ace.thumbnail.jpg")
        );
    }
}
