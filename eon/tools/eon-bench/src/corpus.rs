//! The benchmark corpora: terminal output generated from fixed seeds, the same
//! bytes on every machine and run, each pinned by its recorded SHA-256.
//!
//! A corpus is whole units (a line, or one cursor operation) generated until
//! it reaches its target size, so no file ends inside an escape sequence or a
//! UTF-8 character. Lines end in CRLF, so they draw the same when a parser
//! reads the file directly and through a PTY whose ONLCR turns each CRLF into
//! CR CR LF; the bytes a PTY delivers are therefore not the file's bytes.
//! Changing a generator, seed or size changes a recorded hash; such a change
//! bumps `CORPUS_VERSION` and records the new hashes.

use std::{fs, io::Write, path::Path};

use crate::sha256::Sha256;

/// The version of the corpus set; results name it beside each corpus ID.
pub const CORPUS_VERSION: u32 = 1;

const MIB: usize = 1 << 20;

pub struct Corpus {
    pub id: &'static str,
    pub summary: &'static str,
    seed: u64,
    target: usize,
    unit: fn(&mut Rng, &mut Vec<u8>),
    /// The recorded length in bytes and SHA-256 of the generated corpus.
    pub length: u64,
    pub sha256: &'static str,
}

pub const CORPORA: [Corpus; 6] = [
    Corpus {
        id: "ascii",
        summary: "printable ASCII lines of 0 to 119 characters",
        seed: 0x6173_6369_6900_0001,
        target: 16 * MIB,
        unit: ascii_line,
        length: 16_777_289,
        sha256: "6ebf5259297fd841eaf4471f48ca6885e33ad705acb20f005728a0af7c9bcae8",
    },
    Corpus {
        id: "styled-light",
        summary: "ASCII lines with occasional 16-color SGR runs",
        seed: 0x7374_796c_6564_0001,
        target: 16 * MIB,
        unit: styled_light_line,
        length: 16_777_261,
        sha256: "82948e79a908f7ec2da0518449c866683855c051ef0939e5d85b0865cc19e6fe",
    },
    Corpus {
        id: "styled-heavy",
        summary: "an SGR change every 1 to 4 cells: attributes, 256-color and truecolor",
        seed: 0x7374_796c_6564_0002,
        target: 16 * MIB,
        unit: styled_heavy_line,
        length: 16_777_631,
        sha256: "4a77f87caf1b01b4c01b070ef7db1f44105d67e76e72476a45ba749cf169d3e6",
    },
    Corpus {
        id: "unicode",
        summary: "CJK wide characters, combining marks and emoji graphemes among ASCII",
        seed: 0x756e_6963_6f64_0001,
        target: 16 * MIB,
        unit: unicode_line,
        length: 16_777_337,
        sha256: "804f0e92e9fe395ce9c6c243b790640e16f699348d681376dca1139af9cb58dc",
    },
    Corpus {
        id: "cursor",
        summary: "TUI redraw on an 80x24 grid: cursor motion, erase, save and restore",
        seed: 0x6375_7273_6f72_0001,
        target: 16 * MIB,
        unit: cursor_operation,
        length: 16_777_226,
        sha256: "3a42c3d61ccfd1166570c54536c6da36a3ca9f17042ac706fa007e544d116c07",
    },
    Corpus {
        id: "cat64",
        summary: "64 MiB of the ascii generator, the \"cat a big file\" workload",
        seed: 0x6361_7436_3400_0001,
        target: 64 * MIB,
        unit: ascii_line,
        length: 67_108_884,
        sha256: "5d93c4f2579ed7332d9e9d452e816bdbb5fe7f534cd8b73e60027c6196c7010d",
    },
];

/// Adds the corpus `id` to `only` once, for `--only`.
pub fn add(only: &mut Vec<&'static Corpus>, id: &str) -> Result<(), String> {
    let corpus = find(id).ok_or(format!(
        "no corpus {id}; `eon-bench corpus --help` lists them"
    ))?;
    if !only.iter().any(|known| known.id == corpus.id) {
        only.push(corpus);
    }
    Ok(())
}

