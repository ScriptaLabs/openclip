//! Replay-window and moment grouping logic.
//!
//! Encoded-segment storage and extraction arrive with the recording milestone. The moment
//! detector lives here now because its timing contract determines what that buffer must retain.

use std::collections::VecDeque;
use std::path::PathBuf;

use openclip_core::{
    GameEvent, GameEventType, Moment, MomentClassification, MomentDetector, MomentId,
};

/// One independently playable encoded media segment. Segment creation restarts the encoder,
/// which makes the first video sample a random-access point without retaining raw frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySegment {
    pub path: PathBuf,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub size_bytes: u64,
    pub begins_with_keyframe: bool,
}

impl ReplaySegment {
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.ended_at_ms.saturating_sub(self.started_at_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySelection {
    pub segments: Vec<ReplaySegment>,
    pub duration_ms: u64,
}

/// Hard-bounded, encoded-only rolling replay window.
#[derive(Debug)]
pub struct SegmentReplayBuffer {
    segments: VecDeque<ReplaySegment>,
    max_duration_ms: u64,
    max_bytes: u64,
    total_bytes: u64,
}

impl SegmentReplayBuffer {
    #[must_use]
    pub const fn new(max_duration_ms: u64, max_bytes: u64) -> Self {
        Self {
            segments: VecDeque::new(),
            max_duration_ms,
            max_bytes,
            total_bytes: 0,
        }
    }

    pub fn push(&mut self, segment: ReplaySegment) -> Vec<ReplaySegment> {
        self.total_bytes = self.total_bytes.saturating_add(segment.size_bytes);
        self.segments.push_back(segment);
        self.prune()
    }

    pub fn clear(&mut self) -> Vec<ReplaySegment> {
        self.total_bytes = 0;
        self.segments.drain(..).collect()
    }

    fn prune(&mut self) -> Vec<ReplaySegment> {
        let mut removed = Vec::new();
        while self.total_bytes > self.max_bytes || self.duration_ms() > self.max_duration_ms {
            let Some(segment) = self.segments.pop_front() else {
                break;
            };
            self.total_bytes = self.total_bytes.saturating_sub(segment.size_bytes);
            removed.push(segment);
        }
        removed
    }

    #[must_use]
    pub fn select_recent(&self, requested_ms: u64) -> Option<ReplaySelection> {
        let latest_end = self.segments.back()?.ended_at_ms;
        let requested_start = latest_end.saturating_sub(requested_ms);
        let start = self.segments.iter().position(|segment| {
            segment.begins_with_keyframe && segment.ended_at_ms > requested_start
        })?;
        let segments = self
            .segments
            .iter()
            .skip(start)
            .cloned()
            .collect::<Vec<_>>();
        let duration_ms = latest_end.saturating_sub(segments.first()?.started_at_ms);
        Some(ReplaySelection {
            segments,
            duration_ms,
        })
    }

    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.segments
            .front()
            .zip(self.segments.back())
            .map_or(0, |(first, last)| {
                last.ended_at_ms.saturating_sub(first.started_at_ms)
            })
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

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

    fn segment(index: u64, keyframe: bool) -> ReplaySegment {
        ReplaySegment {
            path: format!("{index}.mp4").into(),
            started_at_ms: index * 2_000,
            ended_at_ms: (index + 1) * 2_000,
            size_bytes: 100,
            begins_with_keyframe: keyframe,
        }
    }

    #[test]
    fn replay_buffer_is_bounded_by_time_and_bytes() {
        let mut buffer = SegmentReplayBuffer::new(6_000, 350);
        for index in 0..5 {
            buffer.push(segment(index, true));
        }
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.duration_ms(), 6_000);
        assert_eq!(buffer.total_bytes(), 300);
    }

    #[test]
    fn recent_selection_starts_only_at_a_keyframe() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        for (index, keyframe) in [true, false, true, true, true].into_iter().enumerate() {
            buffer.push(segment(index as u64, keyframe));
        }
        let selection = buffer.select_recent(7_000).expect("selection");
        assert_eq!(selection.segments[0].path, PathBuf::from("2.mp4"));
        assert_eq!(selection.duration_ms, 6_000);
    }

    #[test]
    fn empty_buffer_has_no_replay() {
        assert!(
            SegmentReplayBuffer::new(30_000, 1_000)
                .select_recent(15_000)
                .is_none()
        );
    }

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
