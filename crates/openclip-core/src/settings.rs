use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingQuality {
    Performance,
    High,
    Ultra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingResolution {
    GameResolution,
    P1080,
    P1440,
}

/// Which video encoder `OpenClip` may use. Every choice keeps recording working on any GPU.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoderPreference {
    /// Hardware encoding on NVIDIA, Intel, AMD and other GPUs, falling back to the CPU if the GPU
    /// offers none.
    #[default]
    Auto,
    /// Hardware encoding only. Recording reports an error instead of using the CPU.
    Hardware,
    /// CPU encoding only. Slower and heavier on the game, but works on any machine.
    Software,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentPreference {
    #[default]
    Mint,
    Blue,
    Purple,
    Neutral,
}

/// Corner for the post-capture overlay. Bottom corners are deliberately unavailable because they
/// cover CS2's health, armor and ammo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    #[default]
    TopLeft,
    TopRight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// Independent capture choices are intentionally represented as toggles in settings and in the UI.
#[allow(clippy::struct_excessive_bools)]
pub struct Settings {
    pub quality: RecordingQuality,
    pub resolution: RecordingResolution,
    pub frames_per_second: u16,
    /// Which encoder records the game. See [`VideoEncoderPreference`].
    #[serde(default)]
    pub video_encoder: VideoEncoderPreference,
    pub replay_buffer_seconds: u16,
    pub pre_roll_seconds: u16,
    pub post_roll_seconds: u16,
    pub capture_kills: bool,
    pub capture_headshots: bool,
    pub capture_multi_kills: bool,
    /// How long after a kill another kill still joins the same multi-kill, in seconds. It is
    /// independent of the post-roll, which only decides how much footage follows the last kill.
    #[serde(default = "default_multi_kill_window")]
    pub multi_kill_window_seconds: u16,
    pub capture_game_audio: bool,
    pub capture_microphone: bool,
    #[serde(default)]
    pub microphone_device_id: Option<String>,
    pub storage_path: PathBuf,
    #[serde(default)]
    pub accent: AccentPreference,
    #[serde(default = "default_true")]
    pub show_clip_notifications: bool,
    #[serde(default = "default_manual_replay_shortcut")]
    pub manual_replay_shortcut: String,
    #[serde(default)]
    pub watermark_enabled: bool,
    #[serde(default)]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_true")]
    pub overlay_clip_preview: bool,
    #[serde(default = "default_true")]
    pub sounds_enabled: bool,
    /// Notification sound loudness, 0 to 100.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: u8,
    /// Download and install new versions in the background.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Closing the window keeps `OpenClip` recording from the tray.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: RecordingQuality::High,
            resolution: RecordingResolution::GameResolution,
            frames_per_second: 60,
            video_encoder: VideoEncoderPreference::Auto,
            replay_buffer_seconds: 45,
            pre_roll_seconds: 10,
            post_roll_seconds: 5,
            capture_kills: true,
            capture_headshots: true,
            capture_multi_kills: true,
            multi_kill_window_seconds: default_multi_kill_window(),
            capture_game_audio: true,
            capture_microphone: false,
            microphone_device_id: None,
            storage_path: PathBuf::from("OpenClip/Clips"),
            accent: AccentPreference::Mint,
            show_clip_notifications: true,
            manual_replay_shortcut: default_manual_replay_shortcut(),
            watermark_enabled: false,
            overlay_position: OverlayPosition::TopLeft,
            overlay_clip_preview: true,
            sounds_enabled: true,
            sound_volume: default_sound_volume(),
            auto_update: true,
            close_to_tray: true,
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_multi_kill_window() -> u16 {
    8
}

const fn default_sound_volume() -> u8 {
    60
}

fn default_manual_replay_shortcut() -> String {
    "F8".to_owned()
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn older_settings_without_microphone_device_remain_compatible() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "quality":"high",
                "resolution":"game_resolution",
                "framesPerSecond":60,
                "replayBufferSeconds":45,
                "preRollSeconds":10,
                "postRollSeconds":5,
                "captureKills":true,
                "captureHeadshots":true,
                "captureMultiKills":true,
                "captureGameAudio":true,
                "captureMicrophone":false,
                "storagePath":"OpenClip/Clips"
            }"#,
        )
        .expect("legacy settings should deserialize");

        assert_eq!(settings.microphone_device_id, None);
        assert_eq!(settings.manual_replay_shortcut, "F8");
        assert!(!settings.watermark_enabled);
        assert_eq!(settings.overlay_position, super::OverlayPosition::TopLeft);
        assert!(settings.overlay_clip_preview);
        assert!(settings.sounds_enabled);
        assert_eq!(settings.sound_volume, 60);
        assert_eq!(settings.multi_kill_window_seconds, 8);
        assert_eq!(settings.video_encoder, super::VideoEncoderPreference::Auto);
        assert!(settings.auto_update);
        assert!(settings.close_to_tray);
    }
}
