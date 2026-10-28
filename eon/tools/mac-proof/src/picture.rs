//! Screen captures as pixels: `sips` turns `screencapture`'s PNG into a small
//! uncompressed BMP, which these checks read without an image library.

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

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= TOLERANCE)
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
