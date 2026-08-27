use std::path::PathBuf;

use openclip_capture::remux_segments;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1).map(PathBuf::from);
    let output = arguments
        .next()
        .ok_or("usage: remux_segments <output.mp4> <segment.mp4>...")?;
    let segments = arguments.collect::<Vec<_>>();
    let info = remux_segments(&segments, &output)?;
    println!(
        "output={} segments={} duration_ms={} size={}x{} has_audio={}",
        output.display(),
        segments.len(),
        info.duration_ms,
        info.width,
        info.height,
        info.has_audio
    );
    Ok(())
}
