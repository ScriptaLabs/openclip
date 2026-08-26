//! Windows capture, H.264 encoding, and manual replay finalization.

// Concrete encoders will document their hardware- and driver-specific failure modes.
#![allow(clippy::missing_errors_doc)]

use std::time::Duration;

use serde::Serialize;

mod config;
mod media;
mod recorder;
#[cfg(windows)]
mod windows;
pub use config::{EncoderConfig, encoder_config};
pub use media::{FinalizedMediaInfo, remux_segments, validate_finalized_segment};
pub use recorder::RecorderService;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// Enumerates active Windows microphone endpoints for persistent user preference selection.
pub fn list_microphone_devices() -> openclip_core::Result<Vec<AudioDevice>> {
    #[cfg(windows)]
    return windows::list_microphone_devices();
    #[cfg(not(windows))]
    Ok(Vec::new())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareEncoder {
    NvidiaNvenc,
    AmdAmf,
    IntelQuickSync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    pub timestamp: Duration,
    pub is_keyframe: bool,
    pub bytes: Vec<u8>,
}

pub trait VideoEncoder: Send {
    fn kind(&self) -> HardwareEncoder;
    fn encode_next(&mut self) -> openclip_core::Result<Option<EncodedPacket>>;
    fn flush(&mut self) -> openclip_core::Result<Vec<EncodedPacket>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTrackKind {
    Game,
    Microphone,
    Communications,
}

pub trait AudioSource: Send {
    fn kind(&self) -> AudioTrackKind;
}
