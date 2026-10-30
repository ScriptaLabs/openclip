//! Clock-injected moment engine.
//!
//! The engine turns generic game events into capture jobs on a single timeline: the recorder's
//! encoded-segment clock. Event offsets are re-stamped from the moment each update was observed,
//! instead of trusting the whole-second GSI provider timestamp. Grouping decisions and the
//! requested capture range therefore agree, and kills in one tick keep a stable relative order.
//! The caller supplies `now_ms`, so the whole pipeline is deterministic under test.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use openclip_core::{
    GameEvent, GameEventType, Match, MatchId, MatchOrigin, Moment, MomentDetector, MomentId,
    Settings,
};

use crate::WindowedMomentDetector;

/// A finalized logical moment plus the recorder-timeline range that must be saved for it.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureJob {
    pub moment: Moment,
    pub capture_range: RangeInclusive<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveRange {
    moment_id: MomentId,
    start_ms: u64,
    /// End of the clip to save: one post-roll after the last kill.
    end_ms: u64,
    /// When the moment stops waiting for another kill. Later than `end_ms` for a wide window.
    close_ms: u64,
}

#[derive(Debug)]
pub struct MomentEngine {
    detector: WindowedMomentDetector,
    pre_roll_ms: u64,
    anchor: Option<(MatchId, u64)>,
    active_range: Option<ActiveRange>,
    /// Where each observed match is played, to leave workshop and community matches out.
    origins: HashMap<MatchId, MatchOrigin>,
    skip_workshop_maps: bool,
    skip_community_servers: bool,
}

