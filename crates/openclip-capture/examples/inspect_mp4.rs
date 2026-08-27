use std::collections::BTreeMap;
use std::path::PathBuf;

use mp4io::Mp4;
use openclip_capture::validate_finalized_segment;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p openclip-capture --example inspect_mp4 -- <path>")?;
    let info = validate_finalized_segment(&path)?;
    let bytes = std::fs::read(&path)?;
    let mp4 = Mp4::parse(&bytes)?;
    println!(
        "path={} duration_ms={} size={}x{} has_audio={}",
        path.display(),
        info.duration_ms,
        info.width,
        info.height,
        info.has_audio
    );
    for (index, track) in mp4.tracks().iter().enumerate() {
        let mut count = 0_u64;
        let mut duration = 0_u64;
        let mut sync = 0_u64;
        let mut durations = BTreeMap::<u32, u64>::new();
        let mut first_nal_types = Vec::new();
        for sample in track.samples() {
            let sample = sample?;
            if count == 0 && matches!(track.sample_entry(), Some(mp4io::SampleEntry::Video(_))) {
                let mut cursor = 0;
                while cursor + 4 <= sample.data.len() {
                    let length =
                        u32::from_be_bytes(sample.data[cursor..cursor + 4].try_into()?) as usize;
                    cursor += 4;
                    if cursor + length > sample.data.len() || length == 0 {
                        break;
                    }
                    first_nal_types.push(sample.data[cursor] & 0x1f);
                    cursor += length;
                }
            }
            count = count.saturating_add(1);
            duration = duration.saturating_add(u64::from(sample.duration));
            sync = sync.saturating_add(u64::from(sample.is_sync));
            *durations.entry(sample.duration).or_default() += 1;
        }
        println!(
            "track={index} codec={:?} timescale={} samples={count} duration_ticks={duration} sync_samples={sync}",
            track.sample_entry().map(mp4io::SampleEntry::codec),
            track.timescale(),
        );
        println!("  sample_duration_histogram={durations:?}");
        if !first_nal_types.is_empty() {
            println!("  first_sample_h264_nal_types={first_nal_types:?}");
        }
    }
    Ok(())
}
