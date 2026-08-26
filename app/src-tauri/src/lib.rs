// Tauri command extraction requires owned `State` wrappers and deserialized command arguments.
#![allow(clippy::needless_pass_by_value)]

use std::sync::Mutex;

use openclip_core::{Match, MatchId, MatchRepository, RecordingState, RecordingStatus, Settings};
use openclip_storage::SqliteRepository;
use tauri::{Manager, State};

struct AppState {
    repository: Mutex<SqliteRepository>,
}

#[tauri::command]
fn get_recording_status() -> RecordingStatus {
    RecordingStatus {
        state: RecordingState::WaitingForGame,
        game: None,
        message: Some("Waiting for CS2".to_owned()),
    }
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
        .map_err(|error| error.to_string())
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
            let repository = SqliteRepository::open(data_dir.join("openclip.db"))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(AppState {
                repository: Mutex::new(repository),
            });
            tracing::info!(path = %data_dir.display(), "OpenClip metadata store ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_recording_status,
            get_games,
            get_game,
            get_settings,
            update_settings
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run OpenClip: {error}");
        });
}
