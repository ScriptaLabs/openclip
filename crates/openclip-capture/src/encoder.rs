//! Picks the H.264 encoder that records the game.
//!
//! Order for `Auto`: NVIDIA's direct NVENC path (lowest overhead, no CPU readback), then the GPU
//! vendor's Media Foundation encoder (Intel, AMD and others), then Microsoft's CPU encoder so a
//! machine without any hardware encoder still records.

use openclip_core::VideoEncoderPreference;
use windows_capture::frame::Frame;

use crate::config::EncoderConfig;
use crate::h264::{BackendError, VideoSegment};
use crate::mf_backend::{Kind, MfEncoder};
use crate::nvenc_backend::DirectNvencEncoder;

pub(crate) enum SegmentEncoder {
    Nvenc(Box<DirectNvencEncoder>),
    MediaFoundation(Box<MfEncoder>),
}

/// What was chosen, for the status line and for the clips it records.
#[derive(Debug, Clone)]
pub(crate) struct EncoderInfo {
    pub label: String,
    pub hardware: bool,
}

impl SegmentEncoder {
    pub(crate) fn select(
        frame: &Frame<'_>,
        config: EncoderConfig,
        preference: VideoEncoderPreference,
    ) -> Result<(Self, EncoderInfo), BackendError> {
        let mut failures: Vec<String> = Vec::new();
        if preference != VideoEncoderPreference::Software {
            if nvenc_present() {
                match DirectNvencEncoder::new(frame, config) {
                    Ok(encoder) => {
                        return Ok((
                            Self::Nvenc(Box::new(encoder)),
                            EncoderInfo {
                                label: "NVIDIA NVENC".to_owned(),
                                hardware: true,
                            },
                        ));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "NVENC unavailable; trying other encoders");
                        failures.push(format!("NVENC: {error}"));
                    }
                }
            }
            match MfEncoder::new(config, Kind::Hardware) {
                Ok(encoder) => return Ok(Self::media_foundation(encoder, true)),
                Err(error) => {
                    tracing::warn!(%error, "no Media Foundation hardware encoder");
                    failures.push(format!("hardware encoder: {error}"));
                }
            }
        }
        if preference != VideoEncoderPreference::Hardware {
            match MfEncoder::new(config, Kind::Software) {
                Ok(encoder) => {
                    if !failures.is_empty() || preference == VideoEncoderPreference::Auto {
                        tracing::warn!("recording with the CPU encoder");
                    }
                    return Ok(Self::media_foundation(encoder, false));
                }
                Err(error) => failures.push(format!("software encoder: {error}")),
            }
        }
        Err(format!("no H.264 encoder could start ({})", failures.join("; ")).into())
    }

    fn media_foundation(encoder: MfEncoder, hardware: bool) -> (Self, EncoderInfo) {
        let label = if hardware {
            encoder.name().to_owned()
        } else {
            format!("{} (CPU)", encoder.name())
        };
        (
            Self::MediaFoundation(Box::new(encoder)),
            EncoderInfo { label, hardware },
        )
    }

    pub(crate) fn encode_frame(
        &mut self,
        frame: &Frame<'_>,
        copies: u32,
    ) -> Result<Option<VideoSegment>, BackendError> {
        match self {
            Self::Nvenc(encoder) => encoder.encode_frame(frame, copies),
            Self::MediaFoundation(encoder) => encoder.encode_frame(frame, copies),
        }
    }

    pub(crate) fn finish_segment(&mut self) -> Result<Option<VideoSegment>, BackendError> {
        match self {
            Self::Nvenc(encoder) => encoder.finish_segment(),
            Self::MediaFoundation(encoder) => encoder.finish_segment(),
        }
    }
}

fn nvenc_present() -> bool {
    std::env::var_os("WINDIR").is_some_and(|windows| {
        std::path::PathBuf::from(windows)
            .join("System32")
            .join("nvEncodeAPI64.dll")
            .is_file()
    })
}
