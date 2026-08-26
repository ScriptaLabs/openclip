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
        for sample in track.samples() {
            let sample = sample?;
            count = count.saturating_add(1);
            duration = duration.saturating_add(u64::from(sample.duration));
            sync = sync.saturating_add(u64::from(sample.is_sync));
        }
        println!(
            "track={index} codec={:?} timescale={} samples={count} duration_ticks={duration} sync_samples={sync}",
            track.sample_entry().map(mp4io::SampleEntry::codec),
            track.timescale(),
        );
    }
    Ok(())
}