pub fn find(id: &str) -> Option<&'static Corpus> {
    CORPORA.iter().find(|corpus| corpus.id == id)
}

impl Corpus {
    pub fn file_name(&self) -> String {
        format!("{}.vt", self.id)
    }

    /// Writes the corpus to `out` and returns its length and SHA-256.
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<(u64, String)> {
        let mut rng = Rng(self.seed);
        let mut hasher = Sha256::new();
        let mut buffer = Vec::with_capacity(64 * 1024 + 4096);
        let mut length = 0;
        while length < self.target {
            let before = buffer.len();
            (self.unit)(&mut rng, &mut buffer);
            length += buffer.len() - before;
            if buffer.len() >= 64 * 1024 || length >= self.target {
                hasher.update(&buffer);
                out.write_all(&buffer)?;
                buffer.clear();
            }
        }
        out.flush()?;
        Ok((length as u64, hasher.hex()))
    }
}

/// Writes each corpus to `<dir>/<id>.vt`, failing on the first one whose
/// bytes differ from the recorded length or SHA-256, then `<dir>/SHA256SUMS`
/// with the recorded hashes, which `sha256sum -c` checks. An earlier
/// SHA256SUMS goes first, so a failed run never leaves one behind.
pub fn write_files(dir: &Path, corpora: &[&Corpus]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let sums_path = dir.join("SHA256SUMS");
    match fs::remove_file(&sums_path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", sums_path.display()));
        }
        _ => {}
    }
    let mut sums = String::new();
    for corpus in corpora {
        let path = dir.join(corpus.file_name());
        let failure = |error: std::io::Error| format!("{}: {error}", path.display());
        let mut file = fs::File::create(&path).map_err(failure)?;
        let (length, sha256) = corpus.write(&mut file).map_err(failure)?;
        if (length, sha256.as_str()) != (corpus.length, corpus.sha256) {
            return Err(format!(
                "{}: generated {length} bytes with SHA-256 {sha256}; recorded {} bytes with {}",
                path.display(),
                corpus.length,
                corpus.sha256
            ));
        }
        sums.push_str(&format!("{}  {}\n", corpus.sha256, corpus.file_name()));
    }
    fs::write(&sums_path, sums).map_err(|error| format!("{}: {error}", sums_path.display()))
}

/// SplitMix64: small, fast, and fixed by its published constants.
pub struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    fn one_in(&mut self, odds: u64) -> bool {
        self.below(odds) == 0
    }

    fn printable(&mut self) -> u8 {
        b' ' + self.below(95) as u8
    }
}

fn ascii_line(rng: &mut Rng, out: &mut Vec<u8>) {
    for _ in 0..rng.below(120) {
        out.push(rng.printable());
    }
    out.extend_from_slice(b"\r\n");
}

fn styled_light_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let mut cells = rng.below(120);
    while cells > 0 {
        if rng.one_in(12) {
            let run = rng.range(3, 12).min(cells);
            let base = if rng.one_in(2) { 30 } else { 90 };
            write!(out, "\x1b[{}m", base + rng.range(1, 7)).unwrap();
            for _ in 0..run {
                out.push(rng.printable());
            }
            out.extend_from_slice(b"\x1b[39m");
            cells -= run;
        } else {
            out.push(rng.printable());
            cells -= 1;
        }
    }
    out.extend_from_slice(b"\x1b[0m\r\n");
}

fn styled_heavy_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let mut cells = rng.below(120);
    while cells > 0 {
        out.extend_from_slice(b"\x1b[0");
        for attribute in [1, 3, 4, 7] {
            if rng.one_in(4) {
                write!(out, ";{attribute}").unwrap();
            }
        }
        color(rng, out, 30);
        if rng.one_in(2) {
            color(rng, out, 40);
        }
        out.push(b'm');
        let run = rng.range(1, 4).min(cells);
        for _ in 0..run {
            out.push(rng.printable());
        }
        cells -= run;
    }
    out.extend_from_slice(b"\x1b[0m\r\n");
}

