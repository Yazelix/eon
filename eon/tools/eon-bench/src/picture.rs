//! RGB captures from grim or a losslessly decoded PNG.

use std::{fs, path::Path};

type Result<T> = std::result::Result<T, String>;

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}
pub(crate) struct Picture {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<u8>,
}
impl Picture {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        Self::parse(&fs::read(path).map_err(failure)?)
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let mut position = 0;
        let mut token = || -> Result<&[u8]> {
            while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
                position += 1;
            }
            let begin = position;
            while bytes
                .get(position)
                .is_some_and(|b| !b.is_ascii_whitespace())
            {
                position += 1;
            }
            if position == begin {
                return Err("short PPM header".into());
            }
            Ok(&bytes[begin..position])
        };
        if token()? != b"P6" {
            return Err("capture is not P6 PPM".into());
        }
        let width = std::str::from_utf8(token()?)
            .map_err(failure)?
            .parse::<usize>()
            .map_err(failure)?;
        let height = std::str::from_utf8(token()?)
            .map_err(failure)?
            .parse::<usize>()
            .map_err(failure)?;
        if token()? != b"255" {
            return Err("capture depth is not 255".into());
        }
        if !bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            return Err("bad PPM separator".into());
        }
        position += 1;
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || bytes.len() - position != width * height * 3
        {
            return Err("invalid PPM dimensions/length".into());
        }
        Ok(Self {
            width,
            height,
            pixels: bytes[position..].to_vec(),
        })
    }
    pub(crate) fn pixel(&self, x: usize, y: usize) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let begin = (y * self.width + x) * 3;
        Some(&self.pixels[begin..begin + 3])
    }
    pub(crate) fn crop(&self, rect: Rect) -> Result<Vec<u8>> {
        if rect.w == 0
            || rect.h == 0
            || u64::from(rect.x) + u64::from(rect.w) > self.width as u64
            || u64::from(rect.y) + u64::from(rect.h) > self.height as u64
        {
            return Err("oracle region is outside the capture".into());
        }
        Ok((rect.y..rect.y + rect.h)
            .flat_map(|y| {
                let start = (y as usize * self.width + rect.x as usize) * 3;
                self.pixels[start..start + rect.w as usize * 3]
                    .iter()
                    .copied()
            })
            .collect())
    }
}
