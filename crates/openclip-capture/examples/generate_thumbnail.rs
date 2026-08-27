use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let clip = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: generate_thumbnail <clip.mp4>")?;
    let thumbnail = openclip_capture::generate_clip_thumbnail(&clip)?;
    println!("{}", thumbnail.display());
    Ok(())
}
