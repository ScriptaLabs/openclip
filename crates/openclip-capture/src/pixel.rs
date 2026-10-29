//! Pixel format conversion for encoders that take system-memory NV12 input.

/// Source and destination geometry of one conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub source_width: usize,
    pub source_height: usize,
    /// Bytes per source row, which may exceed `source_width * 4`.
    pub source_pitch: usize,
    pub width: usize,
    pub height: usize,
}

impl Geometry {
    /// Bytes in one NV12 frame of the destination size.
    pub(crate) const fn nv12_len(&self) -> usize {
        self.width * self.height + self.width * (self.height / 2)
    }
}

/// Bands below this many rows are not worth a thread.
const MIN_ROWS_PER_BAND: usize = 128;
const MAX_BANDS: usize = 4;

/// Converts a BGRA frame to BT.709 limited-range NV12, resizing with nearest-neighbour sampling
/// when the destination differs. `width` and `height` must be even and non-zero. Large frames are
/// split into horizontal bands converted on a few short-lived threads, because this runs on the
/// capture callback and has to fit inside a 60 FPS frame budget.
pub(crate) fn bgra_to_nv12(source: &[u8], geometry: Geometry, output: &mut Vec<u8>) {
    let Geometry {
        source_width,
        source_height,
        source_pitch,
        width,
        height,
    } = geometry;
    debug_assert!(width > 0 && height > 0 && width % 2 == 0 && height % 2 == 0);
    debug_assert!(source.len() >= source_pitch * source_height);
    output.clear();
    output.resize(geometry.nv12_len(), 0);
    let (luma, chroma) = output.split_at_mut(width * height);
    let columns: Vec<usize> = (0..width).map(|x| x * source_width / width * 4).collect();
    let rows: Vec<usize> = (0..height)
        .map(|y| y * source_height / height * source_pitch)
        .collect();

    let pairs = height / 2;
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let bands = (pairs * 2 / MIN_ROWS_PER_BAND).clamp(1, threads.min(MAX_BANDS));
    let pairs_per_band = pairs.div_ceil(bands);
    let convert_band = |first_pair: usize, luma: &mut [u8], chroma: &mut [u8]| {
        for (offset, (luma_rows, chroma_row)) in luma
            .chunks_exact_mut(width * 2)
            .zip(chroma.chunks_exact_mut(width))
            .enumerate()
        {
            let pair = first_pair + offset;
            convert_row_pair(
                source,
                [rows[pair * 2], rows[pair * 2 + 1]],
                &columns,
                luma_rows,
                chroma_row,
            );
        }
    };
    if bands == 1 {
        convert_band(0, luma, chroma);
        return;
    }
    std::thread::scope(|scope| {
        let bands_of = luma
            .chunks_mut(width * 2 * pairs_per_band)
            .zip(chroma.chunks_mut(width * pairs_per_band));
        for (index, (luma_band, chroma_band)) in bands_of.enumerate() {
            let convert_band = &convert_band;
            scope.spawn(move || convert_band(index * pairs_per_band, luma_band, chroma_band));
        }
    });
}

/// Two luma rows and the chroma row they share.
fn convert_row_pair(
    source: &[u8],
    source_rows: [usize; 2],
    columns: &[usize],
    luma_rows: &mut [u8],
    chroma_row: &mut [u8],
) {
    let width = columns.len();
    let (top, bottom) = luma_rows.split_at_mut(width);
    for (x, chroma) in chroma_row.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let (mut r, mut g, mut b) = (0, 0, 0);
        for (dy, row) in [(0, source_rows[0]), (1, source_rows[1])] {
            for dx in 0..2 {
                let at = row + columns[x * 2 + dx];
                let pixel = &source[at..at + 4];
                let (pr, pg, pb) = (
                    i32::from(pixel[2]),
                    i32::from(pixel[1]),
                    i32::from(pixel[0]),
                );
                let luma = clamp_byte(((47 * pr + 157 * pg + 16 * pb + 128) >> 8) + 16);
                if dy == 0 {
                    top[x * 2 + dx] = luma;
                } else {
                    bottom[x * 2 + dx] = luma;
                }
                r += pr;
                g += pg;
                b += pb;
            }
        }
        let (r, g, b) = ((r + 2) / 4, (g + 2) / 4, (b + 2) / 4);
        chroma[0] = clamp_byte(((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128);
        chroma[1] = clamp_byte(((112 * r - 102 * g - 10 * b + 128) >> 8) + 128);
    }
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
const fn clamp_byte(value: i32) -> u8 {
    if value < 0 {
        0
    } else if value > 255 {
        255
    } else {
        value as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, bgra: [u8; 4], pitch_padding: usize) -> (Vec<u8>, usize) {
        let pitch = width * 4 + pitch_padding;
        let mut data = vec![0; pitch * height];
        for row in 0..height {
            for column in 0..width {
                let at = row * pitch + column * 4;
                data[at..at + 4].copy_from_slice(&bgra);
            }
        }
        (data, pitch)
    }

    fn convert(bgra: [u8; 4]) -> Vec<u8> {
        let (data, pitch) = solid(4, 4, bgra, 0);
        let mut out = Vec::new();
        bgra_to_nv12(
            &data,
            Geometry {
                source_width: 4,
                source_height: 4,
                source_pitch: pitch,
                width: 4,
                height: 4,
            },
            &mut out,
        );
        out
    }

    #[test]
    fn white_and_black_use_the_limited_range() {
        let white = convert([255, 255, 255, 255]);
        assert!(white[..16].iter().all(|&y| y == 235));
        assert!(white[16..].iter().all(|&c| (127..=129).contains(&c)));
        let black = convert([0, 0, 0, 255]);
        assert!(black[..16].iter().all(|&y| y == 16));
    }

    #[test]
    fn banded_conversion_matches_a_single_pass() {
        // 512 rows is large enough to be split across threads.
        let (width, height) = (64, 512);
        let mut source = vec![0_u8; width * height * 4];
        for (index, byte) in source.iter_mut().enumerate() {
            *byte = u8::try_from(index * 31 % 251).unwrap();
        }
        let geometry = Geometry {
            source_width: width,
            source_height: height,
            source_pitch: width * 4,
            width,
            height,
        };
        let mut banded = Vec::new();
        bgra_to_nv12(&source, geometry, &mut banded);
        let columns: Vec<usize> = (0..width).map(|x| x * 4).collect();
        let mut single = vec![0_u8; geometry.nv12_len()];
        let (luma, chroma) = single.split_at_mut(width * height);
        for pair in 0..height / 2 {
            convert_row_pair(
                &source,
                [pair * 2 * width * 4, (pair * 2 + 1) * width * 4],
                &columns,
                &mut luma[pair * 2 * width..(pair * 2 + 2) * width],
                &mut chroma[pair * width..(pair + 1) * width],
            );
        }
        assert_eq!(banded, single);
    }

    #[test]
    fn red_has_high_cr_and_low_cb() {
        let red = convert([0, 0, 255, 255]);
        assert!(red[16] < 128, "cb");
        assert!(red[17] > 200, "cr");
    }

    #[test]
    fn row_padding_and_scaling_are_respected() {
        let (data, pitch) = solid(8, 8, [255, 255, 255, 255], 16);
        let mut out = Vec::new();
        let geometry = Geometry {
            source_width: 8,
            source_height: 8,
            source_pitch: pitch,
            width: 4,
            height: 4,
        };
        bgra_to_nv12(&data, geometry, &mut out);
        assert_eq!(out.len(), geometry.nv12_len());
        assert_eq!(out.len(), 24);
        assert!(out[..16].iter().all(|&y| y == 235));
    }
}
