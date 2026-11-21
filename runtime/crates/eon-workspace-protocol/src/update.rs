//! EONU, the Update-only control format (EON-C11 planned Update control),
//! version 1 exactly as approved by Lucca on 2026-11-21 (eon-t3fyd.7).
//!
//! It is independent of the workspace protocol: this module uses only `std`,
//! and an EONW-only change never changes it. Every frame is a 12-byte header
//! (`EONU`, the u16 version, kind, flags 0, the u32 payload length) and an
//! exactly sized payload. The first 6 bytes never change: a reader decides on
//! the version from them alone and answers another version without reading
//! further. Each Update slice codes the frames it uses; this module has the
//! header, every kind's number, direction and bound, and the INSPECT,
//! INSPECTED and REFUSED payloads.

/// The frozen magic.
pub const MAGIC: [u8; 4] = *b"EONU";
/// The one version this build reads and writes.
pub const VERSION: u16 = 1;
/// Magic and version: the part of every frame no version changes.
pub const PREFIX_BYTES: usize = 6;
pub const HEADER_BYTES: usize = 12;
/// Retained terminals one INSPECTED lists.
pub const MAX_HOSTS: usize = 256;
/// Terminal, run and caller identifiers.
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_DETAIL_BYTES: usize = 1024;
/// Request IDs are 16 bytes.
pub type RequestId = [u8; 16];

const KIB: usize = 1024;

/// Every v1 kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Inspect = 1,
    Preflight = 2,
    Commit = 3,
    TargetCheck = 64,
    Carry = 65,
    Inspected = 129,
    Refused = 130,
    Preflighted = 131,
    Updated = 132,
    TargetVerdict = 192,
}

/// Where a frame travels; each kind travels one way only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Client to window, on `update.sock`.
    Request,
    /// Window to client, on `update.sock`.
    Reply,
    /// Window to the target's validation child, on its stdin.
    TargetCheck,
    /// Validation child to window, on its stdout.
    TargetReply,
    /// Old image to the same process after exec.
    Carry,
}

