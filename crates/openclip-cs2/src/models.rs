use std::collections::HashMap;

use chrono::{DateTime, Utc};
use openclip_core::{MatchId, MatchOrigin};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

/// Community and FACEIT-style servers report names like `DE_MIRAGE | RETAKE #61 | TR | XPLAY.GG`
/// or `workshop/3070923343/de_test`. The engine map name is what identifies the map.
#[must_use]
pub fn normalize_map_name(raw: &str) -> String {
    const PREFIXES: [&str; 5] = ["de_", "cs_", "ar_", "dz_", "gd_"];
    let lower = raw.to_ascii_lowercase();
    if let Some(token) = lower
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .find(|token| PREFIXES.iter().any(|prefix| token.starts_with(prefix)))
    {
        return token.to_owned();
    }
    let first = lower.split('|').next().unwrap_or_default().trim();
    first.rsplit('/').next().unwrap_or(first).trim().to_owned()
}

/// Name prefixes of the maps Valve ships. Matchmaking, `FACEIT` and most community servers all use
/// these, so a map without one is a custom map.
const OFFICIAL_MAP_PREFIXES: [&str; 6] = ["de_", "cs_", "ar_", "dz_", "gd_", "rush_"];

/// Works out where a match is played from the map name the game reports.
///
/// The game state never says which server it is, so this reads what servers leave behind:
/// - `workshop/<id>/<map>` is a workshop map;
/// - a name with extras such as `DE_MIRAGE | RETAKE #61 | TR` was renamed by a community server;
/// - a plain name without an official prefix (`aim_botz`) is a custom map.
///
/// Matchmaking and `FACEIT` report a plain official name and are `Official`. A community server
/// that keeps a plain official name and mode looks the same as matchmaking and cannot be told apart.
#[must_use]
pub fn classify_origin(raw: &str) -> MatchOrigin {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        // No name to judge by, so nothing is hidden.
        return MatchOrigin::Official;
    }
    if lower.starts_with("workshop/") || lower.starts_with("workshop\\") {
        return MatchOrigin::Workshop;
    }
    let plain = lower
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !plain {
        return MatchOrigin::Community;
    }
    if OFFICIAL_MAP_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        MatchOrigin::Official
    } else {
        MatchOrigin::Workshop
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GsiPayload {
    pub provider: Option<GsiProvider>,
    pub map: Option<GsiMap>,
    pub round: Option<GsiRound>,
    pub player: Option<GsiPlayer>,
    #[serde(default, deserialize_with = "deserialize_optional_lossy")]
    pub previously: Option<GsiDelta>,
    #[serde(default, deserialize_with = "deserialize_optional_lossy")]
    pub added: Option<GsiDelta>,
}

/// CS2's `previously`/`added` fragments are best-effort hints and occasionally
/// contain boolean sentinels or mixed leaf types. A malformed delta must not
/// discard the authoritative current map/player state in the same payload.
fn deserialize_optional_lossy<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(value).ok())
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

    /// The map as a plain engine name (see [`normalize_map_name`]), stable across server renames.
    #[must_use]
    pub fn normalized_map_name(&self) -> Option<String> {
        self.map_name().map(normalize_map_name)
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

#[cfg(test)]
mod tests {
    use super::GsiPayload;

    #[test]
    fn malformed_delta_does_not_discard_current_state() {
        let payload: GsiPayload = serde_json::from_str(
            r#"{
                "map": { "name": "workshop/3070923343/de_test", "phase": "live" },
                "player": { "match_stats": { "kills": 4 } },
                "previously": { "round": { "win_team": true } },
                "added": true
            }"#,
        )
        .expect("authoritative state remains parseable");

        assert_eq!(payload.map_name(), Some("workshop/3070923343/de_test"));
        assert_eq!(payload.normalized_map_name().as_deref(), Some("de_test"));
        assert_eq!(payload.player.unwrap().match_stats.unwrap().kills, Some(4));
        assert!(payload.previously.is_none());
        assert!(payload.added.is_none());
    }
}

#[cfg(test)]
mod map_name_tests {
    use openclip_core::MatchOrigin;

    use super::{classify_origin, normalize_map_name};

    #[test]
    fn workshop_and_community_maps_are_told_apart_from_official_ones() {
        assert_eq!(classify_origin("de_mirage"), MatchOrigin::Official);
        assert_eq!(classify_origin("cs_office"), MatchOrigin::Official);
        assert_eq!(classify_origin("rush_001"), MatchOrigin::Official);
        assert_eq!(
            classify_origin("workshop/3070923343/de_test"),
            MatchOrigin::Workshop
        );
        assert_eq!(
            classify_origin("workshop/123/aim_botz"),
            MatchOrigin::Workshop
        );
        assert_eq!(classify_origin("aim_botz"), MatchOrigin::Workshop);
        assert_eq!(
            classify_origin("DE_MIRAGE | RETAKE #61 | TR | XPLAY.GG"),
            MatchOrigin::Community
        );
        assert_eq!(
            classify_origin("Aim Botz | 128 tick"),
            MatchOrigin::Community
        );
        assert_eq!(classify_origin(""), MatchOrigin::Official);
    }

    #[test]
    fn server_style_names_reduce_to_the_engine_map() {
        assert_eq!(
            normalize_map_name("DE_MIRAGE | RETAKE #61 | TR | XPLAY.GG"),
            "de_mirage"
        );
        assert_eq!(normalize_map_name("de_inferno"), "de_inferno");
        assert_eq!(normalize_map_name("workshop/3070923343/de_test"), "de_test");
        assert_eq!(normalize_map_name("Aim Botz | 128 tick"), "aim botz");
        assert_eq!(normalize_map_name("workshop/123/aim_botz"), "aim_botz");
    }
}
