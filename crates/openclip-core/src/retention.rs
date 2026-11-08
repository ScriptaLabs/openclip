//! Deciding which saved clips to delete to stay within the user's limits.
//!
//! Pure logic, so the rules can be tested without files: favorites are never removed, brand-new
//! clips are left alone, and when a size limit is exceeded the oldest clips go first.

use chrono::{DateTime, Duration, Utc};

use crate::{ClipId, Settings};

/// Clips this young are never removed, so one that was just saved cannot disappear under a very
/// small limit before it has been seen.
pub const PROTECT_RECENT: Duration = Duration::minutes(10);

const BYTES_PER_GIB: u64 = 1 << 30;

/// The limits the user chose. `None` means no limit of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetentionPolicy {
    pub max_age_days: Option<u16>,
    pub max_total_bytes: Option<u64>,
}

impl RetentionPolicy {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.max_age_days.is_some() || self.max_total_bytes.is_some()
    }
}

impl Settings {
    #[must_use]
    pub fn retention_policy(&self) -> RetentionPolicy {
        RetentionPolicy {
            max_age_days: self.retention_max_age_days.filter(|days| *days > 0),
            max_total_bytes: self
                .retention_max_size_gb
                .filter(|gigabytes| *gigabytes > 0)
                .map(|gigabytes| u64::from(gigabytes) * BYTES_PER_GIB),
        }
    }
}

/// What the planner needs to know about one saved clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredClip {
    pub id: ClipId,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionPlan {
    /// Clips to delete, oldest first.
    pub delete: Vec<ClipId>,
    pub freed_bytes: u64,
}

/// Chooses the clips to delete so the rest respect `policy`.
#[must_use]
pub fn plan_retention(
    clips: &[StoredClip],
    policy: RetentionPolicy,
    now: DateTime<Utc>,
) -> RetentionPlan {
    let mut ordered: Vec<&StoredClip> = clips.iter().collect();
    ordered.sort_by_key(|clip| clip.created_at);
    let removable = |clip: &StoredClip| !clip.favorite && now - clip.created_at >= PROTECT_RECENT;
    let mut doomed = vec![false; ordered.len()];

    if let Some(days) = policy.max_age_days {
        let cutoff = now - Duration::days(i64::from(days));
        for (index, clip) in ordered.iter().enumerate() {
            doomed[index] = removable(clip) && clip.created_at < cutoff;
        }
    }
    if let Some(limit) = policy.max_total_bytes {
        let mut remaining: u64 = ordered
            .iter()
            .zip(&doomed)
            .filter(|(_, gone)| !**gone)
            .map(|(clip, _)| clip.size_bytes)
            .sum();
        for (index, clip) in ordered.iter().enumerate() {
            if remaining <= limit {
                break;
            }
            if !doomed[index] && removable(clip) {
                doomed[index] = true;
                remaining = remaining.saturating_sub(clip.size_bytes);
            }
        }
    }

    let mut plan = RetentionPlan::default();
    for (clip, gone) in ordered.iter().zip(&doomed) {
        if *gone {
            plan.delete.push(clip.id);
            plan.freed_bytes += clip.size_bytes;
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn clip(age_days: i64, gib: u64, favorite: bool, now: DateTime<Utc>) -> StoredClip {
        StoredClip {
            id: ClipId::new(),
            created_at: now - Duration::days(age_days),
            size_bytes: gib * GIB,
            favorite,
        }
    }

    #[test]
    fn nothing_is_deleted_without_limits() {
        let now = Utc::now();
        let clips = [clip(400, 5, false, now)];
        assert_eq!(
            plan_retention(&clips, RetentionPolicy::default(), now),
            RetentionPlan::default()
        );
    }

    #[test]
    fn clips_older_than_the_limit_go_but_favorites_stay() {
        let now = Utc::now();
        let old = clip(40, 1, false, now);
        let old_favorite = clip(90, 1, true, now);
        let fresh = clip(3, 1, false, now);
        let policy = RetentionPolicy {
            max_age_days: Some(30),
            max_total_bytes: None,
        };
        let plan = plan_retention(&[fresh, old_favorite, old], policy, now);
        assert_eq!(plan.delete, vec![old.id]);
        assert_eq!(plan.freed_bytes, GIB);
    }

    #[test]
    fn the_oldest_clips_go_first_until_the_size_fits() {
        let now = Utc::now();
        let oldest = clip(20, 4, false, now);
        let middle = clip(10, 4, false, now);
        let newest = clip(1, 4, false, now);
        let policy = RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(9 * GIB),
        };
        let plan = plan_retention(&[newest, oldest, middle], policy, now);
        assert_eq!(
            plan.delete,
            vec![oldest.id],
            "12 GiB over a 9 GiB limit needs one 4 GiB clip gone"
        );
        let tighter = RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(5 * GIB),
        };
        let plan = plan_retention(&[newest, oldest, middle], tighter, now);
        assert_eq!(plan.delete, vec![oldest.id, middle.id]);
    }

    #[test]
    fn favorites_may_keep_the_total_above_the_limit() {
        let now = Utc::now();
        let favorite = clip(50, 10, true, now);
        let ordinary = clip(40, 2, false, now);
        let policy = RetentionPolicy {
            max_age_days: None,
            max_total_bytes: Some(GIB),
        };
        let plan = plan_retention(&[favorite, ordinary], policy, now);
        assert_eq!(plan.delete, vec![ordinary.id]);
    }

    #[test]
    fn a_clip_saved_moments_ago_is_never_removed() {
        let now = Utc::now();
        let just_saved = StoredClip {
            id: ClipId::new(),
            created_at: now - Duration::minutes(2),
            size_bytes: 5 * GIB,
            favorite: false,
        };
        let policy = RetentionPolicy {
            max_age_days: Some(1),
            max_total_bytes: Some(GIB),
        };
        assert_eq!(
            plan_retention(&[just_saved], policy, now),
            RetentionPlan::default()
        );
    }

    #[test]
    fn age_and_size_limits_do_not_delete_the_same_clip_twice() {
        let now = Utc::now();
        let ancient = clip(100, 3, false, now);
        let recent = clip(2, 3, false, now);
        let policy = RetentionPolicy {
            max_age_days: Some(30),
            max_total_bytes: Some(4 * GIB),
        };
        let plan = plan_retention(&[ancient, recent], policy, now);
        assert_eq!(plan.delete, vec![ancient.id]);
        assert_eq!(plan.freed_bytes, 3 * GIB);
    }

    #[test]
    fn settings_turn_into_a_policy() {
        let settings = Settings {
            retention_max_age_days: Some(30),
            retention_max_size_gb: Some(50),
            ..Settings::default()
        };
        assert_eq!(
            settings.retention_policy(),
            RetentionPolicy {
                max_age_days: Some(30),
                max_total_bytes: Some(50 * GIB),
            }
        );
        assert!(!Settings::default().retention_policy().is_active());
    }
}