/// One SGR color parameter, foreground at base 30 or background at base 40:
/// a 16-color, 256-color or truecolor form.
fn color(rng: &mut Rng, out: &mut Vec<u8>, base: u64) {
    match rng.below(3) {
        0 if rng.one_in(2) => write!(out, ";{}", base + rng.below(8)).unwrap(),
        0 => write!(out, ";{}", base + 60 + rng.below(8)).unwrap(),
        1 => write!(out, ";{};5;{}", base + 8, rng.below(256)).unwrap(),
        _ => write!(
            out,
            ";{};2;{};{};{}",
            base + 8,
            rng.below(256),
            rng.below(256),
            rng.below(256)
        )
        .unwrap(),
    }
}

const SKIN_TONE_BASES: [u32; 7] = [
    0x1f44b, 0x1f44c, 0x1f44d, 0x1f44e, 0x1f44f, 0x1f64f, 0x1f9d1,
];
const ZWJ_SEQUENCES: [&str; 4] = [
    "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
    "\u{1f9d1}\u{200d}\u{1f4bb}",
    "\u{1f3f3}\u{fe0f}\u{200d}\u{1f308}",
    "\u{1f469}\u{200d}\u{1f680}",
];
const FLAGS: [&str; 8] = ["BR", "JP", "DE", "US", "FR", "IN", "KR", "NG"];

/// Grapheme clusters until the line reaches a random width under 80 cells,
/// counting CJK and emoji as two cells; the last cluster can carry it to about
/// 88, so some lines wrap on an 80-column grid.
fn unicode_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let limit = rng.below(80);
    let mut width = 0;
    while width < limit {
        match rng.below(7) {
            0 => {
                for _ in 0..rng.range(1, 8) {
                    out.push(b'a' + rng.below(26) as u8);
                    width += 1;
                }
                out.push(b' ');
                width += 1;
            }
            1 => {
                push(out, 0x4e00 + rng.below(0x5000) as u32);
                width += 2;
            }
            2 => {
                out.push(b'a' + rng.below(26) as u8);
                for _ in 0..rng.range(1, 3) {
                    push(out, 0x300 + rng.below(0x70) as u32);
                }
                width += 1;
            }
            3 => {
                push(out, 0x1f600 + rng.below(0x50) as u32);
                width += 2;
            }
            4 => {
                push(out, SKIN_TONE_BASES[rng.below(7) as usize]);
                push(out, 0x1f3fb + rng.below(5) as u32);
                width += 2;
            }
            5 => {
                out.extend_from_slice(ZWJ_SEQUENCES[rng.below(4) as usize].as_bytes());
                width += 2;
            }
            _ => {
                for letter in FLAGS[rng.below(8) as usize].bytes() {
                    push(out, 0x1f1e6 + u32::from(letter - b'A'));
                }
                width += 2;
            }
        }
    }
    out.extend_from_slice(b"\r\n");
}

fn push(out: &mut Vec<u8>, code: u32) {
    let mut utf8 = [0; 4];
    out.extend_from_slice(
        char::from_u32(code)
            .unwrap()
            .encode_utf8(&mut utf8)
            .as_bytes(),
    );
}

