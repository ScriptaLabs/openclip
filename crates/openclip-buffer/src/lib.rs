//! Replay-window and moment grouping logic.
//!
//! Encoded-segment storage and extraction arrive with the recording milestone. The moment
//! detector lives here now because its timing contract determines what that buffer must retain.

mod engine;

pub use engine::{CaptureJob, MomentEngine};

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use openclip_core::{
    GameEvent, GameEventType, Moment, MomentClassification, MomentDetector, MomentId, Settings,
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
    /// How much of the requested start is missing because the retained buffer begins later
    /// (recorder started mid-window, or older segments were pruned or lost).
    pub leading_shortfall_ms: u64,
    /// False when a gap between consecutive segments means the clip skips footage.
    pub contiguous: bool,
}

impl ReplaySelection {
    /// True when the selection fully covers what was asked for without skipping footage.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.leading_shortfall_ms == 0 && self.contiguous
    }
}

/// Segment boundaries are millisecond-rounded, so tiny jitter is not a real gap.
const CONTIGUITY_TOLERANCE_MS: u64 = 100;

fn is_contiguous(segments: &[ReplaySegment]) -> bool {
    segments.windows(2).all(|pair| {
        pair[1].started_at_ms.saturating_sub(pair[0].ended_at_ms) <= CONTIGUITY_TOLERANCE_MS
    })
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
        let first_start = segments.first()?.started_at_ms;
        let duration_ms = latest_end.saturating_sub(first_start);
        Some(ReplaySelection {
            contiguous: is_contiguous(&segments),
            segments,
            duration_ms,
            leading_shortfall_ms: first_start.saturating_sub(requested_start),
        })
    }

    /// Selects finalized segments intersecting a logical capture range. The selection may begin
    /// before the requested start so decoding begins at a safe random-access boundary.
    #[must_use]
    pub fn select_range(&self, requested: RangeInclusive<u64>) -> Option<ReplaySelection> {
        let requested_start = *requested.start();
        let requested_end = *requested.end();
        if requested_start > requested_end {
            return None;
        }
        let start = self.segments.iter().position(|segment| {
            segment.begins_with_keyframe && segment.ended_at_ms > requested_start
        })?;
        let segments = self
            .segments
            .iter()
            .skip(start)
            .take_while(|segment| segment.started_at_ms <= requested_end)
            .cloned()
            .collect::<Vec<_>>();
        let first = segments.first()?;
        let last = segments.last()?;
        if last.ended_at_ms < requested_end {
            return None;
        }
        Some(ReplaySelection {
            duration_ms: last.ended_at_ms.saturating_sub(first.started_at_ms),
            leading_shortfall_ms: first.started_at_ms.saturating_sub(requested_start),
            contiguous: is_contiguous(&segments),
            segments,
        })
    }

    #[must_use]
    pub fn latest_end_ms(&self) -> Option<u64> {
        self.segments.back().map(|segment| segment.ended_at_ms)
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
    merge_window_ms: u64,
    active: Option<Moment>,
    capture_kills: bool,
    capture_headshots: bool,
    merge_multi_kills: bool,
}

