//! Screen captures as pixels: `sips` turns `screencapture`'s PNG into a small
//! uncompressed BMP, which these checks read without an image library.

use std::collections::BTreeMap;

/// A capture's pixels as RGB, row by row from the top.
#[derive(Debug)]
pub struct Picture {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

/// Part of a picture, in shares of its width and height from its top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// How far apart two channel values may be and still read as the same color.
const TOLERANCE: u8 = 24;
/// How far a captured blend may lie from the expected one in a channel:
/// the references pass through the same screencapture and sips path, so
/// this covers rounding and downscaling, not colour management.
const BLEND: f64 = 12.0;

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= TOLERANCE)
}

/// The backdrop's share through a window, from its colours over two backdrops.
pub fn backdrop_shares(
    p_light: [u8; 3],
    p_dark: [u8; 3],
    b_light: [u8; 3],
    b_dark: [u8; 3],
) -> Result<[f64; 3], String> {
    if b_light
        .iter()
        .zip(b_dark)
        .any(|(light, dark)| light.abs_diff(dark) < 64)
    {
        return Err(format!(
            "P_light {p_light:?}, P_dark {p_dark:?}, B_light {b_light:?}, B_dark {b_dark:?}; \
             backdrops must differ by at least 64 in every channel"
        ));
    }
    Ok(std::array::from_fn(|channel| {
        (f64::from(p_light[channel]) - f64::from(p_dark[channel]))
            / (f64::from(b_light[channel]) - f64::from(b_dark[channel]))
    }))
}

/// Whether every channel's backdrop share is within 0.05 of `expected`.
pub fn check_shares(shares: [f64; 3], expected: f64) -> Result<(), String> {
    if shares.iter().all(|share| (share - expected).abs() <= 0.05) {
        Ok(())
    } else {
        Err(format!(
            "backdrop shares {shares:?} must each be within 0.05 of {expected}; \
             straight-vs-premultiplied or linear-light blending may show this way"
        ))
    }
}

/// The expected colour over a backdrop at the configured background opacity.
pub fn blend(background: [u8; 3], backdrop: [u8; 3], opacity: f64) -> [f64; 3] {
    std::array::from_fn(|channel| {
        opacity * f64::from(background[channel]) + (1.0 - opacity) * f64::from(backdrop[channel])
    })
}

