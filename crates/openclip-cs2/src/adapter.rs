use chrono::{DateTime, Utc};
use openclip_core::{
    Game, GameAdapter, GameEvent, GameEventId, GameEventType, GameSession, GameSessionId,
    GameStateUpdate, Match, MatchId, MatchMode, OpenClipError, Round, RoundId,
};
use serde_json::{Map, Value, json};
use tracing::{debug, info, warn};

use crate::models::{GsiMap, GsiPayload, MapPhase, RoundPhase};

#[derive(Debug, Default)]
pub struct Cs2Adapter {
    previous: Option<GsiPayload>,
    session: Option<GameSession>,
    active_match: Option<Match>,
    active_round: Option<Round>,
    last_kills: Option<u16>,
    last_round_headshots: Option<u16>,
    last_weapon: Option<WeaponSnapshot>,
    shots_since_last_kill: u16,
    last_payload_at: Option<DateTime<Utc>>,
}

/// A real player cannot score more than this many kills between two GSI updates (they are sent
/// at most every 0.1 s). A larger jump means updates were missed, so it is a resync, not kills.
const MAX_KILLS_PER_UPDATE: u16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WeaponSnapshot {
    name: String,
    ammo_clip: Option<i32>,
}

impl Cs2Adapter {
    /// Parses one partial GSI snapshot and derives durable domain changes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-state error when the bytes are not valid GSI JSON.
    pub fn process(&mut self, data: &[u8]) -> openclip_core::Result<GameStateUpdate> {
        let payload: GsiPayload = serde_json::from_slice(data)
            .map_err(|error| OpenClipError::InvalidGameState(error.to_string()))?;
        Ok(self.process_payload(payload))
    }

    /// Ends a match whose GSI stream went silent (CS2 closed, crashed or lost its connection).
    /// CS2 sends a heartbeat every few seconds, so a long gap means the game is gone.
    #[must_use]
    pub fn close_if_idle(&mut self, now: DateTime<Utc>, idle: chrono::Duration) -> GameStateUpdate {
        let mut update = GameStateUpdate::default();
        let silent = self
            .last_payload_at
            .is_some_and(|last| now.signed_duration_since(last) >= idle);
        if silent && self.active_match.is_some() {
            info!("GSI went silent; closing the active match");
            self.finish_match(now, &mut update);
        }
        update
    }

    #[must_use]
    pub fn active_match(&self) -> Option<&Match> {
        self.active_match.as_ref()
    }

    #[must_use]
    pub fn process_payload(&mut self, payload: GsiPayload) -> GameStateUpdate {
        let timestamp = payload.timestamp();
        let mut update = GameStateUpdate::default();
        self.last_payload_at = Some(timestamp);

        self.ensure_session(&payload, timestamp, &mut update);
        self.update_match_lifecycle(&payload, timestamp, &mut update);

        // The round-winning (and match-winning) kill routinely arrives in the same payload that
        // reports the round as over or the map as game over. Attribute it to the round and match
        // that are still open before closing them, otherwise it is dropped or orphaned from the
        // multi-kill it completes.
        let closing_round =
            payload.round.as_ref().and_then(|round| round.phase) == Some(RoundPhase::Over);
        let game_over = payload.map.as_ref().and_then(|map| map.phase) == Some(MapPhase::GameOver);
        if closing_round || game_over {
            self.track_weapon_usage(&payload);
            self.detect_kills(&payload, timestamp, &mut update);
            self.update_round_lifecycle(&payload, timestamp, &mut update);
            if game_over {
                self.finish_match(timestamp, &mut update);
            }
        } else {
            self.update_round_lifecycle(&payload, timestamp, &mut update);
            self.track_weapon_usage(&payload);
            self.detect_kills(&payload, timestamp, &mut update);
        }

        self.previous = Some(payload);
        update
    }

    fn ensure_session(
        &mut self,
        payload: &GsiPayload,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
    ) {
        let is_cs2 = payload
            .provider
            .as_ref()
            .is_some_and(|provider| provider.appid == Some(730))
            || payload.map_name().is_some();
        if self.session.is_none() && is_cs2 {
            let session = GameSession {
                id: GameSessionId::new(),
                game: Game::CounterStrike2,
                started_at: timestamp,
                ended_at: None,
            };
            info!(session_id = %session.id, "cs2 detected");
            update.sessions.push(session.clone());
            self.session = Some(session);
        }
    }

