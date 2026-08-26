use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use chrono::Utc;
use openclip_buffer::SegmentReplayBuffer;
use openclip_core::{
    Clip, ClipId, Game, MatchId, OpenClipError, RecordingState, RecordingStatus, Result, Settings,
};

use crate::remux_segments;

const MANUAL_REPLAY_MS: u64 = 15_000;

pub(crate) struct SharedRecorder {
    pub buffer: Mutex<SegmentReplayBuffer>,
    pub status: Mutex<RecordingStatus>,
    pub settings: RwLock<Settings>,
    pub data_root: PathBuf,
    pub replay_root: PathBuf,
    pub timeline_offset_ms: u64,
    pub save_lock: Mutex<()>,
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
            status: Mutex::new(waiting_status()),
            settings: RwLock::new(settings),
            data_root,
            replay_root,
            timeline_offset_ms,
            save_lock: Mutex::new(()),
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
        *write(&self.shared.settings) = settings;
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
        // Holding the buffer lock pins selected segment files against pruning during remux.
        let buffer = lock(&self.shared.buffer);
        let selection = buffer.select_recent(MANUAL_REPLAY_MS).ok_or_else(|| {
            OpenClipError::Unavailable("the replay buffer has no finalized segments".to_owned())
        })?;
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
            "{}_{}_manual.mp4",
            Utc::now().format("%H-%M-%S"),
            &id.to_string()[..8]
        ));
        let paths = selection
            .segments
            .iter()
            .map(|segment| segment.path.clone())
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let media = remux_segments(&paths, &path)?;
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
            created_at: Utc::now(),
            path,
            duration_ms: media.duration_ms,
            width: media.width,
            height: media.height,
            frames_per_second: settings.frames_per_second.min(60),
            codec: "h264".to_owned(),
            encoder: "windows_media_foundation_hardware".to_owned(),
            has_game_audio: media.has_audio,
        })
    }
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
        let mut writer = Writer::new(WriterConfig::default());
        let video = writer.add_track(TrackParams::video(Codec::H264, 1_280, 720).timescale(1_000));
        writer
            .write_sample(
                video,
                InputSample {
                    data: &[0, 0, 0, 1, 0x65],
                    duration: 1_000,
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
}