impl Kind {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Inspect,
            2 => Self::Preflight,
            3 => Self::Commit,
            64 => Self::TargetCheck,
            65 => Self::Carry,
            129 => Self::Inspected,
            130 => Self::Refused,
            131 => Self::Preflighted,
            132 => Self::Updated,
            192 => Self::TargetVerdict,
            _ => return None,
        })
    }

    /// The kind's payload bound.
    pub fn bound(self) -> usize {
        match self {
            Self::Inspect => 16,
            Self::Preflight | Self::Commit | Self::Carry => 8 * KIB,
            Self::TargetCheck => 16 * KIB * KIB + 128 * KIB,
            Self::Inspected => 128 * KIB,
            Self::Refused => 2 * KIB,
            Self::Preflighted | Self::Updated | Self::TargetVerdict => KIB,
        }
    }

    /// Whether the kind may travel `direction`. REFUSED answers both a
    /// client and a window.
    pub fn travels(self, direction: Direction) -> bool {
        match self {
            Self::Inspect | Self::Preflight | Self::Commit => direction == Direction::Request,
            Self::Inspected | Self::Preflighted | Self::Updated => direction == Direction::Reply,
            Self::Refused => matches!(direction, Direction::Reply | Direction::TargetReply),
            Self::TargetCheck => direction == Direction::TargetCheck,
            Self::TargetVerdict => direction == Direction::TargetReply,
            Self::Carry => direction == Direction::Carry,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The first 4 bytes are not `EONU`.
    Magic,
    /// Another EONU version, which this build never decodes.
    Version(u16),
    /// A bad header or field; names what.
    Malformed(&'static str),
    /// A declared length over the kind's bound.
    Oversized,
    /// Fewer bytes than the frame needs.
    Truncated,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Magic => f.write_str("not an EONU frame"),
            Self::Version(version) => write!(f, "EONU version {version}, not {VERSION}"),
            Self::Malformed(what) => write!(f, "malformed EONU {what}"),
            Self::Oversized => f.write_str("EONU frame over its bound"),
            Self::Truncated => f.write_str("truncated EONU frame"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The version a frame's first 6 bytes declare.
pub fn version(prefix: &[u8; PREFIX_BYTES]) -> Result<u16> {
    if prefix[..4] != MAGIC {
        return Err(Error::Magic);
    }
    Ok(u16::from_le_bytes([prefix[4], prefix[5]]))
}

/// A checked v1 header: its kind and payload length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub length: usize,
}

/// Checks a whole header read in `direction`, before any payload is read:
/// prefix, flags, a known kind travelling its own way, then its bound.
pub fn header(bytes: &[u8; HEADER_BYTES], direction: Direction) -> Result<Header> {
    let found = version(bytes[..PREFIX_BYTES].try_into().expect("6 bytes"))?;
    if found != VERSION {
        return Err(Error::Version(found));
    }
    let kind = Kind::from_byte(bytes[6]).ok_or(Error::Malformed("kind"))?;
    if bytes[7] != 0 {
        return Err(Error::Malformed("flags"));
    }
    if !kind.travels(direction) {
        return Err(Error::Malformed("direction"));
    }
    let length = usize::try_from(u32::from_le_bytes(
        bytes[8..12].try_into().expect("4 bytes"),
    ))
    .map_err(|_| Error::Oversized)?;
    if length > kind.bound() {
        return Err(Error::Oversized);
    }
    Ok(Header { kind, length })
}

fn frame(kind: Kind, payload: Vec<u8>) -> Result<Vec<u8>> {
    if payload.len() > kind.bound() {
        return Err(Error::Oversized);
    }
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.push(kind as u8);
    out.push(0);
    out.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(|_| Error::Oversized)?
            .to_le_bytes(),
    );
    out.extend(payload);
    Ok(out)
}

/// The payload of a whole frame of `kind` read in `direction`.
fn payload(frame: &[u8], kind: Kind, direction: Direction) -> Result<&[u8]> {
    let head: &[u8; HEADER_BYTES] = frame
        .get(..HEADER_BYTES)
        .ok_or(Error::Truncated)?
        .try_into()
        .expect("12 bytes");
    let checked = header(head, direction)?;
    if checked.kind != kind {
        return Err(Error::Malformed("kind"));
    }
    let body = &frame[HEADER_BYTES..];
    match body.len().cmp(&checked.length) {
        std::cmp::Ordering::Less => Err(Error::Truncated),
        std::cmp::Ordering::Greater => Err(Error::Malformed("trailing bytes")),
        std::cmp::Ordering::Equal => Ok(body),
    }
}

/// A frame's kind, for a reader that accepts several kinds.
pub fn kind_of(frame: &[u8], direction: Direction) -> Result<Kind> {
    let head: &[u8; HEADER_BYTES] = frame
        .get(..HEADER_BYTES)
        .ok_or(Error::Truncated)?
        .try_into()
        .expect("12 bytes");
    header(head, direction).map(|header| header.kind)
}

/// REFUSED codes; any other value is malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Malformed = 1,
    Version = 2,
    Unsupported = 3,
    Lifetime = 4,
    StaleBuild = 5,
    Busy = 6,
    Unsettled = 7,
    UntrustedTarget = 8,
    UnknownCaller = 9,
    UnknownOrigin = 10,
    StaleCommand = 11,
    TargetRefused = 12,
    HostProtocol = 13,
    Internal = 14,
}

impl Code {
    fn from_byte(byte: u8) -> Result<Self> {
        Ok(match byte {
            1 => Self::Malformed,
            2 => Self::Version,
            3 => Self::Unsupported,
            4 => Self::Lifetime,
            5 => Self::StaleBuild,
            6 => Self::Busy,
            7 => Self::Unsettled,
            8 => Self::UntrustedTarget,
            9 => Self::UnknownCaller,
            10 => Self::UnknownOrigin,
            11 => Self::StaleCommand,
            12 => Self::TargetRefused,
            13 => Self::HostProtocol,
            14 => Self::Internal,
            _ => return Err(Error::Malformed("refusal code")),
        })
    }

    /// The code's name on the approved page.
    pub fn name(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::Version => "version",
            Self::Unsupported => "unsupported",
            Self::Lifetime => "lifetime",
            Self::StaleBuild => "stale-build",
            Self::Busy => "busy",
            Self::Unsettled => "unsettled",
            Self::UntrustedTarget => "untrusted-target",
            Self::UnknownCaller => "unknown-caller",
            Self::UnknownOrigin => "unknown-origin",
            Self::StaleCommand => "stale-command",
            Self::TargetRefused => "target-refused",
            Self::HostProtocol => "host-protocol",
            Self::Internal => "internal",
        }
    }
}