    fn update_match_lifecycle(
        &mut self,
        payload: &GsiPayload,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
    ) {
        let map = payload.map.as_ref();
        let map_name = payload.normalized_map_name();
        let phase = map.and_then(|map| map.phase);
        let is_live = phase == Some(MapPhase::Live);
        let changed_map = self
            .active_match
            .as_ref()
            .and_then(|game_match| game_match.map.as_deref())
            .zip(map_name.as_deref())
            .is_some_and(|(previous, current)| previous != current);
        let restarted = is_live && self.match_counters_restarted(payload);

        if self.active_match.is_some() && (changed_map || restarted) {
            self.finish_match(timestamp, update);
        }

        if self.active_match.is_none() && is_live {
            self.start_match(payload, timestamp, update);
        }

        if let (Some(active), Some(map)) = (self.active_match.as_mut(), map) {
            let next_score_a = ct_score(map);
            let next_score_b = t_score(map);
            let next_mode = map.mode.as_deref().map_or(active.mode, parse_mode);
            if active.score_team_a != next_score_a
                || active.score_team_b != next_score_b
                || active.mode != next_mode
            {
                active.score_team_a = next_score_a;
                active.score_team_b = next_score_b;
                active.mode = next_mode;
                update.matches.push(active.clone());
            }
        }
    }

    fn start_match(
        &mut self,
        payload: &GsiPayload,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
    ) {
        let Some(session_id) = self.session.as_ref().map(|session| session.id) else {
            return;
        };
        let map = payload.map.as_ref();
        let game_match = Match {
            id: MatchId::new(),
            session_id,
            map: payload.normalized_map_name(),
            mode: map
                .and_then(|map| map.mode.as_deref())
                .map_or(MatchMode::Unknown, parse_mode),
            started_at: timestamp,
            ended_at: None,
            score_team_a: map.and_then(ct_score),
            score_team_b: map.and_then(t_score),
            rounds: Vec::new(),
            moments: Vec::new(),
        };
        let event = event_for(
            &game_match,
            None,
            GameEventType::MatchStart,
            timestamp,
            Some(0),
            json!({"map": game_match.map, "mode": game_match.mode}),
        );
        info!(match_id = %game_match.id, map = ?game_match.map, "match started");
        update.matches.push(game_match.clone());
        update.events.push(event);
        self.active_match = Some(game_match);
        self.last_kills = payload.local_player_stats().and_then(|stats| stats.kills);
        self.last_round_headshots = round_headshots(payload);
    }

    fn match_counters_restarted(&self, payload: &GsiPayload) -> bool {
        let Some(previous_map) = self.previous.as_ref().and_then(|state| state.map.as_ref()) else {
            return false;
        };
        let Some(current_map) = payload.map.as_ref() else {
            return false;
        };
        let round_restarted = previous_map
            .round
            .zip(current_map.round)
            .is_some_and(|(previous, current)| previous > 1 && current <= 1);
        let score_restarted = total_score(previous_map)
            .zip(total_score(current_map))
            .is_some_and(|(previous, current)| previous > 2 && current <= 1);
        let returned_from_game_over = previous_map.phase == Some(MapPhase::GameOver)
            && current_map.phase == Some(MapPhase::Live);
        round_restarted || score_restarted || returned_from_game_over
    }

    fn update_round_lifecycle(
        &mut self,
        payload: &GsiPayload,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
    ) {
        let Some(game_match) = self.active_match.as_ref() else {
            return;
        };
        let match_id = game_match.id;
        let phase = payload.round.as_ref().and_then(|round| round.phase);
        let reported_number = reported_round_number(payload);

        if phase == Some(RoundPhase::Live) {
            let current_number = self.active_round.as_ref().map(|round| round.number);
            if self.active_round.is_some()
                && reported_number.is_some()
                && current_number != reported_number
            {
                self.finish_round(timestamp, update, None);
            }
            if self.active_round.is_none() {
                let number = reported_number.unwrap_or(1);
                let round = Round {
                    id: RoundId::new(),
                    match_id,
                    number,
                    started_at: timestamp,
                    ended_at: None,
                    events: Vec::new(),
                };
                if let Some(game_match) = self.active_match.as_ref() {
                    debug!(match_id = %match_id, round = number, "round started");
                    update.events.push(event_for(
                        game_match,
                        Some(round.id),
                        GameEventType::RoundStart,
                        timestamp,
                        match_offset(game_match, timestamp),
                        json!({"round": number}),
                    ));
                }
                update.rounds.push(round.clone());
                self.active_round = Some(round);
                self.last_round_headshots = round_headshots(payload);
                self.last_weapon = None;
                self.shots_since_last_kill = 0;
            }
        } else if phase == Some(RoundPhase::Over) {
            let winner = payload
                .round
                .as_ref()
                .and_then(|round| round.win_team.as_deref());
            self.finish_round(timestamp, update, winner);
        }
    }

