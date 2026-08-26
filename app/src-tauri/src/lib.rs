// Tauri command extraction requires owned `State` wrappers and deserialized command arguments.
#![allow(clippy::needless_pass_by_value)]

mod automatic;

use std::path::{Path, PathBuf};
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
use tauri_plugin_global_shortcut::ShortcutState;
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
fn update_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    state
        .repository
        .lock()
        .map_err(|_| "database lock was poisoned".to_owned())?
        .save_settings(&settings)
        .map_err(|error| error.to_string())?;
    state.recorder.update_settings(settings.clone());
    state.automatic.update_settings(settings);
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
fn export_interesting_moments(
    match_id: MatchId,
    destination: PathBuf,
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
        std::fs::copy(source, &target).map_err(|error| error.to_string())?;
        exported.push(target);
    }
    Ok(exported)
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
    let automatic =
        AutomaticMomentService::start(repository.clone(), recorder.clone(), settings, notify)
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
            .with_shortcuts(["F8"])?
            .with_handler(|app, _shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let state = app.state::<AppState>();
                    match save_and_persist_replay(&state) {
                        Ok(clip) => {
                            tracing::info!(path = %clip.path.display(), "F8 replay save completed");
                            if let Err(error) = app.emit("replay_saved", clip) {
                                tracing::warn!(%error, "could not emit replay saved event");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "F8 replay save was not available");
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
}