/// Whether a captured colour matches the expected blend in every channel.
pub fn check_blend(picture: [u8; 3], expected: [f64; 3]) -> Result<(), String> {
    if picture
        .iter()
        .zip(expected)
        .all(|(value, expected)| (f64::from(*value) - expected).abs() <= BLEND)
    {
        Ok(())
    } else {
        Err(format!(
            "P {picture:?} must be within {BLEND} of expected {expected:?} in every channel; \
             straight-vs-premultiplied or linear-light blending may show this way"
        ))
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

impl Picture {
    /// Reads an uncompressed 24- or 32-bit BMP (BI_RGB, or BI_BITFIELDS with
    /// BGRA masks), bottom-up or top-down.
    pub fn from_bmp(bytes: &[u8]) -> Result<Picture, String> {
        if bytes.get(..2) != Some(b"BM") {
            return Err("not a BMP".to_owned());
        }
        let short = || "BMP header cut short".to_owned();
        let offset = u32_at(bytes, 10).ok_or_else(short)? as usize;
        let width = u32_at(bytes, 18).ok_or_else(short)? as i32;
        let height = u32_at(bytes, 22).ok_or_else(short)? as i32;
        let bits = u16_at(bytes, 28).ok_or_else(short)?;
        let compression = u32_at(bytes, 30).ok_or_else(short)?;
        let bgra = compression == 3
            && [54, 58, 62].map(|at| u32_at(bytes, at)) == [0xff0000, 0xff00, 0xff].map(Some);
        if !(bits == 24 && compression == 0 || bits == 32 && (compression == 0 || bgra)) {
            return Err(format!(
                "BMP of {bits} bits with compression {compression}; want uncompressed 24 or 32"
            ));
        }
        if width <= 0 || height == 0 || height == i32::MIN {
            return Err(format!("BMP of {width}x{height} pixels"));
        }
        let (width, rows) = (width as usize, height.unsigned_abs() as usize);
        let step = usize::from(bits / 8);
        let stride = (width * step).div_ceil(4) * 4;
        let data = bytes
            .get(offset..)
            .filter(|data| data.len() >= stride * rows)
            .ok_or("BMP pixels cut short")?;
        let mut pixels = Vec::with_capacity(width * rows);
        for row in 0..rows {
            let stored = if height > 0 { rows - 1 - row } else { row };
            let line = &data[stored * stride..][..width * step];
            pixels.extend(line.chunks(step).map(|bgr| [bgr[2], bgr[1], bgr[0]]));
        }
        Ok(Picture {
            width,
            height: rows,
            pixels,
        })
    }

    /// The pixels inside `region`, which must overlap the picture.
    pub fn crop(&self, region: Region) -> Result<Picture, String> {
        let span = |start: f64, length: f64, size: usize| {
            let size_f = size as f64;
            let first = (start * size_f).floor().clamp(0.0, size_f) as usize;
            let end = ((start + length) * size_f).ceil().clamp(0.0, size_f) as usize;
            (first, end)
        };
        let (left, right) = span(region.x, region.width, self.width);
        let (top, bottom) = span(region.y, region.height, self.height);
        if left >= right || top >= bottom {
            return Err(format!("{region:?} lies outside the main display"));
        }
        let pixels = (top..bottom)
            .flat_map(|row| &self.pixels[row * self.width + left..row * self.width + right])
            .copied()
            .collect();
        Ok(Picture {
            width: right - left,
            height: bottom - top,
            pixels,
        })
    }

    /// Whether every pixel is one color, as a capture of nothing is.
    pub fn blank(&self) -> bool {
        self.pixels.iter().all(|pixel| near(*pixel, self.pixels[0]))
    }

    /// The most common exact colour and its share; ties take the greater RGB.
    pub fn dominant(&self) -> ([u8; 3], f64) {
        let mut counts = BTreeMap::new();
        for pixel in &self.pixels {
            *counts.entry(*pixel).or_insert(0usize) += 1;
        }
        let (colour, count) = counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .expect("pixels");
        (colour, count as f64 / self.pixels.len() as f64)
    }

    /// The share of pixels that differ from `other`'s, or 1 when the sizes do.
    pub fn changed(&self, other: &Picture) -> f64 {
        if (self.width, self.height) != (other.width, other.height) {
            return 1.0;
        }
        let changed = self
            .pixels
            .iter()
            .zip(&other.pixels)
            .filter(|(a, b)| !near(**a, **b))
            .count();
        changed as f64 / self.pixels.len() as f64
    }

    /// The share of pixels near pure magenta, the color of the scenario's image.
    pub fn magenta(&self) -> f64 {
        let magenta = self
            .pixels
            .iter()
            .filter(|[r, g, b]| *r >= 180 && *g <= 90 && *b >= 180)
            .count();
        magenta as f64 / self.pixels.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMP of `rows` (top row first), 24-bit bottom-up or 32-bit top-down.
    fn bmp(rows: &[Vec<[u8; 3]>], bits: u16) -> Vec<u8> {
        let (width, height) = (rows[0].len(), rows.len());
        let step = usize::from(bits / 8);
        let stride = (width * step).div_ceil(4) * 4;
        let mut data = Vec::new();
        let order: Vec<&Vec<[u8; 3]>> = if bits == 24 {
            rows.iter().rev().collect()
        } else {
            rows.iter().collect()
        };
        for row in order {
            let mut line: Vec<u8> = row
                .iter()
                .flat_map(|[r, g, b]| [*b, *g, *r, 255].into_iter().take(step))
                .collect();
            line.resize(stride, 0);
            data.extend(line);
        }
        let signed = if bits == 24 {
            height as i32
        } else {
            -(height as i32)
        };
        let mut bytes = b"BM".to_vec();
        for value in [54 + data.len() as u32, 0, 54, 40, width as u32] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(signed.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(bits.to_le_bytes());
        bytes.extend([0u8; 24]);
        bytes.extend(data);
        bytes
    }

    #[test]
    fn dominant_colour_counts_exact_pixels_and_breaks_ties_consistently() {
        let picture = Picture::from_bmp(&bmp(
            &[vec![[9, 8, 7], [1, 2, 3], [9, 8, 7], [9, 8, 6]]],
            24,
        ))
        .unwrap();
        assert_eq!(picture.dominant(), ([9, 8, 7], 0.5));
        for pixels in [vec![[1, 2, 3], [9, 8, 7]], vec![[9, 8, 7], [1, 2, 3]]] {
            let picture = Picture::from_bmp(&bmp(&[pixels], 32)).unwrap();
            assert_eq!(picture.dominant(), ([9, 8, 7], 0.5));
        }
    }

    #[test]
    fn backdrop_shares_accept_translucency_and_reject_wrong_blending() {
        let (light, dark) = ([224; 3], [32; 3]);
        let shares = backdrop_shares([53, 61, 69], [14, 22, 30], light, dark).unwrap();
        assert!(shares.iter().all(|share| (share - 0.2).abs() < 0.005));
        assert!(check_shares(shares, 0.2).is_ok());
        for (lit, unlit) in [([10; 3], [10; 3]), ([108; 3], [16; 3])] {
            let shares = backdrop_shares(lit, unlit, light, dark).unwrap();
            assert!(check_shares(shares, 0.2).is_err(), "{shares:?}");
        }
        assert!(check_shares([0.2, 0.251, 0.2], 0.2).is_err());
        assert!(check_shares([0.2, 0.149, 0.2], 0.2).is_err());
        assert!(check_shares([0.151, 0.249, 0.2], 0.2).is_ok());
        assert!(check_shares([f64::NAN, 0.2, 0.2], 0.2).is_err());
        assert!(backdrop_shares([50; 3], [12; 3], [224, 95, 224], dark).is_err());
        assert!(backdrop_shares([50; 3], [12; 3], [96; 3], dark).is_ok());
    }

    #[test]
    fn blend_check_rejects_wrong_colours_and_compositing() {
        // CI run 40828543315: OSC 11 left the background black.
        for (picture, backdrop) in [([39; 3], [224; 3]), ([6; 3], [32; 3])] {
            assert!(check_blend(picture, blend([0; 3], backdrop, 0.8)).is_ok());
            assert!(check_blend(picture, blend([128; 3], backdrop, 0.8)).is_err());
        }
        let expected = [[224; 3], [32; 3]].map(|backdrop| blend([128; 3], backdrop, 0.8));
        for (picture, expected) in [[147; 3], [109; 3]].into_iter().zip(expected) {
            assert!(check_blend(picture, expected).is_ok());
        }
        for pictures in [
            [[173; 3], [134; 3]],            // Straight RGB supplied as premultiplied.
            [[128; 3], [128; 3]],            // Opaque background.
            [[249, 45, 249], [210, 6, 210]], // Magenta instead of grey.
        ] {
            for (picture, expected) in pictures.into_iter().zip(expected) {
                assert!(
                    check_blend(picture, expected).is_err(),
                    "{picture:?} over {expected:?}"
                );
            }
        }
    }

    #[test]
    fn captures_read_in_both_layouts_and_the_checks_tell_them_apart() {
        let (dark, magenta) = ([20, 20, 30], [250, 10, 245]);
        // Three pixels wide, so 24-bit rows carry padding.
        let desktop = vec![vec![dark; 3]; 4];
        let mut window = desktop.clone();
        window[0] = vec![[200, 200, 200], dark, magenta];
        for bits in [24, 32] {
            let read = Picture::from_bmp(&bmp(&window, bits)).unwrap();
            assert_eq!((read.width, read.height), (3, 4));
            assert_eq!(read.pixels[..3], window[0][..], "{bits}-bit top row");
            assert_eq!(read.pixels[3..], vec![dark; 9][..]);
            let empty = Picture::from_bmp(&bmp(&desktop, bits)).unwrap();
            assert!(empty.blank() && !read.blank());
            assert_eq!(read.changed(&empty), 2.0 / 12.0);
            assert_eq!(read.magenta(), 1.0 / 12.0);
        }
        let read = Picture::from_bmp(&bmp(&window, 32)).unwrap();
        let corner = Region {
            x: 0.5,
            y: -0.5,
            width: 2.0,
            height: 0.75,
        };
        assert_eq!(read.crop(corner).unwrap().pixels, [dark, magenta]);
        let beside = Region { x: 1.0, ..corner };
        assert!(read.crop(beside).is_err());
        let mut cut = bmp(&window, 24);
        cut.truncate(cut.len() - 1);
        assert_eq!(Picture::from_bmp(&cut).unwrap_err(), "BMP pixels cut short");
        assert!(Picture::from_bmp(b"\x89PNG").is_err());
    }
}