/// One redraw step of a full-screen program on an 80x24 grid.
fn cursor_operation(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(10) {
        0..=4 => write!(out, "\x1b[{};{}H", rng.range(1, 24), rng.range(1, 80)).unwrap(),
        5..=6 => {
            let direction = b"ABCD"[rng.below(4) as usize] as char;
            write!(out, "\x1b[{}{direction}", rng.range(1, 10)).unwrap();
        }
        7 => out.extend_from_slice(b"\x1b7"),
        8 => out.extend_from_slice(b"\x1b8"),
        _ => {
            out.extend_from_slice(b"\x1b[24;1H\x1b[7m");
            for _ in 0..rng.range(10, 79) {
                out.push(rng.printable());
            }
            out.extend_from_slice(b"\x1b[0m\x1b[K");
            return;
        }
    }
    for _ in 0..rng.range(1, 20) {
        out.push(rng.printable());
    }
    if rng.one_in(3) {
        out.extend_from_slice(b"\x1b[K");
    }
    if rng.one_in(50) {
        out.extend_from_slice(b"\x1b[J");
    }
    if rng.one_in(500) {
        out.extend_from_slice(b"\x1b[H\x1b[2J");
    }
}

#[cfg(test)]
mod tests {
    use super::{CORPORA, Corpus, write_files};

    fn generate(corpus: &Corpus) -> Vec<u8> {
        let mut bytes = Vec::new();
        let (length, sha256) = corpus.write(&mut bytes).unwrap();
        assert_eq!(length, bytes.len() as u64);
        assert_eq!(
            (length, sha256.as_str()),
            (corpus.length, corpus.sha256),
            "{} differs from its recorded length and SHA-256",
            corpus.id
        );
        bytes
    }

    /// Every corpus generates the same bytes twice, matching its recorded
    /// length and SHA-256, and holds only what its ID promises.
    #[test]
    fn corpora_are_byte_identical_and_match_their_recorded_hashes() {
        for corpus in &CORPORA {
            let bytes = generate(corpus);
            assert!(
                bytes == generate(corpus),
                "{} changed between runs",
                corpus.id
            );
            check_contents(corpus.id, &bytes);
        }
    }

    fn check_contents(id: &str, bytes: &[u8]) {
        let text = std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{id} is not UTF-8"));
        if id == "cursor" {
            assert!(!text.contains('\n'), "cursor holds a newline");
        } else {
            assert!(text.ends_with("\r\n"), "{id} ends inside a line");
            assert!(
                !text.replace("\r\n", "").contains(['\r', '\n']),
                "{id}: bare CR or LF"
            );
        }
        let ascii = matches!(id, "ascii" | "cat64");
        let mut rest = text;
        while let Some(start) = rest.find(['\x1b']) {
            assert!(!ascii, "{id} holds an escape");
            let sequence = &rest[start..];
            let end = if let Some(csi) = sequence.strip_prefix("\x1b[") {
                let final_byte = csi
                    .find(|c: char| !c.is_ascii_digit() && c != ';')
                    .unwrap_or_else(|| panic!("{id} ends inside an escape"));
                assert!(
                    "mHABCDJK".contains(&csi[final_byte..final_byte + 1]),
                    "{id}: unexpected CSI {:?}",
                    &sequence[..final_byte + 3]
                );
                final_byte + 3
            } else {
                assert!(matches!(&sequence[1..2], "7" | "8"), "{id}: unexpected ESC");
                2
            };
            rest = &sequence[end..];
        }
        if ascii {
            assert!(
                text.bytes()
                    .all(|b| (b' '..=b'~').contains(&b) || b == b'\r' || b == b'\n')
            );
        }
        if id == "unicode" {
            assert!(text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
            assert!(text.contains('\u{200d}') && text.contains('\u{301}'));
        }
    }

    /// The file writer stores the bytes the generator produced and a
    /// SHA256SUMS line in `sha256sum -c` format.
    #[test]
    fn write_files_stores_the_corpus_and_its_checksum() {
        let dir = std::env::temp_dir().join(format!("eon-bench-corpus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let corpus = super::find("styled-light").unwrap();
        write_files(&dir, &[corpus]).unwrap();
        assert!(std::fs::read(dir.join("styled-light.vt")).unwrap() == generate(corpus));
        assert_eq!(
            std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap(),
            format!("{}  styled-light.vt\n", corpus.sha256)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