/// INSPECT: what a client sends to identify a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspect {
    pub request: RequestId,
}

/// REFUSED: a reply's refusal, before any effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// All zero when the request could not be read.
    pub request: RequestId,
    pub code: Code,
    pub detail: String,
}

/// INSPECTED's presentation; other values are malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Detached = 0,
    Attached = 1,
}

/// A retained terminal, as the terminal inventory reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub terminal: String,
    pub run: String,
    pub origin: Option<String>,
    pub management: u16,
    pub record_generation: u16,
    pub orbs: Option<u16>,
    pub orbf: Option<u16>,
}

/// The request kinds an outcome can name; other values are malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Preflight = 2,
    Commit = 3,
}

/// The window's last Update outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub request: RequestId,
    pub operation: Operation,
    /// None for success.
    pub refusal: Option<Code>,
    pub detail: String,
}

/// INSPECTED: who the window is, read without any workspace decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspected {
    pub request: RequestId,
    pub pid: u32,
    pub namespace: Vec<u8>,
    pub lifetime: String,
    pub created_build: String,
    pub active_build: String,
    pub active_launcher: Option<Vec<u8>>,
    /// The window's EONW version, as a number only.
    pub eonw: u16,
    pub eonr: u16,
    pub update_schema: u16,
    pub presentation: Presentation,
    pub hosts: Vec<Host>,
    pub last: Option<Outcome>,
}

impl Inspect {
    pub fn encode(&self) -> Result<Vec<u8>> {
        frame(Kind::Inspect, self.request.to_vec())
    }

    pub fn decode(frame: &[u8]) -> Result<Self> {
        let mut input = Reader(payload(frame, Kind::Inspect, Direction::Request)?);
        let request = input.request()?;
        input.finish()?;
        Ok(Self { request })
    }
}

impl Refused {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        out.bytes(&self.request);
        out.u8(self.code as u8);
        out.text(&self.detail, MAX_DETAIL_BYTES)?;
        frame(Kind::Refused, out.0)
    }

    /// A REFUSED read in `direction` (a client's reply or a child's).
    pub fn decode(frame: &[u8], direction: Direction) -> Result<Self> {
        let mut input = Reader(payload(frame, Kind::Refused, direction)?);
        let refused = Self {
            request: input.request()?,
            code: Code::from_byte(input.u8()?)?,
            detail: input.text(MAX_DETAIL_BYTES)?,
        };
        input.finish()?;
        Ok(refused)
    }
}

impl Inspected {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        out.bytes(&self.request);
        out.u32(self.pid);
        out.path(&self.namespace)?;
        for identity in [&self.lifetime, &self.created_build, &self.active_build] {
            out.id35(identity)?;
        }
        out.optional(self.active_launcher.as_deref(), Writer::path)?;
        for number in [self.eonw, self.eonr, self.update_schema] {
            out.u16(number);
        }
        out.u8(self.presentation as u8);
        if self.hosts.len() > MAX_HOSTS {
            return Err(Error::Malformed("host count"));
        }
        out.u16(self.hosts.len() as u16);
        for host in &self.hosts {
            out.text(&host.terminal, MAX_ID_BYTES)?;
            out.text(&host.run, MAX_ID_BYTES)?;
            out.optional(host.origin.as_deref(), Writer::id35)?;
            out.u16(host.management);
            out.u16(host.record_generation);
            out.optional(host.orbs, |out, value| {
                out.u16(value);
                Ok(())
            })?;
            out.optional(host.orbf, |out, value| {
                out.u16(value);
                Ok(())
            })?;
        }
        out.optional(self.last.as_ref(), |out, last| {
            out.bytes(&last.request);
            out.u8(last.operation as u8);
            out.u8(last.refusal.map_or(0, |code| code as u8));
            out.text(&last.detail, MAX_DETAIL_BYTES)
        })?;
        frame(Kind::Inspected, out.0)
    }

    pub fn decode(frame: &[u8]) -> Result<Self> {
        let mut input = Reader(payload(frame, Kind::Inspected, Direction::Reply)?);
        let request = input.request()?;
        let pid = input.u32()?;
        let namespace = input.path()?;
        let lifetime = input.id35()?;
        let created_build = input.id35()?;
        let active_build = input.id35()?;
        let active_launcher = input.optional(Reader::path)?;
        let eonw = input.u16()?;
        let eonr = input.u16()?;
        let update_schema = input.u16()?;
        let presentation = match input.u8()? {
            0 => Presentation::Detached,
            1 => Presentation::Attached,
            _ => return Err(Error::Malformed("presentation")),
        };
        let count = usize::from(input.u16()?);
        if count > MAX_HOSTS {
            return Err(Error::Malformed("host count"));
        }
        let mut hosts = Vec::with_capacity(count);
        for _ in 0..count {
            hosts.push(Host {
                terminal: input.text(MAX_ID_BYTES)?,
                run: input.text(MAX_ID_BYTES)?,
                origin: input.optional(Reader::id35)?,
                management: input.u16()?,
                record_generation: input.u16()?,
                orbs: input.optional(Reader::u16)?,
                orbf: input.optional(Reader::u16)?,
            });
        }
        let last = input.optional(|input| {
            let request = input.request()?;
            let operation = match input.u8()? {
                2 => Operation::Preflight,
                3 => Operation::Commit,
                _ => return Err(Error::Malformed("outcome operation")),
            };
            let refusal = match input.u8()? {
                0 => None,
                code => {
                    Some(Code::from_byte(code).map_err(|_| Error::Malformed("outcome result"))?)
                }
            };
            Ok(Outcome {
                request,
                operation,
                refusal,
                detail: input.text(MAX_DETAIL_BYTES)?,
            })
        })?;
        input.finish()?;
        Ok(Self {
            request,
            pid,
            namespace,
            lifetime,
            created_build,
            active_build,
            active_launcher,
            eonw,
            eonr,
            update_schema,
            presentation,
            hosts,
            last,
        })
    }
}

