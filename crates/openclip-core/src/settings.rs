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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// Independent capture choices are intentionally represented as toggles in settings and in the UI.
#[allow(clippy::struct_excessive_bools)]
pub struct Settings {
    pub quality: RecordingQuality,
    pub resolution: RecordingResolution,
    pub frames_per_second: u16,
    pub replay_buffer_seconds: u16,
    pub pre_roll_seconds: u16,
    pub post_roll_seconds: u16,
    pub capture_kills: bool,
    pub capture_headshots: bool,
    pub capture_multi_kills: bool,
    pub capture_game_audio: bool,
    pub capture_microphone: bool,
    pub storage_path: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: RecordingQuality::High,
            resolution: RecordingResolution::GameResolution,
            frames_per_second: 60,
            replay_buffer_seconds: 45,
            pre_roll_seconds: 10,
            post_roll_seconds: 5,
            capture_kills: true,
            capture_headshots: true,
            capture_multi_kills: true,
            capture_game_audio: true,
            capture_microphone: false,
            storage_path: PathBuf::from("OpenClip/Clips"),
        }
    }
}
