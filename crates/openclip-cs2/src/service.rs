use std::sync::{Arc, Mutex, RwLock};

use openclip_core::{GameStateUpdate, MatchRepository, OpenClipError};

use crate::adapter::Cs2Adapter;
use crate::listener::GsiPayloadHandler;
use crate::models::{Cs2ConnectionState, Cs2Status, GsiPayload};

type UpdateObserver = dyn Fn(&GameStateUpdate) + Send + Sync;

pub struct GsiService<R: MatchRepository> {
    adapter: Mutex<Cs2Adapter>,
    repository: Arc<Mutex<R>>,
    status: RwLock<Cs2Status>,
    observer: Option<Arc<UpdateObserver>>,
}

impl<R: MatchRepository> GsiService<R> {
    #[must_use]
    pub fn new(repository: Arc<Mutex<R>>) -> Self {
        Self {
            adapter: Mutex::new(Cs2Adapter::default()),
            repository,
            status: RwLock::new(Cs2Status::default()),
            observer: None,
        }
    }

    #[must_use]
    pub fn with_observer(repository: Arc<Mutex<R>>, observer: Arc<UpdateObserver>) -> Self {
        Self {
            observer: Some(observer),
            ..Self::new(repository)
        }
    }

    #[must_use]
    pub fn status(&self) -> Cs2Status {
        self.status
            .read()
            .map_or_else(|_| poisoned_status(), |status| status.clone())
    }

    pub fn set_listener_error(&self, error: impl Into<String>) {
        if let Ok(mut status) = self.status.write() {
            status.state = Cs2ConnectionState::Error;
            status.message = error.into();
        }
    }

    /// Closes the active match when the GSI stream has been silent for `idle`.
    ///
    /// # Errors
    ///
    /// Returns an error when the adapter or database lock is poisoned, or the update cannot be stored.
    pub fn close_idle_match(&self, idle: chrono::Duration) -> openclip_core::Result<()> {
        let update = self
            .adapter
            .lock()
            .map_err(|_| OpenClipError::Unavailable("CS2 adapter lock was poisoned".to_owned()))?
            .close_if_idle(chrono::Utc::now(), idle);
        if update.is_empty() {
            return Ok(());
        }
        self.repository
            .lock()
            .map_err(|_| OpenClipError::Storage("database lock was poisoned".to_owned()))?
            .apply_game_update(&update)?;
        if let Some(observer) = &self.observer {
            observer(&update);
        }
        if let Ok(mut status) = self.status.write() {
            status.state = Cs2ConnectionState::WaitingForGame;
            status.map = None;
            status.active_match_id = None;
            "Waiting for Counter-Strike 2".clone_into(&mut status.message);
        }
        Ok(())
    }

    fn process(&self, data: &[u8]) -> openclip_core::Result<()> {
        let payload: GsiPayload = serde_json::from_slice(data)
            .map_err(|error| OpenClipError::InvalidGameState(error.to_string()))?;
        let timestamp = payload.timestamp();
        let map = payload.normalized_map_name();
        let mut adapter = self
            .adapter
            .lock()
            .map_err(|_| OpenClipError::Unavailable("CS2 adapter lock was poisoned".to_owned()))?;
        let update = adapter.process_payload(payload);
        let active_match_id = adapter.active_match().map(|game_match| game_match.id);
        drop(adapter);

        if !update.is_empty() {
            self.repository
                .lock()
                .map_err(|_| OpenClipError::Storage("database lock was poisoned".to_owned()))?
                .apply_game_update(&update)?;
            if let Some(observer) = &self.observer {
                observer(&update);
            }
        }

        if let Ok(mut status) = self.status.write() {
            status.state = if active_match_id.is_some() {
                Cs2ConnectionState::MatchInProgress
            } else {
                Cs2ConnectionState::Connected
            };
            status.map = map;
            status.active_match_id = active_match_id;
            status.last_update_at = Some(timestamp);
            status.message = status.map.as_ref().map_or_else(
                || "Connected to Counter-Strike 2".to_owned(),
                |map| {
                    if active_match_id.is_some() {
                        format!("{} · Match in progress", display_map(map))
                    } else {
                        format!("{} · Connected", display_map(map))
                    }
                },
            );
        }
        Ok(())
    }
}

impl<R: MatchRepository + 'static> GsiPayloadHandler for GsiService<R> {
    fn handle_payload(&self, data: &[u8]) -> openclip_core::Result<()> {
        self.process(data)
    }
}

fn poisoned_status() -> Cs2Status {
    Cs2Status {
        state: Cs2ConnectionState::Error,
        message: "CS2 status lock was poisoned".to_owned(),
        ..Cs2Status::default()
    }
}

fn display_map(map: &str) -> String {
    let name = map.strip_prefix("de_").unwrap_or(map);
    let mut characters = name.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + characters.as_str()
    })
}
