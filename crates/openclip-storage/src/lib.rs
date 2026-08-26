//! `SQLite` persistence for `OpenClip` metadata.

use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use openclip_core::{
    GameEvent, GameEventId, GameSession, GameSessionId, GameStateUpdate, Match, MatchId,
    MatchRepository, Moment, MomentId, OpenClipError, Round, RoundId, Settings,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;
use uuid::Uuid;

const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");

pub struct SqliteRepository {
    connection: Connection,
}

impl SqliteRepository {
    /// Opens a file-backed repository and applies all migrations.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the database cannot be opened or migrated.
    pub fn open(path: impl AsRef<Path>) -> openclip_core::Result<Self> {
        let connection = Connection::open(path).map_err(storage_error)?;
        let repository = Self { connection };
        repository.migrate()?;
        Ok(repository)
    }

    /// Opens a migrated, in-memory repository for tests and transient tooling.
    ///
    /// # Errors
    ///
    /// Returns a storage error when `SQLite` cannot allocate or migrate the database.
    pub fn in_memory() -> openclip_core::Result<Self> {
        let connection = Connection::open_in_memory().map_err(storage_error)?;
        let repository = Self { connection };
        repository.migrate()?;
        Ok(repository)
    }

    fn migrate(&self) -> openclip_core::Result<()> {
        self.connection
            .execute_batch(INITIAL_MIGRATION)
            .map_err(storage_error)
    }

    fn load_rounds(&self, match_id: MatchId) -> openclip_core::Result<Vec<Round>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, number, started_at, ended_at FROM rounds
                 WHERE match_id = ?1 ORDER BY number ASC",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([match_id.to_string()], |row| {
                Ok(Round {
                    id: RoundId(parse_uuid(&row.get::<_, String>(0)?)?),
                    match_id,
                    number: row.get(1)?,
                    started_at: parse_datetime(&row.get::<_, String>(2)?)?,
                    ended_at: parse_optional_datetime(row.get::<_, Option<String>>(3)?.as_deref())?,
                    events: Vec::new(),
                })
            })
            .map_err(storage_error)?;

        let mut rounds = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage_error)?;
        for round in &mut rounds {
            round.events = self.load_events(match_id, Some(round.id))?;
        }
        Ok(rounds)
    }

    fn load_events(
        &self,
        match_id: MatchId,
        round_id: Option<RoundId>,
    ) -> openclip_core::Result<Vec<GameEvent>> {
        let (sql, round_parameter) = if let Some(round_id) = round_id {
            (
                "SELECT id, round_id, source_game, event_type, occurred_at, match_offset_ms,
                        metadata_json FROM events WHERE match_id = ?1 AND round_id = ?2
                 ORDER BY occurred_at ASC",
                Some(round_id.to_string()),
            )
        } else {
            (
                "SELECT id, round_id, source_game, event_type, occurred_at, match_offset_ms,
                        metadata_json FROM events WHERE match_id = ?1 AND round_id IS NULL
                 ORDER BY occurred_at ASC",
                None,
            )
        };
        let mut statement = self.connection.prepare(sql).map_err(storage_error)?;
        let mapper = |row: &Row<'_>| map_event_row(row, match_id);
        let events = match round_parameter {
            Some(round_id) => statement
                .query_map(params![match_id.to_string(), round_id], mapper)
                .map_err(storage_error)?
                .collect::<rusqlite::Result<Vec<_>>>(),
            None => statement
                .query_map([match_id.to_string()], mapper)
                .map_err(storage_error)?
                .collect::<rusqlite::Result<Vec<_>>>(),
        }
        .map_err(storage_error)?;
        Ok(events)
    }

    fn load_moments(&self, match_id: MatchId) -> openclip_core::Result<Vec<Moment>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT m.id, m.round_id, m.start_offset_ms, m.end_offset_ms,
                        m.classification, m.importance, m.is_favorite, c.path
                 FROM moments m LEFT JOIN clips c ON c.moment_id = m.id
                 WHERE m.match_id = ?1 ORDER BY m.start_offset_ms ASC",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([match_id.to_string()], |row| {
                Ok(Moment {
                    id: MomentId(parse_uuid(&row.get::<_, String>(0)?)?),
                    match_id,
                    round_id: parse_optional_uuid(row.get::<_, Option<String>>(1)?.as_deref())?
                        .map(RoundId),
                    start_offset_ms: parse_nonnegative(row.get(2)?)?,
                    end_offset_ms: parse_nonnegative(row.get(3)?)?,
                    events: Vec::new(),
                    classification: parse_json_enum(row.get::<_, String>(4)?)?,
                    importance: row.get(5)?,
                    is_favorite: row.get(6)?,
                    clip_path: row.get::<_, Option<String>>(7)?.map(Into::into),
                })
            })
            .map_err(storage_error)?;
        let mut moments = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage_error)?;

        for moment in &mut moments {
            let mut events_statement = self
                .connection
                .prepare(
                    "SELECT e.id, e.round_id, e.source_game, e.event_type, e.occurred_at,
                            e.match_offset_ms, e.metadata_json
                     FROM events e JOIN moment_events me ON me.event_id = e.id
                     WHERE me.moment_id = ?1 ORDER BY e.occurred_at ASC",
                )
                .map_err(storage_error)?;
            moment.events = events_statement
                .query_map([moment.id.to_string()], |row| map_event_row(row, match_id))
                .map_err(storage_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage_error)?;
        }
        Ok(moments)
    }
}