    fn detect_kills(
        &mut self,
        payload: &GsiPayload,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
    ) {
        let Some(game_match) = self.active_match.as_ref() else {
            return;
        };
        let Some(current_kills) = payload.local_player_stats().and_then(|stats| stats.kills) else {
            return;
        };
        let current_headshots = round_headshots(payload);
        let Some(previous_kills) = self.last_kills else {
            self.last_kills = Some(current_kills);
            self.last_round_headshots = current_headshots;
            return;
        };
        if current_kills <= previous_kills {
            self.last_kills = Some(current_kills);
            if let Some(headshots) = current_headshots {
                self.last_round_headshots = Some(headshots);
            }
            return;
        }

        let kill_delta = current_kills - previous_kills;
        if kill_delta > MAX_KILLS_PER_UPDATE {
            warn!(
                match_id = %game_match.id,
                previous_kills,
                current_kills,
                "kill counter jumped further than one update allows; resynchronizing"
            );
            self.last_kills = Some(current_kills);
            if let Some(headshots) = current_headshots {
                self.last_round_headshots = Some(headshots);
            }
            return;
        }
        let headshot_delta = current_headshots
            .zip(self.last_round_headshots)
            .map_or(0, |(current, previous)| current.saturating_sub(previous));
        let round_id = self.active_round.as_ref().map(|round| round.id);
        let weapon = payload.active_weapon().map(str::to_owned);
        let one_shot_one_kill = kill_delta == 1 && self.shots_since_last_kill == 1;
        for index in 0..kill_delta {
            let is_headshot = index >= kill_delta.saturating_sub(headshot_delta);
            let metadata = kill_metadata(
                previous_kills + index + 1,
                is_headshot,
                weapon.as_deref(),
                one_shot_one_kill,
            );
            update.events.push(event_for(
                game_match,
                round_id,
                GameEventType::Kill,
                timestamp,
                match_offset(game_match, timestamp),
                metadata.clone(),
            ));
            if is_headshot {
                update.events.push(event_for(
                    game_match,
                    round_id,
                    GameEventType::Headshot,
                    timestamp,
                    match_offset(game_match, timestamp),
                    metadata,
                ));
            }
        }
        info!(
            match_id = %game_match.id,
            round = self.active_round.as_ref().map(|round| round.number),
            total_kills = current_kills,
            delta = kill_delta,
            "kill detected"
        );
        self.last_kills = Some(current_kills);
        self.shots_since_last_kill = 0;
        if let Some(headshots) = current_headshots {
            self.last_round_headshots = Some(headshots);
        }
    }

    fn track_weapon_usage(&mut self, payload: &GsiPayload) {
        let current = payload.player.as_ref().and_then(|player| {
            player
                .weapons
                .values()
                .find(|weapon| weapon.state.as_deref() == Some("active"))
                .and_then(|weapon| {
                    Some(WeaponSnapshot {
                        name: weapon.name.clone()?,
                        ammo_clip: weapon.ammo_clip,
                    })
                })
        });
        let Some(current) = current else {
            return;
        };
        if let Some(previous) = &self.last_weapon
            && previous.name == current.name
            && let (Some(previous_ammo), Some(current_ammo)) =
                (previous.ammo_clip, current.ammo_clip)
            && current_ammo < previous_ammo
        {
            self.shots_since_last_kill = self
                .shots_since_last_kill
                .saturating_add(u16::try_from(previous_ammo - current_ammo).unwrap_or(u16::MAX));
        } else if self
            .last_weapon
            .as_ref()
            .is_some_and(|previous| previous.name != current.name)
        {
            self.shots_since_last_kill = 0;
        }
        self.last_weapon = Some(current);
    }

    fn finish_round(
        &mut self,
        timestamp: DateTime<Utc>,
        update: &mut GameStateUpdate,
        winner: Option<&str>,
    ) {
        let (Some(mut round), Some(game_match)) =
            (self.active_round.take(), self.active_match.as_ref())
        else {
            return;
        };
        round.ended_at = Some(timestamp);
        update.rounds.push(round.clone());
        update.events.push(event_for(
            game_match,
            Some(round.id),
            GameEventType::RoundEnd,
            timestamp,
            match_offset(game_match, timestamp),
            json!({"round": round.number, "winner": winner}),
        ));
        if let Some(winner) = winner {
            update.events.push(event_for(
                game_match,
                Some(round.id),
                GameEventType::RoundWin,
                timestamp,
                match_offset(game_match, timestamp),
                json!({"round": round.number, "winner": winner}),
            ));
        }
        debug!(match_id = %game_match.id, round = round.number, winner, "round finished");
    }

