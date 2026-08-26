//! Replay-window and moment grouping logic.
//!
//! Encoded-segment storage and extraction arrive with the recording milestone. The moment
//! detector lives here now because its timing contract determines what that buffer must retain.

use openclip_core::{
    GameEvent, GameEventType, Moment, MomentClassification, MomentDetector, MomentId,
};

#[derive(Debug)]
pub struct WindowedMomentDetector {
    pre_roll_ms: u64,
    post_roll_ms: u64,
    active: Option<Moment>,
}

impl WindowedMomentDetector {
    #[must_use]
    pub const fn new(pre_roll_ms: u64, post_roll_ms: u64) -> Self {
        Self {
            pre_roll_ms,
            post_roll_ms,
            active: None,
        }
    }

    fn is_qualifying(event: &GameEvent) -> bool {
        matches!(
            event.event_type,
            GameEventType::Kill | GameEventType::Headshot
        )
    }

    fn create_moment(&self, event: GameEvent, offset: u64) -> Moment {
        let match_id = event.match_id;
        let round_id = event.round_id;
        let start_offset_ms = offset.saturating_sub(self.pre_roll_ms);
        let end_offset_ms = offset.saturating_add(self.post_roll_ms);
        let mut moment = Moment {
            id: MomentId::new(),
            match_id,
            round_id,
            start_offset_ms,
            end_offset_ms,
            events: vec![event],
            classification: MomentClassification::Other,
            importance: 0.0,
            clip_path: None,
            is_favorite: false,
        };
        Self::reclassify(&mut moment);
        moment
    }

    fn reclassify(moment: &mut Moment) {
        let kills = moment.events.iter().fold(0_u8, |count, event| {
            count.saturating_add(u8::from(event.event_type == GameEventType::Kill))
        });
        let headshots = moment.events.iter().fold(0_u8, |count, event| {
            count.saturating_add(u8::from(event.event_type == GameEventType::Headshot))
        });

        moment.classification = if kills >= 2 {
            MomentClassification::MultiKill
        } else if headshots > 0 {
            MomentClassification::Headshot
        } else {
            MomentClassification::Kill
        };

        let multi_kill_bonus = match kills {
            0 | 1 => 0.0,
            2 => 15.0,
            3 => 30.0,
            4 => 50.0,
            _ => 100.0,
        };
        moment.importance =
            (f32::from(kills) * 10.0) + (f32::from(headshots) * 5.0) + multi_kill_bonus;
    }
}

impl MomentDetector for WindowedMomentDetector {
    fn ingest(&mut self, event: GameEvent) -> openclip_core::Result<Option<Moment>> {
        if !Self::is_qualifying(&event) {
            return Ok(None);
        }
        let Some(offset) = event.match_offset_ms else {
            return Ok(None);
        };

        if let Some(active) = self.active.as_mut()
            && active.match_id == event.match_id
            && offset <= active.end_offset_ms
        {
            active.end_offset_ms = active
                .end_offset_ms
                .max(offset.saturating_add(self.post_roll_ms));
            active.events.push(event);
            Self::reclassify(active);
            return Ok(None);
        }

        let finalized = self.active.take();
        self.active = Some(self.create_moment(event, offset));
        Ok(finalized)
    }

    fn flush(&mut self) -> Option<Moment> {
        self.active.take()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openclip_core::{Game, GameEventId, GameEventType, GameSessionId, MatchId, MomentDetector};
    use serde_json::json;

    use super::*;

    fn event(match_id: MatchId, offset: u64, event_type: GameEventType) -> GameEvent {
        let _session_id = GameSessionId::new();
        GameEvent {
            id: GameEventId::new(),
            match_id,
            round_id: None,
            source_game: Game::CounterStrike2,
            event_type,
            occurred_at: Utc::now(),
            match_offset_ms: Some(offset),
            metadata: json!({}),
        }
    }

    #[test]
    fn nearby_kills_merge_and_extend_the_window() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        assert!(
            detector
                .ingest(event(match_id, 31_000, GameEventType::Kill))
                .unwrap()
                .is_none()
        );
        assert!(
            detector
                .ingest(event(match_id, 34_000, GameEventType::Kill))
                .unwrap()
                .is_none()
        );
        assert!(
            detector
                .ingest(event(match_id, 37_000, GameEventType::Kill))
                .unwrap()
                .is_none()
        );

        let moment = detector.flush().unwrap();
        assert_eq!(moment.start_offset_ms, 21_000);
        assert_eq!(moment.end_offset_ms, 42_000);
        assert_eq!(moment.events.len(), 3);
        assert_eq!(moment.classification, MomentClassification::MultiKill);
        assert!((moment.importance - 60.0).abs() < f32::EPSILON);
    }

    #[test]
    fn distant_event_finalizes_the_previous_moment() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector
            .ingest(event(match_id, 20_000, GameEventType::Kill))
            .unwrap();
        let finalized = detector
            .ingest(event(match_id, 30_001, GameEventType::Kill))
            .unwrap()
            .unwrap();
        assert_eq!(finalized.end_offset_ms, 25_000);
        assert!(detector.flush().is_some());
    }

    #[test]
    fn non_qualifying_events_do_not_open_a_moment() {
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector
            .ingest(event(MatchId::new(), 1_000, GameEventType::RoundStart))
            .unwrap();
        assert!(detector.flush().is_none());
    }
}