impl WindowedMomentDetector {
    #[must_use]
    pub const fn new(pre_roll_ms: u64, post_roll_ms: u64) -> Self {
        Self {
            pre_roll_ms,
            post_roll_ms,
            merge_window_ms: post_roll_ms,
            active: None,
            capture_kills: true,
            capture_headshots: true,
            merge_multi_kills: true,
        }
    }

    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            pre_roll_ms: u64::from(settings.pre_roll_seconds) * 1_000,
            post_roll_ms: u64::from(settings.post_roll_seconds) * 1_000,
            merge_window_ms: merge_window_ms(settings),
            active: None,
            capture_kills: settings.capture_kills,
            capture_headshots: settings.capture_headshots,
            merge_multi_kills: settings.capture_multi_kills,
        }
    }

    pub fn update_settings(&mut self, settings: &Settings) {
        self.pre_roll_ms = u64::from(settings.pre_roll_seconds) * 1_000;
        self.post_roll_ms = u64::from(settings.post_roll_seconds) * 1_000;
        self.merge_window_ms = merge_window_ms(settings);
        self.capture_kills = settings.capture_kills;
        self.capture_headshots = settings.capture_headshots;
        self.merge_multi_kills = settings.capture_multi_kills;
    }

    #[must_use]
    pub const fn active(&self) -> Option<&Moment> {
        self.active.as_ref()
    }

    /// Overrides the multi-kill window (the default equals the post-roll).
    #[must_use]
    pub const fn with_merge_window(mut self, merge_window_ms: u64) -> Self {
        self.merge_window_ms = merge_window_ms;
        self
    }

    /// How long after a kill the next one still joins the same moment. Never shorter than the
    /// post-roll, so a short clip tail cannot make grouping stricter.
    #[must_use]
    pub const fn merge_window_ms(&self) -> u64 {
        if self.merge_window_ms > self.post_roll_ms {
            self.merge_window_ms
        } else {
            self.post_roll_ms
        }
    }

    /// Offset at which the open moment stops accepting kills. This can be later than the end of
    /// its clip: the clip ends one post-roll after the last kill, but the moment waits a little
    /// longer in case another kill follows.
    #[must_use]
    pub fn closes_at_ms(&self) -> Option<u64> {
        let active = self.active.as_ref()?;
        let last = active
            .events
            .last()
            .and_then(|event| event.match_offset_ms)
            .unwrap_or_else(|| active.end_offset_ms.saturating_sub(self.post_roll_ms));
        Some(last.saturating_add(self.merge_window_ms()))
    }

    pub fn finalize_expired(&mut self, now_offset_ms: u64) -> Option<Moment> {
        if self
            .closes_at_ms()
            .is_some_and(|closes| now_offset_ms >= closes)
        {
            return self.active.take();
        }
        None
    }

    fn is_qualifying(&self, event: &GameEvent) -> bool {
        match event.event_type {
            GameEventType::Kill => self.capture_kills,
            GameEventType::Headshot => self.capture_headshots,
            _ => false,
        }
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
            clip_id: None,
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
        let reported_headshots = moment.events.iter().fold(0_u8, |count, event| {
            count.saturating_add(u8::from(event.event_type == GameEventType::Headshot))
        });
        let tagged_headshots = moment.events.iter().fold(0_u8, |count, event| {
            count.saturating_add(u8::from(
                event.event_type == GameEventType::Kill && metadata_flag(event, "headshot"),
            ))
        });
        let headshots = reported_headshots.max(tagged_headshots);

        moment.classification = match kills {
            0 | 1 if headshots > 0 => MomentClassification::Headshot,
            0 | 1 => MomentClassification::Kill,
            2 => MomentClassification::DoubleKill,
            3 => MomentClassification::TripleKill,
            4 => MomentClassification::QuadKill,
            _ => MomentClassification::FiveKill,
        };

        let multi_kill_bonus = match kills {
            0 | 1 => 0.0,
            2 => 10.0,
            3 => 25.0,
            4 => 40.0,
            _ => 60.0,
        };
        let special_bonus = moment
            .events
            .iter()
            .filter(|event| event.event_type == GameEventType::Kill)
            .map(special_kill_score)
            .sum::<f32>();
        moment.importance = (f32::from(kills) * 10.0)
            + (f32::from(headshots) * 5.0)
            + multi_kill_bonus
            + special_bonus;
    }
}

fn merge_window_ms(settings: &Settings) -> u64 {
    u64::from(settings.multi_kill_window_seconds.clamp(2, 30)) * 1_000
}

