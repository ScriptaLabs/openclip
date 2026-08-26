// The port methods all return the shared `OpenClipError`; individual implementations document
// their concrete failure conditions.
#![allow(clippy::missing_errors_doc)]

use std::ops::RangeInclusive;
use std::path::PathBuf;

use thiserror::Error;

use crate::{
    Clip, GameEvent, GameSession, GameStateUpdate, Match, MatchId, Moment, RecordingStatus, Round,
    Settings,
};

#[derive(Debug, Error)]
pub enum OpenClipError {
    #[error("invalid game state: {0}")]
    InvalidGameState(String),
    #[error("component is unavailable: {0}")]
    Unavailable(String),
    #[error("storage operation failed: {0}")]
    Storage(String),
    #[error("capture operation failed: {0}")]
    Capture(String),
}

pub type Result<T> = std::result::Result<T, OpenClipError>;

/// Converts one game's native state payloads into game-agnostic events.
pub trait GameAdapter: Send {
    fn process_state(&mut self, data: &[u8]) -> Result<GameStateUpdate>;
}

/// Owns session/match/round lifecycle without knowing how events were produced.
pub trait MatchManager: Send {
    fn handle_events(&mut self, events: Vec<GameEvent>) -> Result<()>;
    fn active_match(&self) -> Option<&Match>;
}

/// Groups qualifying events into logical, potentially overlapping recording windows.
pub trait MomentDetector: Send {
    fn ingest(&mut self, event: GameEvent) -> Result<Option<Moment>>;
    fn flush(&mut self) -> Option<Moment>;
}

/// Native capture implementations expose lifecycle only; frames never cross into the UI.
pub trait CaptureEngine: Send + Sync {
    fn status(&self) -> RecordingStatus;
    fn start(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
}

/// Encoded replay data is addressed by a match-relative millisecond window.
pub trait ReplayBuffer: Send + Sync {
    fn preserve(&mut self, window_ms: RangeInclusive<u64>) -> Result<PathBuf>;
    fn prune(&mut self) -> Result<()>;
}

/// Persistence boundary used by services and Tauri commands.
pub trait MatchRepository: Send {
    fn list_matches(&self) -> Result<Vec<Match>>;
    fn get_match(&self, id: MatchId) -> Result<Option<Match>>;
    fn save_session(&mut self, session: &GameSession) -> Result<()>;
    fn save_match(&mut self, game_match: &Match) -> Result<()>;
    fn save_round(&mut self, round: &Round) -> Result<()>;
    fn save_event(&mut self, event: &GameEvent) -> Result<()>;
    fn save_moment(&mut self, moment: &Moment) -> Result<()>;
    fn save_moment_with_clip(&mut self, moment: &Moment, clip: &Clip) -> Result<()>;
    fn save_clip(&mut self, clip: &Clip) -> Result<()>;
    fn list_clips(&self) -> Result<Vec<Clip>>;
    fn load_settings(&self) -> Result<Settings>;
    fn save_settings(&mut self, settings: &Settings) -> Result<()>;
    fn apply_game_update(&mut self, update: &GameStateUpdate) -> Result<()>;
}
