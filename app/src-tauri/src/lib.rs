// Tauri command extraction requires owned `State` wrappers and deserialized command arguments.
#![allow(clippy::needless_pass_by_value)]

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

type SharedService = Arc<GsiService<SqliteRepository>>;

struct AppState {
    repository: Arc<Mutex<SqliteRepository>>,
    gsi: SharedService,
    recorder: RecorderService,
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
    state.recorder.update_settings(settings);
    Ok(())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openclip=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let repository = Arc::new(Mutex::new(
                SqliteRepository::open(data_dir.join("openclip.db"))
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            ));
            let app_handle = app.handle().clone();
            let observer = Arc::new(move |update: &GameStateUpdate| {
                emit_game_updates(&app_handle, update);
            });
            let gsi = Arc::new(GsiService::with_observer(repository.clone(), observer));
            let recorder = RecorderService::new(
                &data_dir,
                repository
                    .lock()
                    .map_err(|_| std::io::Error::other("database lock was poisoned"))?
                    .load_settings()
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(AppState {
                repository,
                gsi: gsi.clone(),
                recorder: recorder.clone(),
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
                                Err(error) => tracing::warn!(%error, "F8 replay save was not available"),
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
        })
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
            update_settings
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run OpenClip: {error}");
        });
}
