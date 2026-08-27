use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};

use openclip_core::{OpenClipError, Result};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HGDIOBJ, ReleaseDC,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
};
use windows::core::HSTRING;

const THUMBNAIL_WIDTH: i32 = 384;
const THUMBNAIL_HEIGHT: i32 = 216;

#[must_use]
pub fn thumbnail_path_for_clip(clip: &Path) -> PathBuf {
    let mut name = clip
        .file_stem()
        .map_or_else(|| "clip".into(), std::ffi::OsStr::to_os_string);
    name.push(".thumbnail.bmp");
    clip.with_file_name(name)
}

/// Uses Windows' registered video thumbnail provider, keeping media decoding out of the UI.
pub fn generate_clip_thumbnail(clip: &Path) -> Result<PathBuf> {
    let clip = clip.to_path_buf();
    std::thread::Builder::new()
        .name("openclip-thumbnail".to_owned())
        .spawn(move || generate_clip_thumbnail_sta(&clip))
        .map_err(thumbnail_error)?
        .join()
        .map_err(|_| thumbnail_error("Windows thumbnail worker panicked"))?
}

fn generate_clip_thumbnail_sta(clip: &Path) -> Result<PathBuf> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = generate_clip_thumbnail_inner(clip);
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

fn generate_clip_thumbnail_inner(clip: &Path) -> Result<PathBuf> {
    let source = HSTRING::from(shell_parsing_path(clip));
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&source, None) }.map_err(thumbnail_error)?;
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: THUMBNAIL_WIDTH,
                cy: THUMBNAIL_HEIGHT,
            },
            SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
        )
    }
    .map_err(thumbnail_error)?;

    let output = thumbnail_path_for_clip(clip);
    let write_result = (|| {
        let mut native = BITMAP::default();
        let object_size = i32::try_from(std::mem::size_of::<BITMAP>()).map_err(thumbnail_error)?;
        let copied = unsafe {
            GetObjectW(
                HGDIOBJ(bitmap.0),
                object_size,
                Some((&raw mut native).cast::<c_void>()),
            )
        };
        if copied == 0 || native.bmWidth <= 0 || native.bmHeight <= 0 {
            return Err(thumbnail_error(
                "Windows returned an invalid thumbnail bitmap",
            ));
        }

        let width = u32::try_from(native.bmWidth).map_err(thumbnail_error)?;
        let height = u32::try_from(native.bmHeight).map_err(thumbnail_error)?;
        let stride = width.saturating_mul(4);
        let image_size = stride.saturating_mul(height);
        let mut pixels = vec![0_u8; usize::try_from(image_size).map_err(thumbnail_error)?];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>())
                    .map_err(thumbnail_error)?,
                biWidth: native.bmWidth,
                biHeight: -native.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: image_size,
                ..Default::default()
            },
            ..Default::default()
        };
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return Err(thumbnail_error(
                "could not acquire a Windows display context",
            ));
        }
        let scan_lines = unsafe {
            GetDIBits(
                screen,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr().cast::<c_void>()),
                &raw mut info,
                DIB_RGB_COLORS,
            )
        };
        unsafe { ReleaseDC(None, screen) };
        if scan_lines == 0 {
            return Err(thumbnail_error(
                "could not read the Windows thumbnail bitmap",
            ));
        }

        let header_size = 14_u32 + 40;
        let mut file = std::fs::File::create(&output).map_err(thumbnail_error)?;
        file.write_all(b"BM").map_err(thumbnail_error)?;
        file.write_all(&(header_size + image_size).to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&[0; 4]).map_err(thumbnail_error)?;
        file.write_all(&header_size.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&40_u32.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&native.bmWidth.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&(-native.bmHeight).to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&1_u16.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&32_u16.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&0_u32.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&image_size.to_le_bytes())
            .map_err(thumbnail_error)?;
        file.write_all(&[0; 16]).map_err(thumbnail_error)?;
        file.write_all(&pixels).map_err(thumbnail_error)?;
        Ok(output.clone())
    })();
    let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
    write_result
}

fn shell_parsing_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
}

fn thumbnail_error(error: impl std::fmt::Display) -> OpenClipError {
    OpenClipError::Capture(format!("thumbnail generation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_path_is_adjacent_and_deterministic() {
        assert_eq!(
            thumbnail_path_for_clip(Path::new(r"C:\Clips\ace.mp4")),
            PathBuf::from(r"C:\Clips\ace.thumbnail.bmp")
        );
    }

    #[test]
    fn shell_path_removes_the_extended_length_prefix() {
        assert_eq!(
            shell_parsing_path(Path::new(r"\\?\C:\Clips\ace.mp4")),
            r"C:\Clips\ace.mp4"
        );
    }
}
