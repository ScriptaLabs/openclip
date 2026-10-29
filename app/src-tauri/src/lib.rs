// Tauri command extraction requires owned `State` wrappers and deserialized command arguments.
#![allow(clippy::needless_pass_by_value)]

mod automatic;
mod overlay;
mod realtime;
mod updater;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use openclip_capture::{AudioDevice, RecorderService, list_microphone_devices};
use openclip_core::{
    Clip, GameEventType, GameStateUpdate, Match, MatchId, MatchRepository, Moment, RecordingState,
    RecordingStatus, Settings,
};
use openclip_cs2::{
    ConfigInstallResult, Cs2Install, Cs2Status, GSI_CONFIG_FILE_NAME, GsiEndpoint, GsiService,
    SteamLocator, install_gsi_config, render_gsi_config, serve,
};
use openclip_storage::SqliteRepository;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

use automatic::{AutomaticMomentResult, AutomaticMomentService};

type SharedService = Arc<GsiService<SqliteRepository>>;

struct AppState {
    repository: Arc<Mutex<SqliteRepository>>,
    gsi: SharedService,
    recorder: RecorderService,
    automatic: AutomaticMomentService,
    updates: updater::UpdateService,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Cs2SetupStatus {
    found: bool,
    configured: bool,
    install: Option<Cs2Install>,
    message: String,
}

/// CS2 sends a heartbeat every few seconds; a long silence means the game closed or crashed, so
/// the match is finished instead of staying open forever.
fn spawn_idle_match_watcher(gsi: &SharedService) -> std::io::Result<()> {
    let idle_gsi = gsi.clone();
    std::thread::Builder::new()
        .name("openclip-gsi-idle".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                if let Err(error) = idle_gsi.close_idle_match(chrono::Duration::seconds(60)) {
                    tracing::warn!(%error, "could not close an idle match");
                }
            }
        })
        .map(|_| ())
}

/// Starts the background updater and, when a release channel is configured, its push listener.
fn start_updates(
    app: &tauri::AppHandle,
    repository: &Arc<Mutex<SqliteRepository>>,
    gsi: &SharedService,
    recorder: &RecorderService,
) -> updater::UpdateService {
    let settings_repository = repository.clone();
    let status_gsi = gsi.clone();
    let status_recorder = recorder.clone();
    let policy = updater::Policy {
        auto_update: Box::new(move || {
            settings_repository
                .lock()
                .ok()
                .and_then(|repository| repository.load_settings().ok())
                .is_none_or(|settings| settings.auto_update)
        }),
        safe_to_install: Box::new(move || {
            status_gsi.status().state != openclip_cs2::Cs2ConnectionState::MatchInProgress
                && status_recorder.status().state != RecordingState::SavingReplay
        }),
    };
    let updates = updater::UpdateService::new(app.clone(), policy);

    let (nudge, nudges) = tokio::sync::mpsc::unbounded_channel();
    tauri::async_runtime::spawn(updates.clone().run(nudges));
    if let Some(url) = release_channel_url() {
        tauri::async_runtime::spawn(realtime::watch(url, nudge));
    } else {
        tracing::info!("no release channel configured; updates are checked on a timer");
    }
    updates
}

