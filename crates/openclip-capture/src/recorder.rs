use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use openclip_buffer::SegmentReplayBuffer;
use openclip_core::{
    Clip, ClipId, ClipSource, Game, MatchId, MomentId, OpenClipError, RecordingState,
    RecordingStatus, Result, Settings,
};

use crate::remux_segments;

const MANUAL_REPLAY_MS: u64 = 15_000;

pub(crate) struct SharedRecorder {
    pub buffer: Mutex<SegmentReplayBuffer>,
    pub buffer_finalized: Condvar,
    pub status: Mutex<RecordingStatus>,
    pub settings: RwLock<Settings>,
    pub data_root: PathBuf,
    pub replay_root: PathBuf,
    pub timeline_offset_ms: u64,
    pub clock_started: Instant,
    pub save_lock: Mutex<()>,
    pub pinned_segments: Mutex<HashSet<PathBuf>>,
    pub deferred_deletes: Mutex<Vec<PathBuf>>,
    pub settings_generation: AtomicU64,
}

/// A ranged save plus how faithfully the retained buffer covered the request.
#[derive(Debug, Clone)]
pub struct SavedRange {
    pub clip: Clip,
    /// Requested footage missing at the start of the clip.
    pub leading_shortfall_ms: u64,
    /// False when the clip skips footage between segments.
    pub contiguous: bool,
}

impl SavedRange {
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.leading_shortfall_ms == 0 && self.contiguous
    }
}

#[derive(Clone)]
pub struct RecorderService {
    pub(crate) shared: Arc<SharedRecorder>,
    #[cfg(windows)]
    pub(crate) lifecycle: Arc<crate::windows::CaptureLifecycle>,
}

impl RecorderService {
    /// Creates a recorder whose temp replay data is isolated under the application data folder.
    pub fn new(data_root: impl AsRef<Path>, settings: Settings) -> Result<Self> {
        let data_root = data_root.as_ref().to_path_buf();
        let replay_root = data_root.join("replay");
        std::fs::create_dir_all(&replay_root).map_err(capture_error)?;
        let max_bytes = replay_byte_limit(&settings);
        let mut buffer =
            SegmentReplayBuffer::new(u64::from(settings.replay_buffer_seconds) * 1_000, max_bytes);
        let timeline_offset_ms = recover_replay_directory(&replay_root, &mut buffer)?;
        let shared = Arc::new(SharedRecorder {
            buffer: Mutex::new(buffer),
            buffer_finalized: Condvar::new(),
            status: Mutex::new(waiting_status()),
            settings: RwLock::new(settings),
            data_root,
            replay_root,
            timeline_offset_ms,
            clock_started: Instant::now(),
            save_lock: Mutex::new(()),
            pinned_segments: Mutex::new(HashSet::new()),
            deferred_deletes: Mutex::new(Vec::new()),
            settings_generation: AtomicU64::new(0),
        });
        Ok(Self {
            shared,
            #[cfg(windows)]
            lifecycle: Arc::new(crate::windows::CaptureLifecycle::new()),
        })
    }

    pub fn start(&self) -> Result<()> {
        #[cfg(windows)]
        return self.lifecycle.start(self.shared.clone());
        #[cfg(not(windows))]
        Err(OpenClipError::Unavailable(
            "capture is supported only on Windows 10/11".to_owned(),
        ))
    }

    pub fn stop(&self) -> Result<()> {
        #[cfg(windows)]
        self.lifecycle.stop();
        set_status(&self.shared, RecordingState::Idle, "Capture stopped");
        Ok(())
    }

    #[must_use]
    pub fn status(&self) -> RecordingStatus {
        lock(&self.shared.status).clone()
    }

    pub fn update_settings(&self, settings: Settings) {
        let mut current = write(&self.shared.settings);
        let restart_capture = capture_pipeline_changed(&current, &settings);
        *current = settings;
        drop(current);
        if restart_capture {
            self.shared
                .settings_generation
                .fetch_add(1, Ordering::AcqRel);
        }
    }

    #[must_use]
    pub fn timeline_now_ms(&self) -> u64 {
        self.shared.timeline_offset_ms.saturating_add(
            u64::try_from(self.shared.clock_started.elapsed().as_millis()).unwrap_or(u64::MAX),
        )
    }

    /// Saves encoded buffer samples by remuxing only. The caller persists the returned metadata.
    pub fn save_replay(&self, match_id: Option<MatchId>) -> Result<Clip> {
        let _save = lock(&self.shared.save_lock);
        let previous = self.status();
        if !matches!(
            previous.state,
            RecordingState::Recording | RecordingState::SavingReplay
        ) {
            return Err(OpenClipError::Unavailable(
                "no active replay buffer; launch CS2 and wait for Replay active".to_owned(),
            ));
        }
        set_status(&self.shared, RecordingState::SavingReplay, "Saving replay…");
        let result = self.save_replay_inner(match_id);
        *lock(&self.shared.status) = previous;
        result
    }

