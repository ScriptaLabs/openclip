//! Replays synthetic GSI streams through the real adapter and moment engine on a fake clock.
//!
//! This is the harness for clipping edge cases: every scenario is a list of timed payloads, so a
//! FACEIT or reconnect bug can be pinned down without launching the game.

use openclip_buffer::{CaptureJob, MomentEngine};
use openclip_core::{MomentClassification, Settings};
use openclip_cs2::Cs2Adapter;
use serde_json::{Value, json};

struct Replay {
    adapter: Cs2Adapter,
    engine: MomentEngine,
    jobs: Vec<CaptureJob>,
    gsi_clock: i64,
}

impl Replay {
    fn new() -> Self {
        Self::with(&Settings {
            pre_roll_seconds: 10,
            post_roll_seconds: 5,
            ..Settings::default()
        })
    }

    fn with(settings: &Settings) -> Self {
        Self {
            adapter: Cs2Adapter::default(),
            engine: MomentEngine::new(settings),
            jobs: Vec::new(),
            gsi_clock: 1_700_000_000,
        }
    }

    /// Delivers one payload at `at_ms` on the capture timeline, expiring windows first.
    fn tick(&mut self, at_ms: u64, payload: &Value) {
        self.jobs.extend(self.engine.expire(at_ms));
        self.gsi_clock += 1;
        let mut payload = payload.clone();
        payload["provider"]["timestamp"] = json!(self.gsi_clock);
        let update = self
            .adapter
            .process(&serde_json::to_vec(&payload).expect("payload serializes"))
            .expect("payload parses");
        self.jobs.extend(self.engine.ingest(update.events, at_ms));
    }

    fn finish(&mut self, at_ms: u64) -> &[CaptureJob] {
        self.jobs.extend(self.engine.expire(at_ms));
        &self.jobs
    }
}

fn state(map_phase: &str, round_phase: &str, round: u16, kills: u16) -> Value {
    json!({
        "provider": { "appid": 730, "steamid": "76561198000000001" },
        "map": {
            "mode": "competitive",
            "name": "de_mirage",
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

#[test]
fn ace_that_ends_the_round_is_one_five_kill_clip() {
    let mut replay = Replay::new();
    replay.tick(1_000, &state("live", "live", 4, 0));
    for (at_ms, kills) in [(20_000, 1), (22_000, 2), (24_500, 3), (27_000, 4)] {
        replay.tick(at_ms, &state("live", "live", 4, kills));
    }
    // The fifth kill and the end of the round land in the same GSI payload.
    let mut closing = state("live", "over", 4, 5);
    closing["round"]["win_team"] = json!("CT");
    replay.tick(29_000, &closing);
    replay.tick(31_000, &state("live", "over", 4, 5));

    let jobs = replay.finish(40_000);
    assert_eq!(jobs.len(), 1, "one ace, one clip: {jobs:#?}");
    assert_eq!(
        jobs[0].moment.classification,
        MomentClassification::FiveKill
    );
    assert_eq!(jobs[0].capture_range, 10_000..=34_000);
}

#[test]
fn kills_in_consecutive_rounds_make_separate_clips() {
    let mut replay = Replay::new();
    replay.tick(1_000, &state("live", "live", 1, 0));
    replay.tick(10_000, &state("live", "live", 1, 1));
    let mut over = state("live", "over", 1, 1);
    over["round"]["win_team"] = json!("T");
    replay.tick(12_000, &over);
    replay.tick(25_000, &state("live", "freezetime", 2, 1));
    replay.tick(30_000, &state("live", "live", 2, 1));
    replay.tick(32_000, &state("live", "live", 2, 2));

    let jobs = replay.finish(60_000);
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| {
        job.moment.classification == MomentClassification::Kill && job.moment.events.len() == 1
    }));
    assert_eq!(jobs[0].capture_range, 0..=15_000);
    assert_eq!(jobs[1].capture_range, 22_000..=37_000);
}

#[test]
fn leaving_the_server_mid_window_still_closes_the_clip() {
    let mut replay = Replay::new();
    replay.tick(1_000, &state("live", "live", 7, 3));
    replay.tick(20_000, &state("live", "live", 7, 4));
    // The player drops to the menu (no map in the payload) before the post-roll ends.
    replay.tick(22_000, &json!({ "provider": { "appid": 730 } }));

    // The moment waits out the 8 s multi-kill window after the kill (20 s), then closes.
    assert_eq!(replay.finish(27_999), &[]);
    let jobs = replay.finish(28_000);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].moment.events.len(), 1);
    // The clip itself still ends one post-roll after the kill.
    assert_eq!(jobs[0].capture_range, 10_000..=25_000);
}

#[test]
fn slow_kills_inside_the_multi_kill_window_become_one_clip() {
    // A short clip tail must not make grouping stricter: kills 7 s apart are one triple kill.
    let mut replay = Replay::with(&Settings {
        pre_roll_seconds: 10,
        post_roll_seconds: 3,
        multi_kill_window_seconds: 8,
        ..Settings::default()
    });
    replay.tick(1_000, &state("live", "live", 6, 0));
    replay.tick(20_000, &state("live", "live", 6, 1));
    replay.tick(27_000, &state("live", "live", 6, 2));
    replay.tick(34_000, &state("live", "live", 6, 3));

    let jobs = replay.finish(60_000);
    assert_eq!(jobs.len(), 1, "one triple kill, one clip: {jobs:#?}");
    assert_eq!(
        jobs[0].moment.classification,
        MomentClassification::TripleKill
    );
    assert_eq!(jobs[0].capture_range, 10_000..=37_000);
}

#[test]
fn kills_farther_apart_than_the_window_stay_separate_clips() {
    let mut replay = Replay::with(&Settings {
        pre_roll_seconds: 10,
        post_roll_seconds: 3,
        multi_kill_window_seconds: 8,
        ..Settings::default()
    });
    replay.tick(1_000, &state("live", "live", 6, 0));
    replay.tick(20_000, &state("live", "live", 6, 1));
    replay.tick(29_500, &state("live", "live", 6, 2));

    let jobs = replay.finish(60_000);
    assert_eq!(jobs.len(), 2);
    assert!(
        jobs.iter()
            .all(|job| job.moment.classification == MomentClassification::Kill)
    );
}
