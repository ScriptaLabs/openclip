use std::collections::HashMap;

use chrono::{DateTime, Utc};
use openclip_core::MatchId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiPayload {
    pub provider: Option<GsiProvider>,
    pub map: Option<GsiMap>,
    pub round: Option<GsiRound>,
    pub player: Option<GsiPlayer>,
    pub previously: Option<GsiDelta>,
    pub added: Option<GsiDelta>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiProvider {
    pub name: Option<String>,
    pub appid: Option<u32>,
    pub version: Option<u32>,
    pub steamid: Option<String>,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiMap {
    pub mode: Option<String>,
    pub name: Option<String>,
    pub phase: Option<MapPhase>,
    pub round: Option<u16>,
    pub team_ct: Option<GsiMapTeam>,
    pub team_t: Option<GsiMapTeam>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiMapTeam {
    pub score: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapPhase {
    Warmup,
    Live,
    Intermission,
    #[serde(rename = "gameover")]
    GameOver,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiRound {
    pub phase: Option<RoundPhase>,
    pub win_team: Option<String>,
    pub bomb: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoundPhase {
    #[serde(rename = "freezetime")]
    FreezeTime,
    Live,
    Over,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiPlayer {
    pub steamid: Option<String>,
    pub name: Option<String>,
    pub team: Option<String>,
    pub activity: Option<PlayerActivity>,
    pub state: Option<GsiPlayerState>,
    pub match_stats: Option<GsiMatchStats>,
    #[serde(default)]
    pub weapons: HashMap<String, GsiWeapon>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerActivity {
    Playing,
    Menu,
    TextInput,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiPlayerState {
    pub health: Option<u16>,
    pub armor: Option<u16>,
    pub helmet: Option<bool>,
    pub flashed: Option<u16>,
    pub smoked: Option<u16>,
    pub burning: Option<u16>,
    pub money: Option<u32>,
    pub round_kills: Option<u16>,
    pub round_killhs: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiMatchStats {
    pub kills: Option<u16>,
    pub assists: Option<u16>,
    pub deaths: Option<u16>,
    pub mvps: Option<u16>,
    pub score: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiWeapon {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub weapon_type: Option<String>,
    pub state: Option<String>,
    pub ammo_clip: Option<i32>,
    pub ammo_reserve: Option<i32>,
}

/// CS2 may include typed changed fragments under `previously` and `added`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiDelta {
    pub map: Option<GsiMapDelta>,
    pub round: Option<GsiRoundDelta>,
    pub player: Option<GsiPlayerDelta>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiMapDelta {
    pub name: Option<String>,
    pub phase: Option<MapPhase>,
    pub round: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiRoundDelta {
    pub phase: Option<RoundPhase>,
    pub win_team: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiPlayerDelta {
    pub steamid: Option<String>,
    pub state: Option<GsiPlayerState>,
    pub match_stats: Option<GsiMatchStats>,
}

impl GsiPayload {
    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        self.provider
            .as_ref()
            .and_then(|provider| provider.timestamp)
            .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
            .unwrap_or_else(Utc::now)
    }

    #[must_use]
    pub fn map_name(&self) -> Option<&str> {
        self.map
            .as_ref()?
            .name
            .as_deref()
            .filter(|name| !name.is_empty())
    }

    #[must_use]
    pub fn active_weapon(&self) -> Option<&str> {
        self.player
            .as_ref()?
            .weapons
            .values()
            .find(|weapon| weapon.state.as_deref() == Some("active"))?
            .name
            .as_deref()
    }

    #[must_use]
    pub fn local_player_stats(&self) -> Option<&GsiMatchStats> {
        let player = self.player.as_ref()?;
        let provider_steam_id = self
            .provider
            .as_ref()
            .and_then(|provider| provider.steamid.as_deref());
        let is_local = match (provider_steam_id, player.steamid.as_deref()) {
            (Some(provider), Some(player)) => provider == player,
            _ => player.activity == Some(PlayerActivity::Playing),
        };
        is_local.then_some(player.match_stats.as_ref()).flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cs2ConnectionState {
    WaitingForGame,
    Connected,
    MatchInProgress,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cs2Status {
    pub state: Cs2ConnectionState,
    pub map: Option<String>,
    pub active_match_id: Option<MatchId>,
    pub last_update_at: Option<DateTime<Utc>>,
    pub message: String,
}

impl Default for Cs2Status {
    fn default() -> Self {
        Self {
            state: Cs2ConnectionState::WaitingForGame,
            map: None,
            active_match_id: None,
            last_update_at: None,
            message: "Waiting for Counter-Strike 2".to_owned(),
        }
    }
}