    fn save_replay_inner(&self, match_id: Option<MatchId>) -> Result<Clip> {
        let selection = {
            let buffer = lock(&self.shared.buffer);
            let selection = buffer.select_recent(MANUAL_REPLAY_MS).ok_or_else(|| {
                OpenClipError::Unavailable("the replay buffer has no finalized segments".to_owned())
            })?;
            self.pin_segments(
                &selection
                    .segments
                    .iter()
                    .map(|segment| segment.path.clone())
                    .collect::<Vec<_>>(),
            );
            selection
        };
        self.save_selection(&selection, match_id, None, ClipSource::Manual, "manual")
    }

    /// Saves a specific capture-timeline range after its newest segment has finalized.
    pub fn save_range(
        &self,
        requested: RangeInclusive<u64>,
        match_id: MatchId,
        moment_id: MomentId,
    ) -> Result<SavedRange> {
        let _save = lock(&self.shared.save_lock);
        let previous = self.status();
        // Automatic moments can finish just after CS2 closes or its capture target is recreated.
        // The independently finalized segments are still valid in that state, so always try the
        // retained buffer before declaring the moment unavailable.
        set_status(
            &self.shared,
            RecordingState::SavingReplay,
            "Saving highlight…",
        );
        let result = self.save_range_inner(requested, match_id, moment_id);
        *lock(&self.shared.status) = previous;
        result
    }

    fn save_range_inner(
        &self,
        requested: RangeInclusive<u64>,
        match_id: MatchId,
        moment_id: MomentId,
    ) -> Result<SavedRange> {
        let requested_end = *requested.end();
        let until_requested_end = requested_end.saturating_sub(self.timeline_now_ms());
        let deadline = Instant::now()
            + Duration::from_millis(until_requested_end.min(30_000))
            + Duration::from_secs(5);
        let mut buffer = lock(&self.shared.buffer);
        while buffer.latest_end_ms().is_none_or(|end| end < requested_end) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(OpenClipError::Unavailable(
                    "the newest replay segment did not finalize before the highlight timeout"
                        .to_owned(),
                ));
            }
            let waited = self
                .shared
                .buffer_finalized
                .wait_timeout(buffer, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            buffer = waited.0;
        }
        let selection = buffer.select_range(requested).ok_or_else(|| {
            OpenClipError::Unavailable(
                "the requested replay range is no longer retained".to_owned(),
            )
        })?;
        self.pin_segments(
            &selection
                .segments
                .iter()
                .map(|segment| segment.path.clone())
                .collect::<Vec<_>>(),
        );
        drop(buffer);
        let clip = self.save_selection(
            &selection,
            Some(match_id),
            Some(moment_id),
            ClipSource::AutomaticMoment,
            "highlight",
        )?;
        Ok(SavedRange {
            clip,
            leading_shortfall_ms: selection.leading_shortfall_ms,
            contiguous: selection.contiguous,
        })
    }

    fn save_selection(
        &self,
        selection: &openclip_buffer::ReplaySelection,
        match_id: Option<MatchId>,
        moment_id: Option<MomentId>,
        source: ClipSource,
        suffix: &str,
    ) -> Result<Clip> {
        let settings = read(&self.shared.settings).clone();
        let clip_root = if settings.storage_path.is_absolute() {
            settings.storage_path.clone()
        } else {
            self.shared.data_root.join(&settings.storage_path)
        };
        let date_root = clip_root.join(Utc::now().format("%Y-%m-%d").to_string());
        std::fs::create_dir_all(&date_root).map_err(capture_error)?;
        let id = ClipId::new();
        let path = date_root.join(format!(
            "{}_{}_{}.mp4",
            Utc::now().format("%H-%M-%S"),
            &id.to_string()[..8],
            suffix,
        ));
        let paths = selection
            .segments
            .iter()
            .map(|segment| segment.path.clone())
            .collect::<Vec<_>>();
        self.pin_segments(&paths);
        let started = std::time::Instant::now();
        let media = remux_segments(&paths, &path);
        self.unpin_segments(&paths);
        let media = media?;
        tracing::info!(
            path = %path.display(),
            duration_ms = media.duration_ms,
            segments = paths.len(),
            save_ms = started.elapsed().as_millis(),
            "replay saved without re-encoding"
        );
        Ok(Clip {
            id,
            match_id,
            moment_id,
            source,
            created_at: Utc::now(),
            path,
            duration_ms: media.duration_ms,
            width: media.width,
            height: media.height,
            frames_per_second: settings.frames_per_second.min(60),
            codec: "h264".to_owned(),
            encoder: lock(&self.shared.status)
                .encoder
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            has_game_audio: settings.capture_game_audio && media.has_audio,
        })
    }

    fn pin_segments(&self, paths: &[PathBuf]) {
        let mut pinned = lock(&self.shared.pinned_segments);
        pinned.extend(paths.iter().cloned());
    }

    fn unpin_segments(&self, paths: &[PathBuf]) {
        {
            let mut pinned = lock(&self.shared.pinned_segments);
            for path in paths {
                pinned.remove(path);
            }
        }
        let mut deferred = lock(&self.shared.deferred_deletes);
        let mut index = 0;
        while index < deferred.len() {
            if paths.contains(&deferred[index]) {
                let path = deferred.swap_remove(index);
                if let Err(error) = std::fs::remove_file(&path) {
                    tracing::warn!(%error, path = %path.display(), "could not delete released replay segment");
                }
            } else {
                index += 1;
            }
        }
    }
}

