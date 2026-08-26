use openclip_core::{RecordingQuality, RecordingResolution, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u16,
    pub bitrate: u32,
    pub capture_game_audio: bool,
}

#[must_use]
pub fn encoder_config(settings: &Settings, source_width: u32, source_height: u32) -> EncoderConfig {
    let (width, height) = match settings.resolution {
        RecordingResolution::GameResolution => (source_width, source_height),
        RecordingResolution::P1080 => (1_920, 1_080),
        RecordingResolution::P1440 => (2_560, 1_440),
    };
    let base_bitrate = match settings.quality {
        RecordingQuality::Performance => 12_000_000_u64,
        RecordingQuality::High => 24_000_000,
        RecordingQuality::Ultra => 40_000_000,
    };
    let pixel_scale = u64::from(width)
        .saturating_mul(u64::from(height))
        .max(1)
        .saturating_mul(1_000)
        / (1_920 * 1_080);
    let bitrate = u32::try_from(
        base_bitrate
            .saturating_mul(pixel_scale)
            .saturating_div(1_000)
            .clamp(8_000_000, u64::from(u32::MAX)),
    )
    .unwrap_or(u32::MAX);
    EncoderConfig {
        width,
        height,
        frames_per_second: settings.frames_per_second.clamp(1, 60),
        bitrate,
        capture_game_audio: settings.capture_game_audio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_bounded_h264_configuration() {
        let settings = Settings {
            resolution: RecordingResolution::P1080,
            frames_per_second: 144,
            quality: RecordingQuality::High,
            ..Settings::default()
        };
        let config = encoder_config(&settings, 3_840, 2_160);
        assert_eq!((config.width, config.height), (1_920, 1_080));
        assert_eq!(config.frames_per_second, 60);
        assert_eq!(config.bitrate, 24_000_000);
    }

    #[test]
    fn native_resolution_scales_the_bitrate() {
        let config = encoder_config(&Settings::default(), 2_560, 1_440);
        assert_eq!((config.width, config.height), (2_560, 1_440));
        assert!(config.bitrate > 24_000_000);
    }
}
