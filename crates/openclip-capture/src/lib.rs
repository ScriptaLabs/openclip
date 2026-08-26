//! Platform-neutral contracts for the future Windows capture pipeline.
//!
//! There is intentionally no capture implementation in Milestone 1.

// Concrete encoders will document their hardware- and driver-specific failure modes.
#![allow(clippy::missing_errors_doc)]

use std::time::Duration;

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
