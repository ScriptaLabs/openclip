use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(GameSessionId);
id_type!(MatchId);
id_type!(RoundId);
id_type!(GameEventId);
id_type!(MomentId);
id_type!(ClipId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Game {
    CounterStrike2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSession {
    pub id: GameSessionId,
    pub game: Game,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    Competitive,
    Premier,
    Casual,
    Deathmatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub id: MatchId,
    pub session_id: GameSessionId,
    pub map: Option<String>,
    pub mode: MatchMode,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub score_team_a: Option<u8>,
    pub score_team_b: Option<u8>,
    #[serde(default)]
    pub rounds: Vec<Round>,
    #[serde(default)]
    pub moments: Vec<Moment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    pub id: RoundId,
    pub match_id: MatchId,
    pub number: u8,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub events: Vec<GameEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameEventType {
    MatchStart,
    MatchEnd,
    RoundStart,
    RoundEnd,
    Kill,
    Death,
    Headshot,
    BombPlant,
    BombDefuse,
    RoundWin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameEvent {
    pub id: GameEventId,
    pub match_id: MatchId,
    pub round_id: Option<RoundId>,
    pub source_game: Game,
    pub event_type: GameEventType,
    pub occurred_at: DateTime<Utc>,
    /// Milliseconds since the match began, when a reliable match clock is known.
    pub match_offset_ms: Option<u64>,
    /// Game-specific facts that are useful to classifiers but not required by the core model.
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentClassification {
    Kill,
    Headshot,
    DoubleKill,
    TripleKill,
    QuadKill,
    FiveKill,
    /// Retained for reading pre-Milestone-4 databases; new moments use exact-count labels.
    MultiKill,
    Clutch,
    Ace,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub id: MomentId,
    pub match_id: MatchId,
    pub round_id: Option<RoundId>,
    pub start_offset_ms: u64,
    pub end_offset_ms: u64,
    #[serde(default)]
    pub events: Vec<GameEvent>,
    pub classification: MomentClassification,
    pub importance: f32,
    #[serde(default)]
    pub clip_id: Option<ClipId>,
    pub clip_path: Option<PathBuf>,
    #[serde(default)]
    pub is_favorite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSource {
    Manual,
    AutomaticMoment,
}

/// A saved media file. Manual replays intentionally do not masquerade as detected moments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: ClipId,
    pub match_id: Option<MatchId>,
    pub moment_id: Option<MomentId>,
    pub source: ClipSource,
    pub created_at: DateTime<Utc>,
    pub path: PathBuf,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u16,
    pub codec: String,
    pub encoder: String,
    pub has_game_audio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    Idle,
    WaitingForGame,
    Initializing,
    Recording,
    SavingReplay,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub state: RecordingState,
    pub game: Option<Game>,
    pub message: Option<String>,
    #[serde(default)]
    pub encoder: Option<String>,
    #[serde(default)]
    pub capture_width: Option<u32>,
    #[serde(default)]
    pub capture_height: Option<u32>,
    #[serde(default)]
    pub capture_fps: Option<u16>,
    #[serde(default)]
    pub buffered_seconds: u16,
}

/// One atomic set of domain changes produced from a game-state transition.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStateUpdate {
    #[serde(default)]
    pub sessions: Vec<GameSession>,
    #[serde(default)]
    pub matches: Vec<Match>,
    #[serde(default)]
    pub rounds: Vec<Round>,
    #[serde(default)]
    pub events: Vec<GameEvent>,
}

impl GameStateUpdate {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.matches.is_empty()
            && self.rounds.is_empty()
            && self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        assert_ne!(MatchId::new(), MatchId::new());
    }

    #[test]
    fn external_json_uses_camel_case_fields_and_snake_case_values() {
        let status = RecordingStatus {
            state: RecordingState::WaitingForGame,
            game: Some(Game::CounterStrike2),
            message: None,
            encoder: None,
            capture_width: None,
            capture_height: None,
            capture_fps: None,
            buffered_seconds: 0,
        };
        let json = serde_json::to_value(status).expect("status should serialize");
        assert_eq!(json["state"], "waiting_for_game");
        assert_eq!(json["game"], "counter_strike2");
        assert!(json.get("message").is_some());
    }
}
