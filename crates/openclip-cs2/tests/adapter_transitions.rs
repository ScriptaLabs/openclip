use openclip_core::GameEventType;
use openclip_cs2::Cs2Adapter;
use serde_json::Value;

const INACTIVE: &[u8] = include_bytes!("fixtures/inactive.json");
const WARMUP: &[u8] = include_bytes!("fixtures/warmup.json");
const ROUND_LIVE: &[u8] = include_bytes!("fixtures/round_live.json");
const KILL_1: &[u8] = include_bytes!("fixtures/kill_1.json");
const KILL_2: &[u8] = include_bytes!("fixtures/kill_2.json");
const ROUND_OVER: &[u8] = include_bytes!("fixtures/round_over.json");
const ROUND_2_LIVE: &[u8] = include_bytes!("fixtures/round_2_live.json");
const COUNTER_RESET: &[u8] = include_bytes!("fixtures/counter_reset.json");
const MAP_CHANGE: &[u8] = include_bytes!("fixtures/map_change.json");
const PARTIAL: &[u8] = include_bytes!("fixtures/partial.json");
const MALFORMED: &[u8] = include_bytes!("fixtures/malformed.json");

fn count(update: &openclip_core::GameStateUpdate, event_type: GameEventType) -> usize {
    update
        .events
        .iter()
        .filter(|event| event.event_type == event_type)
        .count()
}

#[test]
fn representative_sequence_emits_lifecycle_and_non_duplicate_kills() {
    let mut adapter = Cs2Adapter::default();
    let inactive = adapter.process(INACTIVE).expect("inactive payload parses");
    assert_eq!(inactive.sessions.len(), 1);
    assert!(inactive.matches.is_empty());

    assert!(
        adapter
            .process(WARMUP)
            .expect("warmup parses")
            .matches
            .is_empty()
    );
    let started = adapter.process(ROUND_LIVE).expect("live round parses");
    assert_eq!(count(&started, GameEventType::MatchStart), 1);
    assert_eq!(count(&started, GameEventType::RoundStart), 1);

    let first_kill = adapter.process(KILL_1).expect("kill parses");
    assert_eq!(count(&first_kill, GameEventType::Kill), 1);
    assert_eq!(count(&first_kill, GameEventType::Headshot), 1);
    assert_eq!(first_kill.events[0].metadata["weapon"], "weapon_ak47");

    let repeated = adapter.process(KILL_1).expect("repeat parses");
    assert_eq!(count(&repeated, GameEventType::Kill), 0);

    let second_kill = adapter.process(KILL_2).expect("second kill parses");
    assert_eq!(count(&second_kill, GameEventType::Kill), 1);
    assert_eq!(count(&second_kill, GameEventType::Headshot), 0);

    let ended = adapter.process(ROUND_OVER).expect("round end parses");
    assert_eq!(count(&ended, GameEventType::RoundEnd), 1);
    assert_eq!(count(&ended, GameEventType::RoundWin), 1);

    let next = adapter.process(ROUND_2_LIVE).expect("next round parses");
    assert_eq!(count(&next, GameEventType::RoundStart), 1);
    assert_eq!(next.rounds[0].number, 2);
}

#[test]
fn initial_nonzero_count_is_a_baseline_not_backfilled_kills() {
    let mut value: Value = serde_json::from_slice(ROUND_LIVE).expect("fixture JSON");
    value["player"]["match_stats"]["kills"] = Value::from(7);
    let initial = serde_json::to_vec(&value).expect("fixture serializes");
    let mut adapter = Cs2Adapter::default();
    let update = adapter.process(&initial).expect("initial live payload");
    assert_eq!(count(&update, GameEventType::Kill), 0);

    value["provider"]["timestamp"] = Value::from(1_700_000_021_i64);
    value["player"]["match_stats"]["kills"] = Value::from(8);
    let next = serde_json::to_vec(&value).expect("fixture serializes");
    assert_eq!(
        count(
            &adapter.process(&next).expect("next payload"),
            GameEventType::Kill
        ),
        1
    );
}

#[test]
fn missing_stats_do_not_erase_the_kill_baseline() {
    let mut adapter = Cs2Adapter::default();
    adapter.process(ROUND_LIVE).expect("live");
    adapter.process(KILL_1).expect("first kill");
    assert!(adapter.process(PARTIAL).expect("partial").events.is_empty());
    assert_eq!(
        count(
            &adapter.process(KILL_2).expect("second kill"),
            GameEventType::Kill
        ),
        1
    );
}

#[test]
fn counter_reset_does_not_emit_a_strange_event() {
    let mut adapter = Cs2Adapter::default();
    for payload in [ROUND_LIVE, KILL_1, KILL_2, ROUND_OVER, ROUND_2_LIVE] {
        adapter.process(payload).expect("sequence payload");
    }
    let reset = adapter.process(COUNTER_RESET).expect("reset parses");
    assert_eq!(count(&reset, GameEventType::Kill), 0);
}

#[test]
fn map_change_finishes_one_match_and_starts_another() {
    let mut adapter = Cs2Adapter::default();
    adapter.process(ROUND_LIVE).expect("first match");
    let previous_id = adapter.active_match().expect("active match").id;
    let changed = adapter.process(MAP_CHANGE).expect("map change");
    let next_id = adapter.active_match().expect("new active match").id;
    assert_ne!(previous_id, next_id);
    assert_eq!(count(&changed, GameEventType::MatchEnd), 1);
    assert_eq!(count(&changed, GameEventType::MatchStart), 1);
    assert_eq!(count(&changed, GameEventType::RoundStart), 1);
}

#[test]
fn malformed_payload_returns_an_error_and_partial_payload_is_safe() {
    let mut adapter = Cs2Adapter::default();
    assert!(adapter.process(MALFORMED).is_err());
    assert!(
        adapter
            .process(PARTIAL)
            .expect("partial parses")
            .matches
            .is_empty()
    );
}
