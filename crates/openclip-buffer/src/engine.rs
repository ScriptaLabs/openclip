//! Clock-injected moment engine.
//!
//! The engine turns generic game events into capture jobs on a single timeline: the recorder's
//! encoded-segment clock. Event offsets are re-stamped from the moment each update was observed,
//! instead of trusting the whole-second GSI provider timestamp. Grouping decisions and the
//! requested capture range therefore agree, and kills in one tick keep a stable relative order.
//! The caller supplies `now_ms`, so the whole pipeline is deterministic under test.

use std::ops::RangeInclusive;

use openclip_core::{
    GameEvent, GameEventType, MatchId, Moment, MomentDetector, MomentId, Settings,
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
    end_ms: u64,
}

#[derive(Debug)]
pub struct MomentEngine {
    detector: WindowedMomentDetector,
    pre_roll_ms: u64,
    anchor: Option<(MatchId, u64)>,
    active_range: Option<ActiveRange>,
}

impl MomentEngine {
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            detector: WindowedMomentDetector::from_settings(settings),
            pre_roll_ms: u64::from(settings.pre_roll_seconds) * 1_000,
            anchor: None,
            active_range: None,
        }
    }

    pub fn update_settings(&mut self, settings: &Settings) {
        self.detector.update_settings(settings);
        self.pre_roll_ms = u64::from(settings.pre_roll_seconds) * 1_000;
    }

    /// Timeline instant at which the open moment's post-roll expires.
    #[must_use]
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.active_range.map(|range| range.end_ms)
    }

    /// Feeds the events of one observed update. All events share `now_ms`.
    pub fn ingest(&mut self, events: Vec<GameEvent>, now_ms: u64) -> Vec<CaptureJob> {
        let mut jobs = Vec::new();
        for mut event in events {
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
            if Some(active.id) != before_id {
                self.active_range = Some(ActiveRange {
                    moment_id: active.id,
                    start_ms: now_ms.saturating_sub(self.pre_roll_ms),
                    end_ms: now_ms.saturating_add(post_roll),
                });
            } else if is_kill
                && active.events.len() > before_events
                && let Some(range) = self.active_range.as_mut()
            {
                range.end_ms = range.end_ms.max(now_ms.saturating_add(post_roll));
            }
        }
        jobs
    }

    /// Closes the open moment once its post-roll has elapsed on the timeline.
    pub fn expire(&mut self, now_ms: u64) -> Option<CaptureJob> {
        let range = self.active_range?;
        if now_ms < range.end_ms {
            return None;
        }
        let end_offset = self.detector.active()?.end_offset_ms;
        let moment = self.detector.finalize_expired(end_offset)?;
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
        assert!(engine.ingest(vec![kill], 63_400).is_empty());
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
    fn expire_before_the_deadline_is_a_no_op() {
        let match_id = MatchId::new();
        let mut engine = MomentEngine::new(&settings());
        engine.ingest(vec![event(match_id, None, GameEventType::Kill)], 20_000);
        assert!(engine.expire(24_999).is_none());
        assert!(engine.flush().is_some());
    }
}
