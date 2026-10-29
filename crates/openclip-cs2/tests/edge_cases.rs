//! GSI edge cases seen on community, retake and FACEIT-style servers, and when CS2 goes away.

use chrono::{DateTime, Duration};
use openclip_core::{GameEventType, GameStateUpdate};
use openclip_cs2::Cs2Adapter;
use serde_json::{Value, json};

fn payload(timestamp: i64, map: &str, round: u16, kills: u16) -> Value {
    json!({
        "provider": { "appid": 730, "steamid": "76561198000000001", "timestamp": timestamp },
        "map": {
            "mode": "competitive",
            "name": map,
            "phase": "live",
            "round": round,
            "team_ct": { "score": 0 },
            "team_t": { "score": 0 }
        },
        "round": { "phase": "live" },
        "player": {
            "steamid": "76561198000000001",
            "activity": "playing",
            "state": { "round_kills": kills, "round_killhs": 0 },
            "match_stats": { "kills": kills, "deaths": 0 }
        }
    })
}

fn feed(adapter: &mut Cs2Adapter, value: &Value) -> GameStateUpdate {
    adapter
        .process(&serde_json::to_vec(value).expect("payload serializes"))
        .expect("payload parses")
}

fn count(update: &GameStateUpdate, kind: GameEventType) -> usize {
    update
        .events
        .iter()
        .filter(|event| event.event_type == kind)
        .count()
}

#[test]
fn a_server_renaming_itself_does_not_split_the_match() {
    let mut adapter = Cs2Adapter::default();
    let first = feed(
        &mut adapter,
        &payload(100, "DE_MIRAGE | RETAKE #61 | TR | XPLAY.GG", 3, 0),
    );
    assert_eq!(count(&first, GameEventType::MatchStart), 1);
    assert_eq!(first.matches[0].map.as_deref(), Some("de_mirage"));

    let renamed = feed(
        &mut adapter,
        &payload(110, "DE_MIRAGE | RETAKE #62 | TR | XPLAY.GG", 3, 1),
    );
    assert_eq!(count(&renamed, GameEventType::MatchEnd), 0);
    assert_eq!(count(&renamed, GameEventType::MatchStart), 0);
    assert_eq!(count(&renamed, GameEventType::Kill), 1);
}

#[test]
fn a_different_map_on_the_same_server_still_starts_a_new_match() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "DE_MIRAGE | RETAKE #61", 3, 0));
    let next = feed(&mut adapter, &payload(200, "DE_INFERNO | RETAKE #61", 0, 0));
    assert_eq!(count(&next, GameEventType::MatchEnd), 1);
    assert_eq!(count(&next, GameEventType::MatchStart), 1);
}

#[test]
fn an_impossible_kill_jump_is_a_resync_not_a_kill_streak() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "de_mirage", 3, 2));
    let jump = feed(&mut adapter, &payload(110, "de_mirage", 3, 14));
    assert_eq!(count(&jump, GameEventType::Kill), 0);

    let next = feed(&mut adapter, &payload(112, "de_mirage", 3, 15));
    assert_eq!(
        count(&next, GameEventType::Kill),
        1,
        "tracking resumes from the new baseline"
    );
}

#[test]
fn five_kills_in_one_update_are_still_real() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "de_mirage", 3, 0));
    let burst = feed(&mut adapter, &payload(101, "de_mirage", 3, 5));
    assert_eq!(count(&burst, GameEventType::Kill), 5);
}

#[test]
fn a_silent_stream_closes_the_match_once() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(1_000, "de_mirage", 3, 0));
    let at = |seconds: i64| DateTime::from_timestamp(1_000 + seconds, 0).unwrap();
    let idle = Duration::seconds(60);

    assert!(
        adapter.close_if_idle(at(30), idle).is_empty(),
        "heartbeats are still arriving"
    );

    let closed = adapter.close_if_idle(at(61), idle);
    assert_eq!(count(&closed, GameEventType::MatchEnd), 1);
    assert!(
        closed
            .matches
            .iter()
            .any(|game_match| game_match.ended_at.is_some())
    );
    assert!(adapter.active_match().is_none());

    assert!(
        adapter.close_if_idle(at(120), idle).is_empty(),
        "closing is not repeated"
    );
}

#[test]
fn a_new_match_starts_cleanly_after_the_game_was_closed() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(1_000, "de_mirage", 20, 9));
    let _ = adapter.close_if_idle(
        DateTime::from_timestamp(1_100, 0).unwrap(),
        Duration::seconds(60),
    );

    let restarted = feed(&mut adapter, &payload(2_000, "de_mirage", 0, 0));
    assert_eq!(count(&restarted, GameEventType::MatchStart), 1);
    let first_kill = feed(&mut adapter, &payload(2_010, "de_mirage", 0, 1));
    assert_eq!(count(&first_kill, GameEventType::Kill), 1);
}

fn rush(timestamp: i64, round: u16, round_phase: &str, kills: u16) -> Value {
    let mut value = payload(timestamp, "rush_001", round, kills);
    value["map"]["mode"] = json!("rush");
    value["round"]["phase"] = json!(round_phase);
    value
}

#[test]
fn rush_matches_are_recognized_with_a_plain_map_name() {
    let mut adapter = Cs2Adapter::default();
    let started = feed(&mut adapter, &rush(100, 0, "live", 0));
    assert_eq!(count(&started, GameEventType::MatchStart), 1);
    let game_match = &started.matches[0];
    assert_eq!(game_match.mode, openclip_core::MatchMode::Rush);
    assert_eq!(game_match.map.as_deref(), Some("rush_001"));
}

#[test]
fn rush_rounds_forty_seconds_apart_keep_their_kills_in_their_own_round() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &rush(100, 0, "live", 0));
    let first_kill = feed(&mut adapter, &rush(120, 0, "live", 1));
    let mut over = rush(140, 0, "over", 2);
    over["round"]["win_team"] = json!("CT");
    let closing = feed(&mut adapter, &over);
    feed(&mut adapter, &rush(151, 1, "freezetime", 2));
    let next_round = feed(&mut adapter, &rush(160, 1, "live", 3));

    let round_of = |update: &GameStateUpdate| {
        update
            .events
            .iter()
            .find(|event| event.event_type == GameEventType::Kill)
            .and_then(|event| event.round_id)
    };
    let first = round_of(&first_kill).expect("first kill has a round");
    assert_eq!(
        round_of(&closing),
        Some(first),
        "the round-ending kill stays in its round"
    );
    let second = round_of(&next_round).expect("kill in the next round has a round");
    assert_ne!(first, second);
}

#[test]
fn an_unlisted_mode_still_records_the_match() {
    let mut adapter = Cs2Adapter::default();
    let mut value = payload(100, "de_mirage", 0, 0);
    value["map"]["mode"] = json!("some_future_mode");
    let started = feed(&mut adapter, &value);
    assert_eq!(count(&started, GameEventType::MatchStart), 1);
    assert_eq!(started.matches[0].mode, openclip_core::MatchMode::Unknown);
}