impl MomentEngine {
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            detector: WindowedMomentDetector::from_settings(settings),
            pre_roll_ms: u64::from(settings.pre_roll_seconds) * 1_000,
            anchor: None,
            active_range: None,
            origins: HashMap::new(),
            skip_workshop_maps: settings.skip_workshop_maps,
            skip_community_servers: settings.skip_community_servers,
        }
    }

    /// Learns where matches are played. Call it with each update's matches before `ingest`.
    pub fn observe_matches(&mut self, matches: &[Match]) {
        for game_match in matches {
            self.origins.insert(game_match.id, game_match.origin);
        }
    }

    fn skips(&self, match_id: MatchId) -> bool {
        match self.origins.get(&match_id) {
            Some(MatchOrigin::Workshop) => self.skip_workshop_maps,
            Some(MatchOrigin::Community) => self.skip_community_servers,
            Some(MatchOrigin::Official) | None => false,
        }
    }

    pub fn update_settings(&mut self, settings: &Settings) {
        self.skip_workshop_maps = settings.skip_workshop_maps;
        self.skip_community_servers = settings.skip_community_servers;
        self.detector.update_settings(settings);
        self.pre_roll_ms = u64::from(settings.pre_roll_seconds) * 1_000;
    }

    /// Timeline instant at which the open moment's post-roll expires.
    #[must_use]
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.active_range.map(|range| range.close_ms)
    }

    /// Feeds the events of one observed update. All events share `now_ms`.
    pub fn ingest(&mut self, events: Vec<GameEvent>, now_ms: u64) -> Vec<CaptureJob> {
        let mut jobs = Vec::new();
        for mut event in events {
            if self.skips(event.match_id) {
                continue;
            }
            self.retime(&mut event, now_ms);
            let before_id = self.detector.active().map(|moment| moment.id);
            let before_events = self
                .detector
                .active()
                .map_or(0, |moment| moment.events.len());
            let is_kill = matches!(
                event.event_type,
                GameEventType::Kill | GameEventType::Headshot
            );
            let event_offset = event.match_offset_ms;
            match self.detector.ingest(event) {
                Ok(Some(moment)) => jobs.extend(self.close(moment)),
                Ok(None) => {}
                // The detector only rejects structurally invalid events; one bad event must not
                // stop the engine from handling the rest of the update.
                Err(_) => continue,
            }
            let Some(active) = self.detector.active() else {
                continue;
            };
            let post_roll = active
                .end_offset_ms
                .saturating_sub(event_offset.unwrap_or(active.end_offset_ms));
            let hold = self.detector.merge_window_ms();
            if Some(active.id) != before_id {
                self.active_range = Some(ActiveRange {
                    moment_id: active.id,
                    start_ms: now_ms.saturating_sub(self.pre_roll_ms),
                    end_ms: now_ms.saturating_add(post_roll),
                    close_ms: now_ms.saturating_add(hold),
                });
            } else if is_kill
                && active.events.len() > before_events
                && let Some(range) = self.active_range.as_mut()
            {
                range.end_ms = range.end_ms.max(now_ms.saturating_add(post_roll));
                range.close_ms = range.close_ms.max(now_ms.saturating_add(hold));
            }
        }
        jobs
    }

    /// Closes the open moment once its post-roll has elapsed on the timeline.
    pub fn expire(&mut self, now_ms: u64) -> Option<CaptureJob> {
        let range = self.active_range?;
        if now_ms < range.close_ms {
            return None;
        }
        let closes_at = self.detector.closes_at_ms()?;
        let moment = self.detector.finalize_expired(closes_at)?;
        self.close(moment)
    }

    /// Closes whatever is open, e.g. when the game session ends.
    pub fn flush(&mut self) -> Option<CaptureJob> {
        let moment = self.detector.flush()?;
        self.close(moment)
    }

    fn close(&mut self, moment: Moment) -> Option<CaptureJob> {
        let range = self.active_range.take()?;
        debug_assert_eq!(moment.id, range.moment_id);
        Some(CaptureJob {
            moment,
            capture_range: range.start_ms..=range.end_ms,
        })
    }

    fn retime(&mut self, event: &mut GameEvent, now_ms: u64) {
        let anchor_ms = match self.anchor {
            Some((match_id, anchor_ms))
                if match_id == event.match_id && event.event_type != GameEventType::MatchStart =>
            {
                anchor_ms
            }
            _ => {
                let anchor_ms = if event.event_type == GameEventType::MatchStart {
                    now_ms
                } else {
                    now_ms.saturating_sub(event.match_offset_ms.unwrap_or(0))
                };
                self.anchor = Some((event.match_id, anchor_ms));
                anchor_ms
            }
        };
        event.match_offset_ms = Some(now_ms.saturating_sub(anchor_ms));
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openclip_core::{Game, GameEventId, MomentClassification, RoundId};
    use serde_json::json;

    use super::*;

    fn settings() -> Settings {
        Settings {
            pre_roll_seconds: 10,
            post_roll_seconds: 5,
            multi_kill_window_seconds: 5,
            ..Settings::default()
        }
    }

    fn wide_window() -> Settings {
        Settings {
            pre_roll_seconds: 10,
            post_roll_seconds: 3,
            multi_kill_window_seconds: 8,
            ..Settings::default()
        }
    }

    fn event(match_id: MatchId, round_id: Option<RoundId>, event_type: GameEventType) -> GameEvent {
        GameEvent {
            id: GameEventId::new(),
            match_id,
            round_id,
            source_game: Game::CounterStrike2,
            event_type,
            occurred_at: Utc::now(),
            match_offset_ms: Some(0),
            metadata: json!({}),
        }
    }

    fn match_of(id: MatchId, origin: MatchOrigin) -> Match {
        Match {
            id,
            session_id: openclip_core::GameSessionId::new(),
            map: None,
            mode: openclip_core::MatchMode::Competitive,
            origin,
            started_at: Utc::now(),
            ended_at: None,
            score_team_a: None,
            score_team_b: None,
            rounds: Vec::new(),
            moments: Vec::new(),
        }
    }

    #[test]
    fn workshop_and_community_matches_make_no_moments_while_filtered() {
        for origin in [MatchOrigin::Workshop, MatchOrigin::Community] {
            let match_id = MatchId::new();
            let mut engine = MomentEngine::new(&settings());
            engine.observe_matches(&[match_of(match_id, origin)]);
            engine.ingest(vec![event(match_id, None, GameEventType::Kill)], 30_000);
            assert_eq!(engine.next_deadline_ms(), None, "{origin:?} is skipped");
        }
    }

    #[test]
    fn official_matches_and_unfiltered_origins_still_record() {
        let official = MatchId::new();
        let mut engine = MomentEngine::new(&settings());
        engine.observe_matches(&[match_of(official, MatchOrigin::Official)]);
        engine.ingest(vec![event(official, None, GameEventType::Kill)], 30_000);
        assert!(engine.next_deadline_ms().is_some());

        let workshop = MatchId::new();
        let allowing = Settings {
            skip_workshop_maps: false,
            ..settings()
        };
        let mut engine = MomentEngine::new(&allowing);
        engine.observe_matches(&[match_of(workshop, MatchOrigin::Workshop)]);
        engine.ingest(vec![event(workshop, None, GameEventType::Kill)], 30_000);
        assert!(engine.next_deadline_ms().is_some());
    }

    #[test]
    fn turning_the_filter_on_applies_to_the_running_match() {
        let match_id = MatchId::new();
        let allowing = Settings {
            skip_community_servers: false,
            ..settings()
        };
        let mut engine = MomentEngine::new(&allowing);
        engine.observe_matches(&[match_of(match_id, MatchOrigin::Community)]);
        engine.update_settings(&settings());
        engine.ingest(vec![event(match_id, None, GameEventType::Kill)], 30_000);
        assert_eq!(engine.next_deadline_ms(), None);
    }

    #[test]
    fn range_is_anchored_to_the_observation_clock_not_gsi_offsets() {
        let match_id = MatchId::new();
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(
            vec![event(match_id, None, GameEventType::MatchStart)],
            40_000,
        );
        // The adapter claims 1s (whole-second GSI clock); the observation clock says 63.4s.
        let mut kill = event(match_id, None, GameEventType::Kill);
        kill.match_offset_ms = Some(1_000);
        assert_eq!(engine.ingest(vec![kill], 63_400), Vec::new());
        assert_eq!(engine.next_deadline_ms(), Some(68_400));

        let job = engine.expire(68_400).expect("expired moment");
        assert_eq!(job.capture_range, 53_400..=68_400);
        assert_eq!(job.moment.events[0].match_offset_ms, Some(23_400));
    }

    #[test]
    fn a_spray_across_ticks_becomes_one_extended_quad_kill() {
        let match_id = MatchId::new();
        let round = Some(RoundId::new());
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(vec![event(match_id, round, GameEventType::MatchStart)], 0);
        for now in [30_000, 32_500, 32_500, 36_000] {
            engine.ingest(vec![event(match_id, round, GameEventType::Kill)], now);
        }
        assert_eq!(engine.next_deadline_ms(), Some(41_000));
        let job = engine.expire(41_000).expect("moment");
        assert_eq!(job.moment.classification, MomentClassification::QuadKill);
        assert_eq!(job.capture_range, 20_000..=41_000);
        let offsets: Vec<_> = job
            .moment
            .events
            .iter()
            .map(|item| item.match_offset_ms.unwrap())
            .collect();
        assert_eq!(offsets, [30_000, 32_500, 32_500, 36_000]);
    }

    #[test]
    fn round_end_closes_but_keeps_the_post_roll_of_the_last_kill() {
        let match_id = MatchId::new();
        let round = Some(RoundId::new());
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(vec![event(match_id, None, GameEventType::MatchStart)], 0);
        let jobs = engine.ingest(
            vec![
                event(match_id, round, GameEventType::Kill),
                event(match_id, round, GameEventType::RoundEnd),
            ],
            50_000,
        );
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].capture_range, 40_000..=55_000);
        assert!(engine.next_deadline_ms().is_none());
    }

    #[test]
    fn kills_in_different_rounds_never_share_a_clip() {
        let match_id = MatchId::new();
        let first = Some(RoundId::new());
        let second = Some(RoundId::new());
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(vec![event(match_id, None, GameEventType::MatchStart)], 0);
        engine.ingest(vec![event(match_id, first, GameEventType::Kill)], 10_000);
        let jobs = engine.ingest(vec![event(match_id, second, GameEventType::Kill)], 12_000);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].moment.events.len(), 1);
        assert_eq!(engine.next_deadline_ms(), Some(17_000));
    }

    #[test]
    fn spaced_kills_stay_together_without_stretching_the_clip() {
        let match_id = MatchId::new();
        let round = Some(RoundId::new());
        let mut engine = MomentEngine::new(&wide_window());
        engine.ingest(vec![event(match_id, round, GameEventType::MatchStart)], 0);
        // Seven seconds apart: farther than the 3 s post-roll, inside the 8 s window.
        for now in [30_000, 37_000, 44_000] {
            assert_eq!(
                engine.ingest(vec![event(match_id, round, GameEventType::Kill)], now),
                Vec::new()
            );
        }
        assert_eq!(engine.next_deadline_ms(), Some(52_000));
        assert!(engine.expire(51_999).is_none());
        let job = engine.expire(52_000).expect("moment");
        assert_eq!(job.moment.classification, MomentClassification::TripleKill);
        // The clip still ends one post-roll after the last kill, not at the end of the window.
        assert_eq!(job.capture_range, 20_000..=47_000);
    }

    #[test]
    fn kills_farther_apart_than_the_window_are_separate_moments() {
        let match_id = MatchId::new();
        let round = Some(RoundId::new());
        let mut engine = MomentEngine::new(&wide_window());
        engine.ingest(vec![event(match_id, round, GameEventType::MatchStart)], 0);
        engine.ingest(vec![event(match_id, round, GameEventType::Kill)], 30_000);
        let jobs = engine.ingest(vec![event(match_id, round, GameEventType::Kill)], 38_001);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].moment.events.len(), 1);
        assert_eq!(jobs[0].capture_range, 20_000..=33_000);
    }

    #[test]
    fn expire_before_the_deadline_is_a_no_op() {
        let match_id = MatchId::new();
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(vec![event(match_id, None, GameEventType::Kill)], 20_000);
        assert!(engine.expire(24_999).is_none());
        assert!(engine.flush().is_some());
    }
}
