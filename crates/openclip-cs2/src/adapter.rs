use chrono::{DateTime, Utc};
use openclip_core::{
    Game, GameAdapter, GameEvent, GameEventId, GameEventType, GameSession, GameSessionId,
    GameStateUpdate, Match, MatchId, MatchMode, OpenClipError, Round, RoundId,
};
use serde_json::{Map, Value, json};
use tracing::{debug, info};

use crate::models::{GsiMap, GsiPayload, MapPhase, RoundPhase};

#[derive(Debug, Default)]
pub struct Cs2Adapter {
    previous: Option<GsiPayload>,
    session: Option<GameSession>,
    active_match: Option<Match>,
    active_round: Option<Round>,
    last_kills: Option<u16>,
    last_round_headshots: Option<u16>,
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

    #[must_use]
    pub fn active_match(&self) -> Option<&Match> {
        self.active_match.as_ref()
    }

    #[must_use]
    pub fn process_payload(&mut self, payload: GsiPayload) -> GameStateUpdate {
        let timestamp = payload.timestamp();
        let mut update = GameStateUpdate::default();

        self.ensure_session(&payload, timestamp, &mut update);
        self.update_match_lifecycle(&payload, timestamp, &mut update);
        self.update_round_lifecycle(&payload, timestamp, &mut update);
        self.detect_kills(&payload, timestamp, &mut update);

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
        let map_name = payload.map_name();
        let phase = map.and_then(|map| map.phase);
        let is_live = phase == Some(MapPhase::Live);
        let changed_map = self
            .active_match
            .as_ref()
            .and_then(|game_match| game_match.map.as_deref())
            .zip(map_name)
            .is_some_and(|(previous, current)| previous != current);
        let restarted = is_live && self.match_counters_restarted(payload);

        if self.active_match.is_some() && (changed_map || restarted) {
            self.finish_match(timestamp, update);
        }

        if self.active_match.is_none() && is_live {
            self.start_match(payload, timestamp, update);
        }

        if phase == Some(MapPhase::GameOver) {
            self.finish_match(timestamp, update);
            return;
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
            map: payload.map_name().map(str::to_owned),
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
        let headshot_delta = current_headshots
            .zip(self.last_round_headshots)
            .map_or(0, |(current, previous)| current.saturating_sub(previous));
        let round_id = self.active_round.as_ref().map(|round| round.id);
        let weapon = payload.active_weapon().map(str::to_owned);
        for index in 0..kill_delta {
            let is_headshot = index >= kill_delta.saturating_sub(headshot_delta);
            let metadata =
                kill_metadata(previous_kills + index + 1, is_headshot, weapon.as_deref());
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
        if let Some(headshots) = current_headshots {
            self.last_round_headshots = Some(headshots);
        }
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

fn kill_metadata(total_kills: u16, headshot: bool, weapon: Option<&str>) -> Value {
    let mut metadata = Map::new();
    metadata.insert("total_kills".to_owned(), Value::from(total_kills));
    metadata.insert("headshot".to_owned(), Value::from(headshot));
    if let Some(weapon) = weapon {
        metadata.insert("weapon".to_owned(), Value::from(weapon));
    }
    Value::Object(metadata)
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
