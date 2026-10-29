//! H.264 bitstream helpers shared by the encoder backends.

use mp4io::{Codec, InputSample, TrackParams, Writer, WriterConfig};

const MP4_VIDEO_TIMESCALE: u32 = 90_000;

pub(crate) type BackendError = Box<dyn std::error::Error + Send + Sync>;

/// A single independently playable H.264 segment produced by one continuous encoder session.
pub(crate) struct VideoSegment {
    pub bytes: Vec<u8>,
    pub frame_count: u32,
}

pub(crate) fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let prefix = if data[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if data[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, prefix));
        index += prefix;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, (start, prefix))| {
            let body_start = start + prefix;
            let body_end = starts.get(position + 1).map_or(data.len(), |next| next.0);
            (body_start < body_end).then_some(&data[body_start..body_end])
        })
        .collect()
}

pub(crate) fn avcc_from_annex_b(data: &[u8]) -> Result<Vec<u8>, BackendError> {
    let nals = annex_b_nals(data);
    let sps = nals
        .iter()
        .copied()
        .find(|nal| nal.first().is_some_and(|byte| byte & 0x1f == 7))
        .ok_or("the first IDR picture omitted the SPS")?;
    let pps = nals
        .iter()
        .copied()
        .find(|nal| nal.first().is_some_and(|byte| byte & 0x1f == 8))
        .ok_or("the first IDR picture omitted the PPS")?;
    if sps.len() < 4 || sps.len() > usize::from(u16::MAX) || pps.len() > usize::from(u16::MAX) {
        return Err("invalid H.264 parameter set".into());
    }
    let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    avcc.extend_from_slice(&u16::try_from(sps.len())?.to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&u16::try_from(pps.len())?.to_be_bytes());
    avcc.extend_from_slice(pps);
    Ok(avcc)
}

pub(crate) fn annex_b_to_length_prefixed(data: &[u8]) -> Result<Vec<u8>, BackendError> {
    let mut output = Vec::with_capacity(data.len());
    for nal in annex_b_nals(data) {
        output.extend_from_slice(&u32::try_from(nal.len())?.to_be_bytes());
        output.extend_from_slice(nal);
    }
    if output.is_empty() {
        return Err("the encoder returned an empty or non-Annex-B packet".into());
    }
    Ok(output)
}

/// Muxes one GOP of length-prefixed samples into an independently playable MP4 segment. The first
/// sample must be the IDR picture.
pub(crate) fn assemble_segment(
    avcc: &[u8],
    (width, height): (u32, u32),
    frames_per_second: u16,
    frames: &[Vec<u8>],
) -> Result<VideoSegment, BackendError> {
    let width = u16::try_from(width).map_err(|_| "encoder width exceeds the MP4 field limit")?;
    let height = u16::try_from(height).map_err(|_| "encoder height exceeds the MP4 field limit")?;
    let mut writer = Writer::new(WriterConfig::default());
    let track = writer.add_track(
        TrackParams::video(Codec::H264, width, height)
            .timescale(MP4_VIDEO_TIMESCALE)
            .avc_config(avcc.to_vec()),
    );
    let frame_duration = MP4_VIDEO_TIMESCALE / u32::from(frames_per_second.max(1));
    for (index, bytes) in frames.iter().enumerate() {
        writer.write_sample(
            track,
            InputSample {
                data: bytes,
                duration: frame_duration,
                composition_offset: 0,
                is_sync: index == 0,
            },
        )?;
    }
    Ok(VideoSegment {
        bytes: writer.finalize()?,
        frame_count: u32::try_from(frames.len()).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_avcc_and_length_prefixed_samples_from_annex_b() {
        let packet = [
            0, 0, 0, 1, 0x67, 0x64, 0, 0x2a, 1, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80, 0, 0, 1, 0x65,
            0xaa,
        ];
        let avcc = avcc_from_annex_b(&packet).unwrap();
        assert_eq!(&avcc[..6], &[1, 0x64, 0, 0x2a, 0xff, 0xe1]);
        let sample = annex_b_to_length_prefixed(&packet).unwrap();
        assert_eq!(&sample[..4], &5_u32.to_be_bytes());
        assert!(sample.ends_with(&[0, 0, 0, 2, 0x65, 0xaa]));
    }

    #[test]
    fn assembles_a_segment_with_one_sync_sample() {
        let packet = [
            0, 0, 0, 1, 0x67, 0x64, 0, 0x2a, 1, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80, 0, 0, 1, 0x65,
            0xaa,
        ];
        let avcc = avcc_from_annex_b(&packet).unwrap();
        let sample = annex_b_to_length_prefixed(&packet).unwrap();
        let segment = assemble_segment(&avcc, (64, 64), 30, &[sample.clone(), sample]).unwrap();
        assert_eq!(segment.frame_count, 2);
        assert!(segment.bytes.windows(4).any(|window| window == b"ftyp"));
    }
}
