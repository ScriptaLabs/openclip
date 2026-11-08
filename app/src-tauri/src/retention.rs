//! Keeping saved clips within the user's age and size limits.
//!
//! Disabled until the user sets a limit. When a limit is exceeded the oldest clips are deleted first;
//! favorites and clips saved in the last few minutes are never touched. The video file is removed
//! first, and the clip is forgotten only once its file is gone, so a clip that cannot be deleted (for
//! example because it is open in a player) is simply tried again at the next sweep.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use openclip_core::{Clip, MatchRepository, RetentionPolicy, StoredClip, plan_retention};
use openclip_storage::SqliteRepository;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// First sweep shortly after start, so the app is not busy while CS2 is launching.
const FIRST_SWEEP: Duration = Duration::from_secs(45);
const SWEEP_INTERVAL: Duration = Duration::from_mins(30);

/// What a set of limits would do, shown before the user applies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionPreview {
    pub total_clips: usize,
    pub total_bytes: u64,
    pub favorite_clips: usize,
    pub delete_clips: usize,
    pub delete_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SweepResult {
    pub deleted: usize,
    pub freed_bytes: u64,
}

struct Entry {
    clip: Clip,
    size_bytes: u64,
}

fn entries(repository: &impl MatchRepository) -> openclip_core::Result<Vec<Entry>> {
    Ok(repository
        .list_clips()?
        .into_iter()
        .filter_map(|clip| {
            // A clip whose file is already gone has nothing to free and nothing to protect.
            let size_bytes = std::fs::metadata(&clip.path)
                .ok()
                .filter(std::fs::Metadata::is_file)?
                .len();
            Some(Entry { clip, size_bytes })
        })
        .collect())
}

fn stored(entries: &[Entry]) -> Vec<StoredClip> {
    entries
        .iter()
        .map(|entry| StoredClip {
            id: entry.clip.id,
            created_at: entry.clip.created_at,
            size_bytes: entry.size_bytes,
            favorite: entry.clip.is_favorite,
        })
        .collect()
}

pub(crate) fn preview(
    repository: &impl MatchRepository,
    policy: RetentionPolicy,
    now: DateTime<Utc>,
) -> openclip_core::Result<RetentionPreview> {
    let entries = entries(repository)?;
    let plan = plan_retention(&stored(&entries), policy, now);
    Ok(RetentionPreview {
        total_clips: entries.len(),
        total_bytes: entries.iter().map(|entry| entry.size_bytes).sum(),
        favorite_clips: entries
            .iter()
            .filter(|entry| entry.clip.is_favorite)
            .count(),
        delete_clips: plan.delete.len(),
        delete_bytes: plan.freed_bytes,
    })
}

/// Deletes what the limits call for. Failures to remove a file leave that clip in place.
pub(crate) fn sweep(
    repository: &mut impl MatchRepository,
    policy: RetentionPolicy,
    now: DateTime<Utc>,
) -> openclip_core::Result<SweepResult> {
    if !policy.is_active() {
        return Ok(SweepResult::default());
    }
    let entries = entries(repository)?;
    let plan = plan_retention(&stored(&entries), policy, now);
    let mut result = SweepResult::default();
    for id in plan.delete {
        let Some(entry) = entries.iter().find(|entry| entry.clip.id == id) else {
            continue;
        };
        if let Err(error) = remove_clip_files(&entry.clip.path) {
            tracing::warn!(%error, path = %entry.clip.path.display(), "could not delete a clip; will retry");
            continue;
        }
        repository.delete_clip(id)?;
        result.deleted += 1;
        result.freed_bytes += entry.size_bytes;
    }
    if result.deleted > 0 {
        tracing::info!(
            deleted = result.deleted,
            freed_mb = result.freed_bytes / (1 << 20),
            "retention removed old clips"
        );
    }
    Ok(result)
}

/// Removes the video and its thumbnail. Only a recorded `.mp4` file is ever touched.
fn remove_clip_files(path: &Path) -> std::io::Result<()> {
    let is_video = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
    if !is_video || !path.is_file() {
        return Err(std::io::Error::other("not a clip file"));
    }
    std::fs::remove_file(path)?;
    let mut thumbnail = path.file_stem().unwrap_or_default().to_os_string();
    thumbnail.push(".thumb.jpg");
    let _ = std::fs::remove_file(path.with_file_name(thumbnail));
    Ok(())
}

