use std::path::{Path, PathBuf};

use mp4io::{CodecConfig, InputSample, Mp4, SampleEntry, TrackParams, Writer, WriterConfig};
use openclip_core::{OpenClipError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedMediaInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

/// Parses a closed MP4 and verifies that its first video sample is independently decodable.
pub fn validate_finalized_segment(path: &Path) -> Result<FinalizedMediaInfo> {
    let bytes = std::fs::read(path).map_err(capture_error)?;
    let mp4 = Mp4::parse(&bytes).map_err(capture_error)?;
    let video = mp4
        .tracks()
        .iter()
        .find(|track| matches!(track.sample_entry(), Some(SampleEntry::Video(_))))
        .ok_or_else(|| OpenClipError::Capture("finalized segment has no video track".to_owned()))?;
    let first = video
        .samples()
        .next()
        .transpose()
        .map_err(capture_error)?
        .ok_or_else(|| {
            OpenClipError::Capture("finalized segment has no video samples".to_owned())
        })?;
    if !first.is_sync {
        return Err(OpenClipError::Capture(
            "finalized segment does not begin with an H.264 sync sample".to_owned(),
        ));
    }
    let Some(SampleEntry::Video(entry)) = video.sample_entry() else {
        unreachable!("video track was selected above");
    };
    let duration_ms = video
        .samples()
        .map(|sample| sample.map(|sample| u64::from(sample.duration)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(capture_error)?
        .into_iter()
        .sum::<u64>()
        .saturating_mul(1_000)
        / u64::from(video.timescale());
    Ok(FinalizedMediaInfo {
        duration_ms,
        width: u32::from(entry.width),
        height: u32::from(entry.height),
        has_audio: mp4.tracks().iter().any(|track| {
            matches!(track.sample_entry(), Some(SampleEntry::Audio(_)))
                && track.samples().next().is_some_and(|sample| sample.is_ok())
        }),
    })
}

/// Remuxes independent MP4 segments into one MP4 without decoding or re-encoding samples.
pub fn remux_segments(paths: &[PathBuf], output: &Path) -> Result<FinalizedMediaInfo> {
    let first_path = paths
        .first()
        .ok_or_else(|| OpenClipError::Unavailable("the replay buffer is empty".to_owned()))?;
    let first_bytes = std::fs::read(first_path).map_err(capture_error)?;
    let first = Mp4::parse(&first_bytes).map_err(capture_error)?;
    let mut writer = Writer::new(WriterConfig::default());
    let mut mapping = Vec::new();
    for (index, track) in first.tracks().iter().enumerate() {
        let Some(params) = track_params(track)? else {
            continue;
        };
        mapping.push((index, writer.add_track(params)));
    }
    if mapping.is_empty() {
        return Err(OpenClipError::Capture(
            "segment has no supported media tracks".to_owned(),
        ));
    }

    for path in paths {
        let bytes = std::fs::read(path).map_err(capture_error)?;
        let mp4 = Mp4::parse(&bytes).map_err(capture_error)?;
        if mp4.tracks().len() != first.tracks().len() {
            return Err(OpenClipError::Capture(format!(
                "segment track layout changed at {}",
                path.display()
            )));
        }
        for (index, writer_track) in &mapping {
            let track = &mp4.tracks()[*index];
            if track.timescale() != first.tracks()[*index].timescale()
                || sample_entry_fingerprint(track.sample_entry())
                    != sample_entry_fingerprint(first.tracks()[*index].sample_entry())
            {
                return Err(OpenClipError::Capture(format!(
                    "segment codec configuration changed at {}",
                    path.display()
                )));
            }
            for sample in track.samples() {
                let sample = sample.map_err(capture_error)?;
                let dts = i64::try_from(sample.dts).map_err(capture_error)?;
                let composition_offset = i32::try_from(sample.cts - dts).map_err(capture_error)?;
                writer
                    .write_sample(
                        *writer_track,
                        InputSample {
                            data: sample.data,
                            duration: sample.duration,
                            composition_offset,
                            is_sync: sample.is_sync,
                        },
                    )
                    .map_err(capture_error)?;
            }
        }
    }
    let bytes = writer.finalize().map_err(capture_error)?;
    let partial = output.with_extension("mp4.partial");
    std::fs::write(&partial, bytes).map_err(capture_error)?;
    let info = validate_finalized_segment(&partial)?;
    std::fs::rename(&partial, output).map_err(capture_error)?;
    Ok(info)
}

fn sample_entry_fingerprint(entry: Option<&SampleEntry>) -> String {
    format!("{entry:?}")
}

fn track_params(track: &mp4io::Track<'_>) -> Result<Option<TrackParams>> {
    let Some(entry) = track.sample_entry() else {
        return Ok(None);
    };
    Ok(match entry {
        SampleEntry::Video(video) => {
            let mut params = TrackParams::video(video.codec, video.width, video.height)
                .timescale(track.timescale());
            params = match &video.config {
                CodecConfig::Avc(config) => params.avc_config(config.raw.clone()),
                CodecConfig::Hevc(config) => params.hevc_config(config.raw.clone()),
                CodecConfig::Av1(config) => params.av1_config(config.raw.clone()),
                _ => params,
            };
            Some(params.edit_list(track.edits().to_vec()))
        }
        SampleEntry::Audio(audio) => {
            let mut params =
                TrackParams::audio(audio.codec, audio.channels, sample_rate(audio.sample_rate)?)
                    .timescale(track.timescale());
            if let CodecConfig::Aac(config) = &audio.config {
                params = params.aac_config(
                    config.object_type,
                    config.sample_rate,
                    config.channels,
                    config.decoder_specific.clone(),
                );
            }
            Some(params)
        }
        _ => None,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_rate(value: f64) -> Result<u32> {
    if !value.is_finite() || value < 1.0 || value > f64::from(u32::MAX) {
        return Err(OpenClipError::Capture(format!(
            "invalid audio sample rate: {value}"
        )));
    }
    Ok(value.round() as u32)
}

fn capture_error(error: impl std::fmt::Display) -> OpenClipError {
    OpenClipError::Capture(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp4io::Codec;

    fn write_test_segment(path: &Path, sync: bool) {
        let mut writer = Writer::new(WriterConfig::default());
        let video =
            writer.add_track(TrackParams::video(Codec::H264, 1_920, 1_080).timescale(1_000));
        writer
            .write_sample(
                video,
                InputSample {
                    data: &[0, 0, 0, 1, 0x65],
                    duration: 1_000,
                    composition_offset: 0,
                    is_sync: sync,
                },
            )
            .unwrap();
        std::fs::write(path, writer.finalize().unwrap()).unwrap();
    }

    #[test]
    fn truncated_in_flight_segment_is_rejected_without_touching_it() {
        let directory = tempfile::tempdir().expect("temp directory");
        let partial = directory.path().join("segment-1.mp4.partial");
        std::fs::write(&partial, b"not a finalized mp4").expect("write fixture");
        assert!(validate_finalized_segment(&partial).is_err());
        assert!(partial.exists());
    }

    #[test]
    fn no_segments_cannot_be_remuxed() {
        let directory = tempfile::tempdir().expect("temp directory");
        assert!(remux_segments(&[], &directory.path().join("clip.mp4")).is_err());
    }

    #[test]
    fn finalized_segments_survive_an_interrupted_neighbor_and_remux() {
        let directory = tempfile::tempdir().expect("temp directory");
        let first = directory.path().join("first.mp4");
        let second = directory.path().join("second.mp4");
        let interrupted = directory.path().join("third.mp4.partial");
        let output = directory.path().join("saved.mp4");
        write_test_segment(&first, true);
        write_test_segment(&second, true);
        std::fs::write(&interrupted, b"abrupt termination").unwrap();

        let info = remux_segments(&[first, second], &output).expect("valid segments remux");
        assert_eq!(info.duration_ms, 2_000);
        assert!(output.exists());
        assert!(interrupted.exists());
        assert!(validate_finalized_segment(&interrupted).is_err());
    }

    #[test]
    fn non_sync_segment_is_not_accepted_for_replay() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("unsafe.mp4");
        write_test_segment(&path, false);
        assert!(validate_finalized_segment(&path).is_err());
    }
}