/// Whether `value` is a lifetime or build identity: `g1-` and 32 lowercase
/// hex digits (the frozen Reserved Stop spelling, validated here on its own).
pub fn valid_id35(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_path(path: &[u8]) -> bool {
    !path.is_empty() && path.len() <= MAX_PATH_BYTES && path[0] == b'/' && !path.contains(&0)
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }
    fn sized(&mut self, bytes: &[u8]) {
        self.u16(bytes.len() as u16);
        self.bytes(bytes);
    }
    fn text(&mut self, value: &str, maximum: usize) -> Result<()> {
        if value.len() > maximum {
            return Err(Error::Malformed("text length"));
        }
        self.sized(value.as_bytes());
        Ok(())
    }
    fn path(&mut self, path: &[u8]) -> Result<()> {
        if !valid_path(path) {
            return Err(Error::Malformed("path"));
        }
        self.sized(path);
        Ok(())
    }
    fn id35(&mut self, value: &str) -> Result<()> {
        if !valid_id35(value) {
            return Err(Error::Malformed("identity"));
        }
        self.bytes(value.as_bytes());
        Ok(())
    }
    fn optional<T>(
        &mut self,
        value: Option<T>,
        write: impl FnOnce(&mut Self, T) -> Result<()>,
    ) -> Result<()> {
        match value {
            None => {
                self.u8(0);
                Ok(())
            }
            Some(value) => {
                self.u8(1);
                write(self, value)
            }
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        if self.0.len() < length {
            return Err(Error::Truncated);
        }
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(taken)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    fn request(&mut self) -> Result<RequestId> {
        Ok(self.take(16)?.try_into().expect("16 bytes"))
    }
    fn sized(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = usize::from(self.u16()?);
        if length > maximum {
            return Err(Error::Malformed("length"));
        }
        self.take(length)
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        std::str::from_utf8(self.sized(maximum)?)
            .map(str::to_owned)
            .map_err(|_| Error::Malformed("text encoding"))
    }
    fn path(&mut self) -> Result<Vec<u8>> {
        let path = self.sized(MAX_PATH_BYTES)?;
        if !valid_path(path) {
            return Err(Error::Malformed("path"));
        }
        Ok(path.to_vec())
    }
    fn id35(&mut self) -> Result<String> {
        let value =
            std::str::from_utf8(self.take(35)?).map_err(|_| Error::Malformed("identity"))?;
        if !valid_id35(value) {
            return Err(Error::Malformed("identity"));
        }
        Ok(value.to_owned())
    }
    fn optional<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(Error::Malformed("presence tag")),
        }
    }
    fn finish(self) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Malformed("trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests;