/// Sweeps in the background for as long as the app runs.
pub(crate) fn start(
    app: AppHandle,
    repository: Arc<Mutex<SqliteRepository>>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("openclip-retention".to_owned())
        .spawn(move || {
            thread::sleep(FIRST_SWEEP);
            loop {
                if let Ok(mut repository) = repository.lock() {
                    let policy = repository
                        .load_settings()
                        .map(|settings| settings.retention_policy())
                        .unwrap_or_default();
                    match sweep(&mut *repository, policy, Utc::now()) {
                        Ok(result) if result.deleted > 0 => {
                            let _ = app.emit("clips_changed", result.deleted);
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!(%error, "retention sweep failed"),
                    }
                }
                thread::sleep(SWEEP_INTERVAL);
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Duration as ChronoDuration;
    use openclip_core::{ClipId, ClipSource};

    use super::*;

    fn clip_on_disk(
        directory: &Path,
        name: &str,
        bytes: usize,
        age_days: i64,
        favorite: bool,
    ) -> Clip {
        let path = directory.join(format!("{name}.mp4"));
        std::fs::write(&path, vec![0_u8; bytes]).unwrap();
        std::fs::write(directory.join(format!("{name}.thumb.jpg")), b"jpg").unwrap();
        Clip {
            id: ClipId::new(),
            match_id: None,
            moment_id: None,
            source: ClipSource::Manual,
            created_at: Utc::now() - ChronoDuration::days(age_days),
            path,
            duration_ms: 1_000,
            width: 1_280,
            height: 720,
            frames_per_second: 60,
            codec: "h264".to_owned(),
            encoder: "test".to_owned(),
            has_game_audio: false,
            is_favorite: favorite,
        }
    }

    fn setup(clips: &[Clip]) -> SqliteRepository {
        let mut repository = SqliteRepository::in_memory().unwrap();
        for clip in clips {
            repository.save_clip(clip).unwrap();
            if clip.is_favorite {
                repository.set_clip_favorite(clip.id, true).unwrap();
            }
        }
        repository
    }

    #[test]
    fn old_clips_and_their_thumbnails_are_deleted_but_favorites_and_new_clips_stay() {
        let directory = tempfile::tempdir().unwrap();
        let old = clip_on_disk(directory.path(), "old", 100, 60, false);
        let old_favorite = clip_on_disk(directory.path(), "keeper", 100, 90, true);
        let fresh = clip_on_disk(directory.path(), "fresh", 100, 1, false);
        let mut repository = setup(&[old.clone(), old_favorite.clone(), fresh.clone()]);
        let policy = RetentionPolicy {
            max_age_days: Some(30),
            max_total_bytes: None,
        };

        let result = sweep(&mut repository, policy, Utc::now()).unwrap();
        assert_eq!(
            result,
            SweepResult {
                deleted: 1,
                freed_bytes: 100
            }
        );
        assert!(!old.path.exists());
        assert!(!directory.path().join("old.thumb.jpg").exists());
        assert!(old_favorite.path.exists() && fresh.path.exists());
        let left: Vec<_> = repository
            .list_clips()
            .unwrap()
            .into_iter()
            .map(|clip| clip.id)
            .collect();
        assert!(
            left.contains(&old_favorite.id) && left.contains(&fresh.id) && !left.contains(&old.id)
        );
    }

    #[test]
    fn the_oldest_clips_go_first_when_the_size_limit_is_passed() {
        let directory = tempfile::tempdir().unwrap();
        let oldest = clip_on_disk(directory.path(), "a", 600, 20, false);
        let middle = clip_on_disk(directory.path(), "b", 600, 10, false);
        let newest = clip_on_disk(directory.path(), "c", 600, 2, false);
        let mut repository = setup(&[oldest.clone(), middle.clone(), newest.clone()]);
        // 1_800 bytes on disk and a limit of 1_000 bytes. Whole gigabytes are the user-facing unit;
        // the planner works in bytes, so the test sets a byte limit directly.
        let policy = RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(1_000),
        };
        let result = sweep(&mut repository, policy, Utc::now()).unwrap();
        assert_eq!(result.deleted, 2);
        assert!(!oldest.path.exists() && !middle.path.exists() && newest.path.exists());
    }

    #[test]
    fn nothing_happens_without_a_limit_and_the_preview_does_not_delete() {
        let directory = tempfile::tempdir().unwrap();
        let old = clip_on_disk(directory.path(), "old", 100, 400, false);
        let mut repository = setup(std::slice::from_ref(&old));
        assert_eq!(
            sweep(&mut repository, RetentionPolicy::default(), Utc::now()).unwrap(),
            SweepResult::default()
        );
        let policy = RetentionPolicy {
            max_age_days: Some(30),
            max_total_bytes: None,
        };
        let shown = preview(&repository, policy, Utc::now()).unwrap();
        assert_eq!(
            shown,
            RetentionPreview {
                total_clips: 1,
                total_bytes: 100,
                favorite_clips: 0,
                delete_clips: 1,
                delete_bytes: 100,
            }
        );
        assert!(old.path.exists(), "a preview never deletes");
    }

    #[test]
    fn a_file_that_cannot_be_removed_keeps_its_record() {
        let directory = tempfile::tempdir().unwrap();
        let mut old = clip_on_disk(directory.path(), "old", 100, 60, false);
        // Not an .mp4, so it is refused rather than deleted.
        let odd = directory.path().join("old.bin");
        std::fs::rename(&old.path, &odd).unwrap();
        old.path = odd.clone();
        let mut repository = setup(&[old.clone()]);
        let policy = RetentionPolicy {
            max_age_days: Some(30),
            max_total_bytes: None,
        };
        let result = sweep(&mut repository, policy, Utc::now()).unwrap();
        assert_eq!(result.deleted, 0);
        assert!(odd.exists());
        assert_eq!(repository.list_clips().unwrap().len(), 1);
    }
}
