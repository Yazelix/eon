//! The runtime references of an ELF file: its interpreter (`PT_INTERP`)
//! and its library search path (`DT_RUNPATH`, `DT_RPATH`). Only these
//! headers are read, never a raw byte scan, so debug-info paths never count.

use std::path::Path;

use crate::Result;

const PT_LOAD: u64 = 1;
const PT_DYNAMIC: u64 = 2;
const PT_INTERP: u64 = 3;
const DT_STRTAB: u64 = 5;
const DT_RPATH: u64 = 15;
const DT_RUNPATH: u64 = 29;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refs {
    pub interpreter: Option<String>,
    /// Every `DT_RUNPATH` and `DT_RPATH` entry, in order.
    pub runpath: Vec<String>,
}

pub fn read(path: &Path) -> Result<Refs> {
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&bytes).map_err(|err| format!("{}: {err}", path.display()))
}

/// Parses a little-endian 64-bit ELF file.
pub fn parse(bytes: &[u8]) -> Result<Refs> {
    let field = |offset: u64, size: usize| -> Result<u64> {
        let start = usize::try_from(offset).map_err(|err| err.to_string())?;
        let slice = bytes
            .get(start..start.checked_add(size).ok_or("ELF offset overflows")?)
            .ok_or("truncated ELF file")?;
        Ok(slice
            .iter()
            .rev()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte)))
    };
    let text = |offset: u64| -> Result<String> {
        let start = usize::try_from(offset).map_err(|err| err.to_string())?;
        let rest = bytes.get(start..).ok_or("truncated ELF string")?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(rest.len());
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    if bytes.get(..6) != Some(b"\x7fELF\x02\x01".as_slice()) {
        return Err("not a little-endian 64-bit ELF file".into());
    }
    let (phoff, phentsize, phnum) = (field(0x20, 8)?, field(0x36, 2)?, field(0x38, 2)?);
    let mut refs = Refs::default();
    let mut loads = Vec::new();
    let mut dynamic = None;
    for index in 0..phnum {
        let header = phoff + index * phentsize;
        let (kind, offset) = (field(header, 4)?, field(header + 8, 8)?);
        match kind {
            PT_INTERP => refs.interpreter = Some(text(offset)?),
            PT_LOAD => loads.push((field(header + 16, 8)?, offset, field(header + 32, 8)?)),
            PT_DYNAMIC => dynamic = Some((offset, field(header + 32, 8)?)),
            _ => {}
        }
    }
    let Some((offset, size)) = dynamic else {
        return Ok(refs);
    };
    let (mut strtab, mut paths) = (None, Vec::new());
    for entry in 0..size / 16 {
        let (tag, value) = (
            field(offset + entry * 16, 8)?,
            field(offset + entry * 16 + 8, 8)?,
        );
        match tag {
            0 => break,
            DT_STRTAB => strtab = Some(value),
            DT_RPATH | DT_RUNPATH => paths.push(value),
            _ => {}
        }
    }
    if paths.is_empty() {
        return Ok(refs);
    }
    // DT_STRTAB is an address: the PT_LOAD segment holding it gives its offset.
    let address = strtab.ok_or("DT_RUNPATH without DT_STRTAB")?;
    let table = loads
        .iter()
        .find(|(start, _, length)| (*start..start + length).contains(&address))
        .map(|(start, file, _)| address - start + file)
        .ok_or("DT_STRTAB lies in no PT_LOAD segment")?;
    for path in paths {
        refs.runpath.extend(
            text(table + path)?
                .split(':')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned),
        );
    }
    Ok(refs)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal ELF64 file: PT_INTERP (if any), one PT_LOAD over the whole
    /// file at address 0x400000, and PT_DYNAMIC with the given runpaths.
    pub(crate) fn elf(interpreter: Option<&str>, runpath: &[(u64, &str)]) -> Vec<u8> {
        let headers = 3u64;
        let mut bytes = vec![0u8; 0x40 + 0x38 * headers as usize];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
        bytes[0x36..0x38].copy_from_slice(&0x38u16.to_le_bytes());
        bytes[0x38..0x3a].copy_from_slice(&(headers as u16).to_le_bytes());
        let put =
            |bytes: &mut Vec<u8>, index: usize, kind: u32, offset: u64, vaddr: u64, size: u64| {
                let header = 0x40 + 0x38 * index;
                bytes[header..header + 4].copy_from_slice(&kind.to_le_bytes());
                bytes[header + 8..header + 16].copy_from_slice(&offset.to_le_bytes());
                bytes[header + 16..header + 24].copy_from_slice(&vaddr.to_le_bytes());
                bytes[header + 32..header + 40].copy_from_slice(&size.to_le_bytes());
            };
        if let Some(interpreter) = interpreter {
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(format!("{interpreter}\0").as_bytes());
            put(&mut bytes, 0, 3, offset, 0, interpreter.len() as u64 + 1);
        }
        let strtab = bytes.len() as u64;
        let mut entries: Vec<(u64, u64)> = vec![(DT_STRTAB, 0x400000 + strtab)];
        let mut strings = vec![0u8];
        for (tag, text) in runpath {
            entries.push((*tag, strings.len() as u64));
            strings.extend_from_slice(text.as_bytes());
            strings.push(0);
        }
        bytes.extend_from_slice(&strings);
        let dynamic = bytes.len() as u64;
        entries.push((0, 0));
        for (tag, value) in &entries {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let length = bytes.len() as u64;
        put(&mut bytes, 1, 1, 0, 0x400000, length);
        put(&mut bytes, 2, 2, dynamic, 0, entries.len() as u64 * 16);
        bytes
    }

    #[test]
    fn interpreter_and_runpaths_are_read() {
        let bytes = elf(
            Some("/nix/store/a-glibc/lib/ld-linux-x86-64.so.2"),
            &[
                (
                    DT_RUNPATH,
                    "/nix/store/a-glibc/lib:/nix/store/b-gcc-lib/lib",
                ),
                (DT_RPATH, "$ORIGIN/../lib"),
            ],
        );
        assert_eq!(
            parse(&bytes).unwrap(),
            Refs {
                interpreter: Some("/nix/store/a-glibc/lib/ld-linux-x86-64.so.2".into()),
                runpath: vec![
                    "/nix/store/a-glibc/lib".into(),
                    "/nix/store/b-gcc-lib/lib".into(),
                    "$ORIGIN/../lib".into(),
                ],
            }
        );
        assert_eq!(parse(&elf(None, &[])).unwrap(), Refs::default());
        assert!(parse(b"#!/bin/sh\n").is_err());
        assert!(parse(&bytes[..0x50]).is_err());
    }
}
