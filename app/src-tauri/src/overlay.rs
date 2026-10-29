//! Post-capture overlay window.
//!
//! A separate, transparent, click-through window that never takes focus. It is shown only while a
//! notification is on screen (an always-visible topmost window can stop the game from using its
//! most efficient presentation path) and only after a moment has been finalized, so it is never
//! part of the footage.

use std::time::Duration;

use openclip_core::{Moment, OverlayPosition, Settings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::automatic::{AutomaticMomentResult, CaptureStatus};

pub(crate) const OVERLAY_LABEL: &str = "overlay";

/// Logical size of the overlay window; the card inside is smaller and anchored to the corner.
const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 340.0;
const MARGIN: f64 = 28.0;
/// Lets the compositor attach the freshly shown window before the entrance animation starts.
const SHOW_SETTLE: Duration = Duration::from_millis(80);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPayload<'a> {
    moment: &'a Moment,
    status: CaptureStatus,
    missing_ms: u64,
    clip_preview: bool,
    overlay_position: OverlayPosition,
    post_roll_seconds: u16,
    sounds_enabled: bool,
    sound_volume: u8,
}

/// Creates the hidden overlay window. Failure is non-fatal: capture works without it.
pub(crate) fn create(app: &AppHandle) {
    let built = WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("index.html".into()))
        .title("OpenClip overlay")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        // The notification sound plays without a user gesture in this window.
        .additional_browser_args(
            "--autoplay-policy=no-user-gesture-required \
             --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection",
        )
        .build();
    match built {
        Ok(window) => {
            if let Err(error) = window.set_ignore_cursor_events(true) {
                tracing::warn!(%error, "overlay could not become click-through");
            }
        }
        Err(error) => tracing::warn!(%error, "overlay window could not be created"),
    }
}

/// Shows the overlay for a finalized moment, honouring the user's settings.
pub(crate) fn present(app: &AppHandle, settings: &Settings, result: &AutomaticMomentResult) {
    if !settings.show_clip_notifications {
        return;
    }
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return;
    };
    let clip_preview = settings.overlay_clip_preview
        && result.captured
        && match result.moment.clip_path.as_deref() {
            // Grant the webview read access to exactly this clip, nothing broader.
            Some(path) => app.asset_protocol_scope().allow_file(path).is_ok(),
            None => false,
        };

    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let (x, y) = overlay_origin(
            settings.overlay_position,
            (monitor.position().x, monitor.position().y),
            monitor.size().width,
            scale,
        );
        if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
            tracing::warn!(%error, "overlay could not be positioned");
        }
    }
    if let Err(error) = window.show() {
        tracing::warn!(%error, "overlay could not be shown");
        return;
    }
    std::thread::sleep(SHOW_SETTLE);
    let payload = OverlayPayload {
        moment: &result.moment,
        status: result.status,
        missing_ms: result.missing_ms,
        clip_preview,
        overlay_position: settings.overlay_position,
        post_roll_seconds: settings.post_roll_seconds,
        sounds_enabled: settings.sounds_enabled,
        sound_volume: settings.sound_volume.min(100),
    };
    if let Err(error) = app.emit_to(OVERLAY_LABEL, "overlay_show", payload) {
        tracing::warn!(%error, "overlay event could not be delivered");
    }
}

pub(crate) fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL)
        && let Err(error) = window.hide()
    {
        tracing::warn!(%error, "overlay could not be hidden");
    }
}

/// Physical top-left corner of the overlay window on a monitor.
fn overlay_origin(
    position: OverlayPosition,
    monitor_origin: (i32, i32),
    monitor_width: u32,
    scale: f64,
) -> (i32, i32) {
    let margin = (MARGIN * scale).round();
    let width = (WIDTH * scale).round();
    let x = match position {
        OverlayPosition::TopLeft => f64::from(monitor_origin.0) + margin,
        OverlayPosition::TopRight => {
            f64::from(monitor_origin.0) + f64::from(monitor_width) - width - margin
        }
    };
    #[allow(clippy::cast_possible_truncation)]
    let (x, y) = (x as i32, (f64::from(monitor_origin.1) + margin) as i32);
    (x, y)
}

/// A synthetic quad kill used by the Settings "Test overlay" button. It previews the newest
/// existing clip when there is one, so the whole overlay can be checked without a CS2 match.
pub(crate) fn demo_result(
    status: CaptureStatus,
    clip_path: Option<std::path::PathBuf>,
) -> AutomaticMomentResult {
    use chrono::Utc;
    use openclip_core::{
        Game, GameEvent, GameEventId, GameEventType, MatchId, MomentClassification, MomentId,
    };

    let match_id = MatchId::new();
    let events = (0..4)
        .map(|index| GameEvent {
            id: GameEventId::new(),
            match_id,
            round_id: None,
            source_game: Game::CounterStrike2,
            event_type: GameEventType::Kill,
            occurred_at: Utc::now(),
            match_offset_ms: Some(1_000 * index),
            metadata: serde_json::json!({ "weapon": "weapon_ak47", "headshot": index % 2 == 0 }),
        })
        .collect();
    let has_clip = status != CaptureStatus::Failed && clip_path.is_some();
    AutomaticMomentResult {
        moment: Moment {
            id: MomentId::new(),
            match_id,
            round_id: None,
            start_offset_ms: 0,
            end_offset_ms: 20_000,
            events,
            classification: MomentClassification::QuadKill,
            importance: 100.0,
            clip_id: None,
            clip_path: if has_clip { clip_path } else { None },
            is_favorite: false,
        },
        captured: status != CaptureStatus::Failed,
        status,
        missing_ms: if status == CaptureStatus::Partial {
            4_000
        } else {
            0
        },
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_left_sits_in_the_corner_with_a_margin() {
        assert_eq!(
            overlay_origin(OverlayPosition::TopLeft, (0, 0), 2560, 1.0),
            (28, 28)
        );
    }

    #[test]
    fn top_right_accounts_for_width_margin_and_scale() {
        // 1920 - 400 - 28 on a 1.0 scale monitor.
        assert_eq!(
            overlay_origin(OverlayPosition::TopRight, (0, 0), 1920, 1.0),
            (1492, 28)
        );
        // Physical 3840 wide at 2x: 3840 - 800 - 56.
        assert_eq!(
            overlay_origin(OverlayPosition::TopRight, (0, 0), 3840, 2.0),
            (2984, 56)
        );
    }

    #[test]
    fn secondary_monitor_offsets_are_respected() {
        assert_eq!(
            overlay_origin(OverlayPosition::TopLeft, (-1920, 0), 1920, 1.0),
            (-1892, 28)
        );
    }
}