/// Launched by Windows at sign-in (`--background`): stay in the tray instead of opening a window.
fn show_window_unless_background(app: &tauri::App) {
    if std::env::args().any(|argument| argument == "--background") {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn close_to_tray_enabled(app: &tauri::AppHandle) -> bool {
    app.state::<AppState>()
        .repository
        .lock()
        .ok()
        .and_then(|repository| repository.load_settings().ok())
        .is_none_or(|settings| settings.close_to_tray)
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OpenClip", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit OpenClip", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("OpenClip")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "check" => {
                let updates = app.state::<AppState>().updates.clone();
                tauri::async_runtime::spawn(async move { updates.check(true).await });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// The release channel is public by design (an anon key behind row level security) and is baked
/// into the build by CI. Anything installed from it must still carry a valid updater signature.
fn release_channel_url() -> Option<url::Url> {
    option_env!("OPENCLIP_REALTIME_URL").and_then(|configured| {
        realtime::websocket_url(configured, option_env!("OPENCLIP_SUPABASE_ANON_KEY"))
    })
}

#[tauri::command]
fn get_update_status(state: State<'_, AppState>) -> updater::UpdateStatus {
    state.updates.status()
}

#[tauri::command]
async fn check_for_update(state: State<'_, AppState>) -> Result<updater::UpdateStatus, String> {
    let updates = state.updates.clone();
    updates.check(true).await;
    Ok(updates.status())
}

#[tauri::command]
async fn update_now(state: State<'_, AppState>) -> Result<(), String> {
    let updates = state.updates.clone();
    updates.update_now().await;
    Ok(())
}

#[tauri::command]
fn get_launch_at_startup(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_launch_at_startup(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|error| error.to_string())
}

/// Settings "Test overlay": shows a synthetic quad kill, previewing the newest saved clip.
#[tauri::command]
fn preview_overlay(
    status: automatic::CaptureStatus,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (mut settings, newest_clip) = {
        let repository = state
            .repository
            .lock()
            .map_err(|_| "database lock was poisoned".to_owned())?;
        let settings = repository
            .load_settings()
            .map_err(|error| error.to_string())?;
        let newest_clip = repository
            .list_clips()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|clip| clip.path.is_file())
            .max_by_key(|clip| clip.created_at)
            .map(|clip| clip.path);
        (settings, newest_clip)
    };
    // The button is a test of the overlay itself, so it ignores the on/off switch.
    settings.show_clip_notifications = true;
    overlay::present(&app, &settings, &overlay::demo_result(status, newest_clip));
    Ok(())
}

/// Called by the overlay window once its last notification has animated out.
#[tauri::command]
fn hide_overlay(app: tauri::AppHandle) {
    overlay::hide(&app);
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
    settings.sound_volume = settings.sound_volume.min(100);
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipMedia {
    clip_path: String,
    thumbnail_path: Option<String>,
}

/// Verbatim (`\\?\`) prefixes are valid on disk but not in the URLs the webview builds.
fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// A moment's clip file, and how many seconds into it the first kill happens (when known).
fn find_moment_clip(
    state: &AppState,
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
) -> Result<(PathBuf, Option<f64>), String> {
    let game_match = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .get_match(match_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "match was not found".to_owned())?;
    let moment = game_match
        .moments
        .into_iter()
        .find(|moment| moment.id == moment_id)
        .ok_or_else(|| "moment was not found".to_owned())?;
    let first_kill_seconds = moment
        .events
        .iter()
        .find(|event| event.event_type == GameEventType::Kill)
        .and_then(|event| event.match_offset_ms)
        .map(|offset| offset.saturating_sub(moment.start_offset_ms))
        .map(|millis| std::time::Duration::from_millis(millis).as_secs_f64());
    let path = moment
        .clip_path
        .ok_or_else(|| "this moment has no captured video".to_owned())?;
    if path.is_file() {
        Ok((path, first_kill_seconds))
    } else {
        Err("the captured video file is no longer available".to_owned())
    }
}

/// Grants the webview read access to exactly one clip and its thumbnail, and returns their paths
/// for `convertFileSrc`. The asset protocol scope stays empty otherwise.
// Runs on the async pool: generating a thumbnail spawns FFmpeg and must not block the UI thread.
#[tauri::command(async)]
fn prepare_clip_media(
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ClipMedia, String> {
    let (clip, first_kill_seconds) = find_moment_clip(&state, match_id, moment_id)?;
    let scope = app.asset_protocol_scope();
    scope.allow_file(&clip).map_err(|error| error.to_string())?;
    let thumbnail = jpeg_thumbnail_path(&clip);
    if !thumbnail.is_file()
        && let Err(error) = generate_jpeg_thumbnail(&app, &clip, &thumbnail, first_kill_seconds)
    {
        tracing::warn!(%error, %moment_id, path = %clip.display(), "could not generate clip thumbnail");
    }
    let thumbnail_path = if thumbnail.is_file() && scope.allow_file(&thumbnail).is_ok() {
        Some(display_path(&thumbnail))
    } else {
        None
    };
    Ok(ClipMedia {
        clip_path: display_path(&clip),
        thumbnail_path,
    })
}

#[tauri::command]
fn set_moment_favorite(
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut repository = state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?;
    let mut moment = repository
        .get_match(match_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "match was not found".to_owned())?
        .moments
        .into_iter()
        .find(|moment| moment.id == moment_id)
        .ok_or_else(|| "moment was not found".to_owned())?;
    moment.is_favorite = favorite;
    repository
        .save_moment(&moment)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn reveal_moment_clip(
    match_id: MatchId,
    moment_id: openclip_core::MomentId,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (clip, _) = find_moment_clip(&state, match_id, moment_id)?;
    app.opener()
        .reveal_item_in_dir(clip)
        .map_err(|error| error.to_string())
}

fn jpeg_thumbnail_path(clip: &Path) -> PathBuf {
    let mut name = clip.file_stem().unwrap_or_default().to_os_string();
    name.push(".thumb.jpg");
    clip.with_file_name(name)
}

fn generate_jpeg_thumbnail(
    app: &tauri::AppHandle,
    clip: &Path,
    thumbnail: &Path,
    first_kill_seconds: Option<f64>,
) -> Result<(), String> {
    // The frame at the first kill is the most recognizable one; fall back to earlier moments if
    // the seek lands past the end of a short clip.
    let mut attempts: Vec<f64> = Vec::new();
    if let Some(seconds) = first_kill_seconds {
        attempts.push((seconds - 0.3).max(0.0));
    }
    attempts.extend([1.0, 0.0]);
    let mut last_error = String::new();
    for seek in attempts {
        match extract_frame(app, clip, thumbnail, seek) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn extract_frame(
    app: &tauri::AppHandle,
    clip: &Path,
    thumbnail: &Path,
    seek_seconds: f64,
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
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{seek_seconds:.2}"))
        .arg("-i")
        .arg(clip)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=640:-2",
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
    // A seek past the end exits successfully without writing a frame.
    if !output.status.success() || !partial.is_file() {
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
    let notification_repository = repository.clone();
    let notify = Arc::new(move |result: AutomaticMomentResult| {
        let overlay_settings = notification_repository
            .lock()
            .ok()
            .and_then(|repository| repository.load_settings().ok());
        if let Some(settings) = overlay_settings {
            overlay::present(&notification_app, &settings, &result);
        }
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
    let updates = start_updates(app.handle(), &repository, &gsi, &recorder);
    app.manage(AppState {
        repository,
        gsi: gsi.clone(),
        recorder: recorder.clone(),
        automatic,
        updates,
    });

    // Only after the state is managed. Creating another WebView2 window pumps messages, which
    // lets the main window's first commands run; they must find the state already registered.
    overlay::create(app.handle());
    setup_tray(app)?;
    show_window_unless_background(app);

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

    spawn_idle_match_watcher(&gsi)?;

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

/// Logs to the console in development. A release build has no console window, so it writes to
/// `%APPDATA%\dev.openclip.app\openclip.log` instead (rotated at 2 MB) to keep problems diagnosable.
fn init_logging() {
    const DEFAULT_FILTER: &str = "warn,openclip_app_lib=info,openclip_capture=info,openclip_cs2=info,openclip_buffer=info,openclip_storage=info";
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| DEFAULT_FILTER.into());
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match release_log_file() {
        Some(file) => builder
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        None => builder.init(),
    }
}

fn release_log_file() -> Option<std::fs::File> {
    if cfg!(debug_assertions) {
        return None;
    }
    let directory = PathBuf::from(std::env::var_os("APPDATA")?).join("dev.openclip.app");
    std::fs::create_dir_all(&directory).ok()?;
    let path = directory.join("openclip.log");
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 2_000_000) {
        let _ = std::fs::rename(&path, directory.join("openclip.previous.log"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    tauri::Builder::default()
        // Must be first: a second launch (for example the Start menu while OpenClip is in the
        // tray) brings the running window forward instead of starting another recorder.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _directory| {
                show_main_window(app);
            },
        ))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("OpenClip")
                .args(["--background"])
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && close_to_tray_enabled(window.app_handle())
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(setup_app)
        .invoke_handler(tauri::generate_handler![
            get_update_status,
            check_for_update,
            update_now,
            get_launch_at_startup,
            set_launch_at_startup,
            hide_overlay,
            preview_overlay,
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
            prepare_clip_media,
            set_moment_favorite,
            reveal_moment_clip,
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
            PathBuf::from(r"C:\Clips\ace.thumb.jpg")
        );
    }
}