impl MatchRepository for SqliteRepository {
    fn list_matches(&self) -> openclip_core::Result<Vec<Match>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, session_id, map, mode, started_at, ended_at, score_team_a,
                        score_team_b FROM matches ORDER BY started_at DESC",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], map_match_row)
            .map_err(storage_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage_error)?;
        rows.into_iter()
            .map(|mut game_match| {
                game_match.rounds = self.load_rounds(game_match.id)?;
                game_match.moments = self.load_moments(game_match.id)?;
                Ok(game_match)
            })
            .collect()
    }

    fn get_match(&self, id: MatchId) -> openclip_core::Result<Option<Match>> {
        let mut game_match = self
            .connection
            .query_row(
                "SELECT id, session_id, map, mode, started_at, ended_at, score_team_a,
                        score_team_b FROM matches WHERE id = ?1",
                [id.to_string()],
                map_match_row,
            )
            .optional()
            .map_err(storage_error)?;
        if let Some(game_match) = &mut game_match {
            game_match.rounds = self.load_rounds(id)?;
            game_match.moments = self.load_moments(id)?;
        }
        Ok(game_match)
    }

    fn save_session(&mut self, session: &GameSession) -> openclip_core::Result<()> {
        write_session(&self.connection, session)
    }

    fn save_match(&mut self, game_match: &Match) -> openclip_core::Result<()> {
        write_match(&self.connection, game_match)
    }

    fn save_round(&mut self, round: &Round) -> openclip_core::Result<()> {
        write_round(&self.connection, round)
    }

    fn save_event(&mut self, event: &GameEvent) -> openclip_core::Result<()> {
        write_event(&self.connection, event)
    }

    fn save_moment(&mut self, moment: &Moment) -> openclip_core::Result<()> {
        let start_offset_ms = to_sql_integer(moment.start_offset_ms)?;
        let end_offset_ms = to_sql_integer(moment.end_offset_ms)?;
        let transaction = self.connection.transaction().map_err(storage_error)?;
        transaction
            .execute(
                "INSERT INTO moments
                 (id, match_id, round_id, start_offset_ms, end_offset_ms, classification,
                  importance, is_favorite) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET end_offset_ms = excluded.end_offset_ms,
                   classification = excluded.classification, importance = excluded.importance,
                   is_favorite = excluded.is_favorite",
                params![
                    moment.id.to_string(),
                    moment.match_id.to_string(),
                    moment.round_id.map(|id| id.to_string()),
                    start_offset_ms,
                    end_offset_ms,
                    json_enum(moment.classification)?,
                    moment.importance,
                    moment.is_favorite,
                ],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                "DELETE FROM moment_events WHERE moment_id = ?1",
                [moment.id.to_string()],
            )
            .map_err(storage_error)?;
        for event in &moment.events {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO moment_events (moment_id, event_id) VALUES (?1, ?2)",
                    params![moment.id.to_string(), event.id.to_string()],
                )
                .map_err(storage_error)?;
        }
        if let Some(path) = &moment.clip_path {
            transaction
                .execute(
                    "INSERT INTO clips (moment_id, path) VALUES (?1, ?2)
                     ON CONFLICT(moment_id) DO UPDATE SET path = excluded.path",
                    params![moment.id.to_string(), path.to_string_lossy()],
                )
                .map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)
    }

    fn load_settings(&self) -> openclip_core::Result<Settings> {
        let json = self
            .connection
            .query_row("SELECT value_json FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        json.map_or_else(
            || Ok(Settings::default()),
            |json: String| serde_json::from_str(&json).map_err(storage_error),
        )
    }

    fn save_settings(&mut self, settings: &Settings) -> openclip_core::Result<()> {
        let json = serde_json::to_string(settings).map_err(storage_error)?;
        self.connection
            .execute(
                "INSERT INTO settings (id, value_json) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET value_json = excluded.value_json",
                [json],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn apply_game_update(&mut self, update: &GameStateUpdate) -> openclip_core::Result<()> {
        if update.is_empty() {
            return Ok(());
        }
        let transaction = self.connection.transaction().map_err(storage_error)?;
        for session in &update.sessions {
            write_session(&transaction, session)?;
        }
        for game_match in &update.matches {
            write_match(&transaction, game_match)?;
        }
        for round in &update.rounds {
            write_round(&transaction, round)?;
        }
        for event in &update.events {
            write_event(&transaction, event)?;
        }
        transaction.commit().map_err(storage_error)
    }
}

fn write_session(connection: &Connection, session: &GameSession) -> openclip_core::Result<()> {
    connection
        .execute(
            "INSERT INTO sessions (id, game, started_at, ended_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET ended_at = excluded.ended_at",
            params![
                session.id.to_string(),
                json_enum(session.game)?,
                session.started_at.to_rfc3339(),
                session.ended_at.map(|value| value.to_rfc3339()),
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn write_match(connection: &Connection, game_match: &Match) -> openclip_core::Result<()> {
    connection
        .execute(
            "INSERT INTO matches
             (id, session_id, map, mode, started_at, ended_at, score_team_a, score_team_b)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET map = excluded.map, mode = excluded.mode,
               ended_at = excluded.ended_at, score_team_a = excluded.score_team_a,
               score_team_b = excluded.score_team_b",
            params![
                game_match.id.to_string(),
                game_match.session_id.to_string(),
                game_match.map,
                json_enum(game_match.mode)?,
                game_match.started_at.to_rfc3339(),
                game_match.ended_at.map(|value| value.to_rfc3339()),
                game_match.score_team_a,
                game_match.score_team_b,
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn write_round(connection: &Connection, round: &Round) -> openclip_core::Result<()> {
    connection
        .execute(
            "INSERT INTO rounds (id, match_id, number, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET ended_at = excluded.ended_at",
            params![
                round.id.to_string(),
                round.match_id.to_string(),
                round.number,
                round.started_at.to_rfc3339(),
                round.ended_at.map(|value| value.to_rfc3339()),
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn write_event(connection: &Connection, event: &GameEvent) -> openclip_core::Result<()> {
    let match_offset_ms = event.match_offset_ms.map(to_sql_integer).transpose()?;
    connection
        .execute(
            "INSERT INTO events
             (id, match_id, round_id, source_game, event_type, occurred_at,
              match_offset_ms, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET metadata_json = excluded.metadata_json",
            params![
                event.id.to_string(),
                event.match_id.to_string(),
                event.round_id.map(|id| id.to_string()),
                json_enum(event.source_game)?,
                json_enum(event.event_type)?,
                event.occurred_at.to_rfc3339(),
                match_offset_ms,
                event.metadata.to_string(),
            ],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn map_match_row(row: &Row<'_>) -> rusqlite::Result<Match> {
    Ok(Match {
        id: MatchId(parse_uuid(&row.get::<_, String>(0)?)?),
        session_id: GameSessionId(parse_uuid(&row.get::<_, String>(1)?)?),
        map: row.get(2)?,
        mode: parse_json_enum(row.get::<_, String>(3)?)?,
        started_at: parse_datetime(&row.get::<_, String>(4)?)?,
        ended_at: parse_optional_datetime(row.get::<_, Option<String>>(5)?.as_deref())?,
        score_team_a: row.get(6)?,
        score_team_b: row.get(7)?,
        rounds: Vec::new(),
        moments: Vec::new(),
    })
}

fn map_event_row(row: &Row<'_>, match_id: MatchId) -> rusqlite::Result<GameEvent> {
    Ok(GameEvent {
        id: GameEventId(parse_uuid(&row.get::<_, String>(0)?)?),
        match_id,
        round_id: parse_optional_uuid(row.get::<_, Option<String>>(1)?.as_deref())?.map(RoundId),
        source_game: parse_json_enum(row.get::<_, String>(2)?)?,
        event_type: parse_json_enum(row.get::<_, String>(3)?)?,
        occurred_at: parse_datetime(&row.get::<_, String>(4)?)?,
        match_offset_ms: row
            .get::<_, Option<i64>>(5)?
            .map(parse_nonnegative)
            .transpose()?,
        metadata: serde_json::from_str(&row.get::<_, String>(6)?).map_err(sql_conversion)?,
    })
}

fn json_enum<T: serde::Serialize>(value: T) -> openclip_core::Result<String> {
    serde_json::to_value(value)
        .map_err(storage_error)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| OpenClipError::Storage("enum did not serialize as a string".to_owned()))
}

fn parse_json_enum<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_value(Value::String(value)).map_err(sql_conversion)
}

fn parse_uuid(value: &str) -> rusqlite::Result<Uuid> {
    Uuid::from_str(value).map_err(sql_conversion)
}

fn parse_optional_uuid(value: Option<&str>) -> rusqlite::Result<Option<Uuid>> {
    value.map(parse_uuid).transpose()
}

fn parse_datetime(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(sql_conversion)
}

fn parse_optional_datetime(value: Option<&str>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    value.map(parse_datetime).transpose()
}

fn parse_nonnegative(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(sql_conversion)
}

fn to_sql_integer(value: u64) -> openclip_core::Result<i64> {
    i64::try_from(value)
        .map_err(|_| OpenClipError::Storage("millisecond value exceeds SQLite INTEGER".to_owned()))
}

fn sql_conversion(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn storage_error(error: impl std::fmt::Display) -> OpenClipError {
    OpenClipError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use openclip_core::{
        Game, GameEventType, MatchMode, MatchRepository, MomentClassification, RecordingQuality,
    };
    use serde_json::json;

    use super::*;

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid test time")
    }

    #[test]
    fn round_trips_a_match_graph_and_settings() {
        let mut repository = SqliteRepository::in_memory().expect("database opens");
        let session = GameSession {
            id: GameSessionId::new(),
            game: Game::CounterStrike2,
            started_at: timestamp(1_000),
            ended_at: Some(timestamp(2_000)),
        };
        let match_id = MatchId::new();
        let round_id = RoundId::new();
        let event = GameEvent {
            id: GameEventId::new(),
            match_id,
            round_id: Some(round_id),
            source_game: Game::CounterStrike2,
            event_type: GameEventType::Kill,
            occurred_at: timestamp(1_100),
            match_offset_ms: Some(100_000),
            metadata: json!({"weapon": "ak47"}),
        };
        let game_round = Round {
            id: round_id,
            match_id,
            number: 1,
            started_at: timestamp(1_050),
            ended_at: Some(timestamp(1_120)),
            events: vec![event.clone()],
        };
        let moment = Moment {
            id: MomentId::new(),
            match_id,
            round_id: Some(round_id),
            start_offset_ms: 90_000,
            end_offset_ms: 105_000,
            events: vec![event.clone()],
            classification: MomentClassification::Kill,
            importance: 10.0,
            clip_path: Some("clips/mirage-kill.mkv".into()),
            is_favorite: true,
        };
        let game_match = Match {
            id: match_id,
            session_id: session.id,
            map: Some("de_mirage".to_owned()),
            mode: MatchMode::Competitive,
            started_at: timestamp(1_000),
            ended_at: Some(timestamp(2_000)),
            score_team_a: Some(13),
            score_team_b: Some(8),
            rounds: vec![game_round.clone()],
            moments: vec![moment.clone()],
        };

        repository.save_session(&session).expect("session saves");
        repository.save_match(&game_match).expect("match saves");
        repository.save_round(&game_round).expect("round saves");
        repository.save_event(&event).expect("event saves");
        repository.save_moment(&moment).expect("moment saves");

        let loaded = repository
            .get_match(match_id)
            .expect("query works")
            .unwrap();
        assert_eq!(loaded.map.as_deref(), Some("de_mirage"));
        assert_eq!(loaded.rounds[0].events[0].metadata["weapon"], "ak47");
        assert_eq!(loaded.moments[0].events.len(), 1);
        assert_eq!(loaded.moments[0].clip_path, moment.clip_path);
        assert!(loaded.moments[0].is_favorite);

        let settings = Settings {
            quality: RecordingQuality::Ultra,
            ..Settings::default()
        };
        repository.save_settings(&settings).expect("settings save");
        assert_eq!(repository.load_settings().unwrap(), settings);
    }

    #[test]
    fn defaults_settings_when_no_row_exists() {
        let repository = SqliteRepository::in_memory().expect("database opens");
        assert_eq!(repository.load_settings().unwrap(), Settings::default());
    }
}