fn metadata_flag(event: &GameEvent, name: &str) -> bool {
    event
        .metadata
        .get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn special_kill_score(event: &GameEvent) -> f32 {
    let headshot = metadata_flag(event, "headshot");
    let one_shot = metadata_flag(event, "oneShotOneKill");
    let aerial = metadata_flag(event, "aerial");
    let mut score = 0.0;
    if one_shot {
        score += 10.0;
        if headshot {
            score += 20.0;
        }
    }
    if metadata_flag(event, "knifeKill") {
        score += 25.0;
    }
    if metadata_flag(event, "zeusKill") {
        score += 20.0;
    }
    if metadata_flag(event, "noScope") {
        score += 15.0;
    }
    if aerial {
        score += 15.0;
        if headshot {
            score += 10.0;
        }
    }
    score
}

/// A chain of kills that never pauses (deathmatch, retakes) must not become one endless clip.
const MAX_MOMENT_SPAN_MS: u64 = 60_000;

impl MomentDetector for WindowedMomentDetector {
    fn ingest(&mut self, event: GameEvent) -> openclip_core::Result<Option<Moment>> {
        let offset = event.match_offset_ms;
        let boundary = match event.event_type {
            GameEventType::MatchEnd => self
                .active
                .as_ref()
                .is_some_and(|active| active.match_id == event.match_id),
            GameEventType::RoundEnd => self.active.as_ref().is_some_and(|active| {
                active.match_id == event.match_id && active.round_id == event.round_id
            }),
            GameEventType::MatchStart | GameEventType::RoundStart => {
                self.active.as_ref().is_some_and(|active| {
                    active.match_id != event.match_id || active.round_id != event.round_id
                })
            }
            _ => false,
        };
        if boundary {
            // A round or match boundary stops further merging, but it must not steal the
            // post-roll already promised to the triggering event. Last-kill highlights are
            // otherwise finalized before the kill animation reaches the encoded timeline.
            return Ok(self.active.take());
        }
        if !self.is_qualifying(&event) {
            return Ok(None);
        }
        let Some(offset) = offset else {
            return Ok(None);
        };

        let window = self.merge_window_ms();
        if let Some(active) = self.active.as_mut()
            && active.match_id == event.match_id
            && active.round_id == event.round_id
            && active
                .events
                .last()
                .and_then(|last| last.match_offset_ms)
                .is_none_or(|last| offset <= last.saturating_add(window))
            && active
                .events
                .first()
                .and_then(|first| first.match_offset_ms)
                .is_none_or(|first| offset.saturating_sub(first) < MAX_MOMENT_SPAN_MS)
            && (self.merge_multi_kills
                || (event.event_type == GameEventType::Headshot
                    && active.events.last().and_then(|item| item.match_offset_ms) == Some(offset)))
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
    fn ranged_selection_includes_complete_intersecting_segments() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        for index in 0..6 {
            buffer.push(segment(index, true));
        }
        let selection = buffer.select_range(3_500..=8_200).expect("selection");
        assert_eq!(selection.segments[0].started_at_ms, 2_000);
        assert_eq!(selection.segments.last().unwrap().ended_at_ms, 10_000);
        assert_eq!(selection.duration_ms, 8_000);
    }

    #[test]
    fn ranged_selection_reports_missing_leading_footage() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        for index in 3..6 {
            buffer.push(segment(index, true));
        }
        let selection = buffer.select_range(2_000..=11_000).expect("selection");
        assert_eq!(selection.leading_shortfall_ms, 4_000);
        assert!(!selection.is_complete());
    }

    #[test]
    fn ranged_selection_reports_gaps_between_segments() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        buffer.push(segment(0, true));
        buffer.push(segment(1, true));
        buffer.push(segment(3, true));
        let selection = buffer.select_range(0..=7_500).expect("selection");
        assert_eq!(selection.leading_shortfall_ms, 0);
        assert!(!selection.contiguous);
        assert!(!selection.is_complete());
    }

    #[test]
    fn contiguous_full_coverage_is_complete() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        for index in 0..6 {
            buffer.push(segment(index, true));
        }
        assert!(
            buffer
                .select_range(3_500..=8_200)
                .expect("selection")
                .is_complete()
        );
    }

    #[test]
    fn ranged_selection_waits_when_requested_end_is_not_finalized() {
        let mut buffer = SegmentReplayBuffer::new(60_000, 10_000);
        buffer.push(segment(0, true));
        assert!(buffer.select_range(0..=2_500).is_none());
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

    fn special_kill(match_id: MatchId, offset: u64) -> GameEvent {
        let mut event = event(match_id, offset, GameEventType::Kill);
        event.metadata = json!({
            "weapon": "weapon_ssg08",
            "headshot": true,
            "oneShotOneKill": true,
            "noScope": true,
            "aerial": true
        });
        event
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
        assert_eq!(moment.classification, MomentClassification::TripleKill);
        assert!((moment.importance - 55.0).abs() < f32::EPSILON);
    }

    #[test]
    fn an_unbroken_kill_chain_is_split_into_bounded_moments() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        let mut finalized = Vec::new();
        // A kill every 4 s for 2 minutes never lets the post-roll expire.
        for second in (0..120).step_by(4) {
            let offset = 10_000 + u64::try_from(second).unwrap() * 1_000;
            if let Some(moment) = detector
                .ingest(event(match_id, offset, GameEventType::Kill))
                .unwrap()
            {
                finalized.push(moment);
            }
        }
        finalized.extend(detector.flush());
        assert!(finalized.len() >= 2, "got {} moments", finalized.len());
        for moment in &finalized {
            let first = moment.events.first().unwrap().match_offset_ms.unwrap();
            let last = moment.events.last().unwrap().match_offset_ms.unwrap();
            assert!(last - first < 60_000);
        }
        let kills: usize = finalized.iter().map(|moment| moment.events.len()).sum();
        assert_eq!(kills, 30, "no kill is dropped by splitting");
    }

    #[test]
    fn single_kill_expires_as_one_moment() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector
            .ingest(event(match_id, 20_000, GameEventType::Kill))
            .unwrap();
        assert!(detector.finalize_expired(24_999).is_none());
        let moment = detector.finalize_expired(25_000).expect("expired moment");
        assert_eq!(moment.classification, MomentClassification::Kill);
        assert_eq!(moment.events.len(), 1);
    }

    #[test]
    fn two_nearby_kills_classify_as_double_kill() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector
            .ingest(event(match_id, 20_000, GameEventType::Kill))
            .unwrap();
        detector
            .ingest(event(match_id, 22_000, GameEventType::Kill))
            .unwrap();
        assert_eq!(
            detector.flush().unwrap().classification,
            MomentClassification::DoubleKill
        );
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

    #[test]
    fn round_boundary_closes_active_moment() {
        let match_id = MatchId::new();
        let round_id = openclip_core::RoundId::new();
        let mut kill = event(match_id, 20_000, GameEventType::Kill);
        kill.round_id = Some(round_id);
        let mut boundary = event(match_id, 22_000, GameEventType::RoundEnd);
        boundary.round_id = Some(round_id);
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector.ingest(kill).unwrap();
        let finalized = detector.ingest(boundary).unwrap().expect("finalized");
        assert_eq!(finalized.end_offset_ms, 25_000);
        assert!(detector.active().is_none());
    }

    #[test]
    fn match_boundary_closes_active_moment() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector
            .ingest(event(match_id, 20_000, GameEventType::Kill))
            .unwrap();
        let finalized = detector
            .ingest(event(match_id, 22_000, GameEventType::MatchEnd))
            .unwrap()
            .expect("finalized");
        assert_eq!(finalized.match_id, match_id);
        assert!(detector.active().is_none());
    }

    #[test]
    fn disabled_kills_do_not_open_a_moment() {
        let settings = Settings {
            capture_kills: false,
            capture_headshots: false,
            ..Settings::default()
        };
        let mut detector = WindowedMomentDetector::from_settings(&settings);
        detector
            .ingest(event(MatchId::new(), 1_000, GameEventType::Kill))
            .unwrap();
        assert!(detector.active().is_none());
    }

    #[test]
    fn combinable_special_kills_receive_additive_importance() {
        let match_id = MatchId::new();
        let mut detector = WindowedMomentDetector::new(10_000, 5_000);
        detector.ingest(special_kill(match_id, 20_000)).unwrap();
        let moment = detector.flush().expect("special moment");
        assert_eq!(moment.classification, MomentClassification::Headshot);
        assert!((moment.importance - 85.0).abs() < f32::EPSILON);
    }
}