fn capture_pipeline_changed(previous: &Settings, next: &Settings) -> bool {
    previous.quality != next.quality
        || previous.resolution != next.resolution
        || previous.frames_per_second != next.frames_per_second
        || previous.video_encoder != next.video_encoder
        || previous.capture_game_audio != next.capture_game_audio
        || previous.capture_microphone != next.capture_microphone
        || previous.microphone_device_id != next.microphone_device_id
}

pub(crate) fn waiting_status() -> RecordingStatus {
    RecordingStatus {
        state: RecordingState::WaitingForGame,
        game: Some(Game::CounterStrike2),
        message: Some("Waiting for CS2".to_owned()),
        encoder: None,
        capture_width: None,
        capture_height: None,
        capture_fps: None,
        buffered_seconds: 0,
    }
}

pub(crate) fn set_status(
    shared: &SharedRecorder,
    state: RecordingState,
    message: impl Into<String>,
) {
    let mut status = lock(&shared.status);
    status.state = state;
    status.message = Some(message.into());
    if state != RecordingState::Recording && state != RecordingState::SavingReplay {
        status.buffered_seconds = 0;
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn recover_replay_directory(root: &Path, buffer: &mut SegmentReplayBuffer) -> Result<u64> {
    let mut paths = std::fs::read_dir(root)
        .map_err(capture_error)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(capture_error)?;
    paths.sort_by(|left, right| {
        let left_modified = std::fs::metadata(left)
            .and_then(|metadata| metadata.modified())
            .ok();
        let right_modified = std::fs::metadata(right)
            .and_then(|metadata| metadata.modified())
            .ok();
        left_modified
            .cmp(&right_modified)
            .then_with(|| left.cmp(right))
    });
    let mut cursor_ms = 0_u64;
    for path in paths {
        // Open/incomplete segments deliberately retain `.partial`; they are never indexed.
        if path
            .extension()
            .is_some_and(|extension| extension == "partial")
        {
            tracing::warn!(path = %path.display(), "preserving interrupted replay segment for diagnostics");
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "mp4") {
            continue;
        }
        match crate::validate_finalized_segment(&path) {
            Ok(media) => {
                let started_at_ms = cursor_ms;
                cursor_ms = cursor_ms.saturating_add(media.duration_ms);
                let size_bytes = std::fs::metadata(&path).map_err(capture_error)?.len();
                let removed = buffer.push(openclip_buffer::ReplaySegment {
                    path,
                    started_at_ms,
                    ended_at_ms: cursor_ms,
                    size_bytes,
                    begins_with_keyframe: true,
                });
                for segment in removed {
                    if let Err(error) = std::fs::remove_file(&segment.path) {
                        tracing::warn!(%error, path = %segment.path.display(), "could not prune recovered replay segment");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "ignoring invalid finalized replay segment");
            }
        }
    }
    if !buffer.is_empty() {
        tracing::info!(
            segments = buffer.len(),
            buffered_seconds = buffer.duration_ms() / 1_000,
            "recovered finalized replay segments after restart"
        );
    }
    Ok(cursor_ms)
}

fn replay_byte_limit(settings: &Settings) -> u64 {
    let estimated_high_bitrate = 50_000_000_u64;
    estimated_high_bitrate / 8 * u64::from(settings.replay_buffer_seconds) * 2 + 32 * 1024 * 1024
}

fn capture_error(error: impl std::fmt::Display) -> OpenClipError {
    OpenClipError::Capture(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp4io::{Codec, InputSample, TrackParams, Writer, WriterConfig};

    fn write_finalized_segment(path: &Path) {
        write_finalized_segment_with_duration(path, 1_000);
    }

    fn write_finalized_segment_with_duration(path: &Path, duration: u32) {
        let mut writer = Writer::new(WriterConfig::default());
        let video = writer.add_track(TrackParams::video(Codec::H264, 1_280, 720).timescale(1_000));
        writer
            .write_sample(
                video,
                InputSample {
                    data: &[0, 0, 0, 1, 0x65],
                    duration,
                    composition_offset: 0,
                    is_sync: true,
                },
            )
            .unwrap();
        std::fs::write(path, writer.finalize().unwrap()).unwrap();
    }

    #[test]
    fn recovery_preserves_but_never_indexes_partial_segments() {
        let directory = tempfile::tempdir().expect("temp directory");
        let partial = directory
            .path()
            .join("replay")
            .join("interrupted.mp4.partial");
        std::fs::create_dir_all(partial.parent().unwrap()).unwrap();
        std::fs::write(&partial, b"incomplete").unwrap();
        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();
        assert!(partial.exists());
        assert!(lock(&recorder.shared.buffer).is_empty());
    }

    #[test]
    fn recovery_reindexes_independently_finalized_segments() {
        let directory = tempfile::tempdir().expect("temp directory");
        let replay = directory.path().join("replay");
        std::fs::create_dir_all(&replay).unwrap();
        write_finalized_segment(&replay.join("before-crash.mp4"));
        std::fs::write(replay.join("during-crash.mp4.partial"), b"truncated").unwrap();

        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();
        let buffer = lock(&recorder.shared.buffer);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.duration_ms(), 1_000);
        assert_eq!(recorder.shared.timeline_offset_ms, 1_000);
    }

    #[test]
    fn saving_without_recording_is_a_clear_error() {
        let directory = tempfile::tempdir().expect("temp directory");
        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();
        assert!(recorder.save_replay(None).is_err());
    }

    #[test]
    fn ranged_save_salvages_segments_after_capture_leaves_recording_state() {
        let directory = tempfile::tempdir().expect("temp directory");
        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();
        let saving = recorder.clone();
        let match_id = MatchId::new();
        let moment_id = MomentId::new();
        let handle = std::thread::spawn(move || saving.save_range(0..=2_500, match_id, moment_id));

        std::thread::sleep(Duration::from_millis(50));
        let segment_path = recorder.shared.replay_root.join("pending.mp4");
        write_finalized_segment_with_duration(&segment_path, 3_000);
        let size_bytes = std::fs::metadata(&segment_path).unwrap().len();
        lock(&recorder.shared.buffer).push(openclip_buffer::ReplaySegment {
            path: segment_path,
            started_at_ms: 0,
            ended_at_ms: 3_000,
            size_bytes,
            begins_with_keyframe: true,
        });
        recorder.shared.buffer_finalized.notify_all();

        let clip = handle.join().unwrap().expect("ranged clip saves").clip;
        assert_eq!(clip.source, ClipSource::AutomaticMoment);
        assert_eq!(clip.moment_id, Some(moment_id));
    }

    #[test]
    fn manual_replay_remains_a_distinct_clip_source() {
        let directory = tempfile::tempdir().expect("temp directory");
        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();
        set_status(&recorder.shared, RecordingState::Recording, "testing");
        let segment_path = recorder.shared.replay_root.join("manual-source.mp4");
        write_finalized_segment(&segment_path);
        let size_bytes = std::fs::metadata(&segment_path).unwrap().len();
        lock(&recorder.shared.buffer).push(openclip_buffer::ReplaySegment {
            path: segment_path,
            started_at_ms: 0,
            ended_at_ms: 1_000,
            size_bytes,
            begins_with_keyframe: true,
        });

        let clip = recorder.save_replay(None).expect("manual clip saves");
        assert_eq!(clip.source, ClipSource::Manual);
        assert_eq!(clip.moment_id, None);
    }

    #[test]
    fn only_capture_pipeline_changes_restart_the_capture_session() {
        let directory = tempfile::tempdir().expect("temp directory");
        let recorder = RecorderService::new(directory.path(), Settings::default()).unwrap();

        let presentation_change = Settings {
            show_clip_notifications: false,
            ..Settings::default()
        };
        recorder.update_settings(presentation_change);
        assert_eq!(
            recorder.shared.settings_generation.load(Ordering::Acquire),
            0
        );

        let mut microphone_change = read(&recorder.shared.settings).clone();
        microphone_change.capture_microphone = true;
        recorder.update_settings(microphone_change);
        assert_eq!(
            recorder.shared.settings_generation.load(Ordering::Acquire),
            1
        );
    }
}
