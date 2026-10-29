//! Regression tests for kills that arrive in the same GSI payload as a round or match boundary.

use openclip_core::{GameEventType, GameStateUpdate};
use openclip_cs2::Cs2Adapter;
use serde_json::{Value, json};

fn payload(timestamp: i64, map_phase: &str, round_phase: &str, round: u16, kills: u16) -> Value {
    json!({
        "provider": {
            "name": "Counter-Strike 2",
            "appid": 730,
            "steamid": "76561198000000001",
            "timestamp": timestamp
        },
        "map": {
            "mode": "competitive",
            "name": "de_inferno",
            "phase": map_phase,
            "round": round,
            "team_ct": { "score": 0 },
            "team_t": { "score": 0 }
        },
        "round": { "phase": round_phase },
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

fn types(update: &GameStateUpdate) -> Vec<GameEventType> {
    update.events.iter().map(|event| event.event_type).collect()
}

#[test]
fn round_winning_kill_in_the_round_over_payload_keeps_its_round() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "live", "live", 0, 0));
    feed(&mut adapter, &payload(110, "live", "live", 0, 1));

    let mut ending = payload(120, "live", "over", 0, 2);
    ending["round"]["win_team"] = json!("CT");
    let update = feed(&mut adapter, &ending);

    let kinds = types(&update);
    let kill = kinds
        .iter()
        .position(|kind| *kind == GameEventType::Kill)
        .expect("the final kill is emitted");
    let round_end = kinds
        .iter()
        .position(|kind| *kind == GameEventType::RoundEnd)
        .expect("the round ends");
    assert!(kill < round_end, "kill must precede the round boundary");
    let round_id = update.events[round_end].round_id;
    assert!(round_id.is_some());
    assert_eq!(update.events[kill].round_id, round_id);
}

#[test]
fn match_winning_kill_in_the_game_over_payload_is_not_lost() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "live", "live", 23, 10));

    let mut final_payload = payload(120, "gameover", "over", 23, 11);
    final_payload["map"]["team_ct"]["score"] = json!(13);
    final_payload["map"]["team_t"]["score"] = json!(10);
    let update = feed(&mut adapter, &final_payload);

    let kinds = types(&update);
    let kill = kinds
        .iter()
        .position(|kind| *kind == GameEventType::Kill)
        .expect("the final kill is emitted");
    let match_end = kinds
        .iter()
        .position(|kind| *kind == GameEventType::MatchEnd)
        .expect("the match ends");
    assert!(kill < match_end);
    let finished = update.matches.last().expect("finished match is reported");
    assert!(finished.ended_at.is_some());
    assert_eq!(finished.score_team_a, Some(13));
    assert_eq!(finished.score_team_b, Some(10));
}

#[test]
fn kill_after_a_counter_dip_is_counted_once() {
    let mut adapter = Cs2Adapter::default();
    feed(&mut adapter, &payload(100, "live", "live", 3, 5));
    let dip = feed(&mut adapter, &payload(105, "live", "live", 3, 4));
    assert!(!types(&dip).contains(&GameEventType::Kill));
    let recovered = feed(&mut adapter, &payload(110, "live", "live", 3, 5));
    assert_eq!(
        types(&recovered)
            .iter()
            .filter(|kind| **kind == GameEventType::Kill)
            .count(),
        1
    );
}