    fn finish_match(&mut self, timestamp: DateTime<Utc>, update: &mut GameStateUpdate) {
        self.finish_round(timestamp, update, None);
        let Some(mut game_match) = self.active_match.take() else {
            return;
        };
        game_match.ended_at = Some(timestamp);
        update.events.push(event_for(
            &game_match,
            None,
            GameEventType::MatchEnd,
            timestamp,
            match_offset(&game_match, timestamp),
            json!({
                "map": game_match.map,
                "score_team_a": game_match.score_team_a,
                "score_team_b": game_match.score_team_b,
            }),
        ));
        info!(match_id = %game_match.id, map = ?game_match.map, "match finished");
        update.matches.push(game_match);
        self.last_kills = None;
        self.last_round_headshots = None;
        self.last_weapon = None;
        self.shots_since_last_kill = 0;
    }
}

impl GameAdapter for Cs2Adapter {
    fn process_state(&mut self, data: &[u8]) -> openclip_core::Result<GameStateUpdate> {
        self.process(data)
    }
}

fn event_for(
    game_match: &Match,
    round_id: Option<RoundId>,
    event_type: GameEventType,
    timestamp: DateTime<Utc>,
    match_offset_ms: Option<u64>,
    metadata: Value,
) -> GameEvent {
    GameEvent {
        id: GameEventId::new(),
        match_id: game_match.id,
        round_id,
        source_game: Game::CounterStrike2,
        event_type,
        occurred_at: timestamp,
        match_offset_ms,
        metadata,
    }
}

fn kill_metadata(
    total_kills: u16,
    headshot: bool,
    weapon: Option<&str>,
    one_shot_one_kill: bool,
) -> Value {
    let mut metadata = Map::new();
    metadata.insert("total_kills".to_owned(), Value::from(total_kills));
    metadata.insert("headshot".to_owned(), Value::from(headshot));
    metadata.insert("oneShotOneKill".to_owned(), Value::from(one_shot_one_kill));
    if let Some(weapon) = weapon {
        metadata.insert("weapon".to_owned(), Value::from(weapon));
        metadata.insert("knifeKill".to_owned(), Value::from(is_knife_weapon(weapon)));
        metadata.insert("zeusKill".to_owned(), Value::from(is_zeus_weapon(weapon)));
    }
    Value::Object(metadata)
}

fn is_knife_weapon(weapon: &str) -> bool {
    weapon.contains("knife") || weapon.contains("bayonet")
}

fn is_zeus_weapon(weapon: &str) -> bool {
    matches!(weapon, "weapon_taser" | "weapon_zeus")
}

fn match_offset(game_match: &Match, timestamp: DateTime<Utc>) -> Option<u64> {
    u64::try_from((timestamp - game_match.started_at).num_milliseconds()).ok()
}

fn reported_round_number(payload: &GsiPayload) -> Option<u8> {
    payload
        .map
        .as_ref()
        .and_then(|map| map.round)
        .and_then(|round| u8::try_from(round.saturating_add(1)).ok())
}

fn round_headshots(payload: &GsiPayload) -> Option<u16> {
    payload
        .player
        .as_ref()
        .and_then(|player| player.state.as_ref())
        .and_then(|state| state.round_killhs)
}

fn parse_mode(mode: &str) -> MatchMode {
    match mode.to_ascii_lowercase().as_str() {
        "competitive" => MatchMode::Competitive,
        "premier" => MatchMode::Premier,
        "casual" => MatchMode::Casual,
        "deathmatch" => MatchMode::Deathmatch,
        _ => MatchMode::Unknown,
    }
}

fn ct_score(map: &GsiMap) -> Option<u8> {
    map.team_ct
        .as_ref()
        .and_then(|team| team.score)
        .and_then(|score| u8::try_from(score).ok())
}

fn t_score(map: &GsiMap) -> Option<u8> {
    map.team_t
        .as_ref()
        .and_then(|team| team.score)
        .and_then(|score| u8::try_from(score).ok())
}

fn total_score(map: &GsiMap) -> Option<u16> {
    Some(map.team_ct.as_ref()?.score? + map.team_t.as_ref()?.score?)
}
