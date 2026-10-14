//! Private bounded lifecycle management values for one Orbit run.

use std::fmt;

const MESSAGE_MAGIC: [u8; 4] = *b"ORBM";
const RECORD_MAGIC: [u8; 4] = *b"ORBR";

pub const VERSION: u16 = 1;
pub const RECORD_GENERATION: u16 = 1;
pub const HEADER_BYTES: usize = 12;
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024;
pub const MAX_RECORD_BYTES: usize = 4 * 1024;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_DETAIL_BYTES: usize = 1024;
const MAX_PATH_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointIdentity {
    pub path: Vec<u8>,
    pub object: ObjectIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveIdentity {
    pub session_id: String,
    pub run_id: String,
    pub component_generation: String,
    pub record_generation: u16,
    pub management_generation: u16,
    pub process_id: u32,
    pub process_start: u64,
    pub uid: u32,
    pub presentation: EndpointIdentity,
    pub management: EndpointIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminationReason {
    NaturalExit,
    ExplicitStop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessOutcome {
    ExitCode(i32),
    Signal(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub identity: LiveIdentity,
    pub reason: TerminationReason,
    pub outcome: ProcessOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCode {
    InvalidIdentity,
    InvalidRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: FailureCode,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    Acquire {
        expected: LiveIdentity,
        record: ObjectIdentity,
    },
    Status,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    Lease(LiveIdentity),
    Status(LiveIdentity),
    Busy,
    Stopped(Tombstone),
    Failure(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Live(LiveIdentity),
    Tombstone(Tombstone),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidMagic,
    UnsupportedVersion {
        version: u16,
    },
    InvalidFlags {
        value: u8,
    },
    PayloadTooLarge {
        size: usize,
        maximum: usize,
    },
    ValueTooLarge {
        field: &'static str,
        size: usize,
        maximum: usize,
    },
    Truncated,
    TrailingBytes {
        count: usize,
    },
    InvalidTag {
        field: &'static str,
        value: u8,
    },
    InvalidValue {
        field: &'static str,
    },
    InvalidUtf8 {
        field: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("invalid management magic"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported management version {version}")
            }
            Self::InvalidFlags { value } => {
                write!(formatter, "invalid management flags {value:#x}")
            }
            Self::PayloadTooLarge { size, maximum } => {
                write!(
                    formatter,
                    "management payload is {size} bytes; maximum is {maximum}"
                )
            }
            Self::ValueTooLarge {
                field,
                size,
                maximum,
            } => write!(formatter, "{field} is {size} bytes; maximum is {maximum}"),
            Self::Truncated => formatter.write_str("truncated management value"),
            Self::TrailingBytes { count } => {
                write!(formatter, "management value has {count} trailing bytes")
            }
            Self::InvalidTag { field, value } => {
                write!(formatter, "invalid management {field} tag {value}")
            }
            Self::InvalidValue { field } => write!(formatter, "invalid management {field}"),
            Self::InvalidUtf8 { field } => write!(formatter, "management {field} is not UTF-8"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn client_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    framed_len(bytes, MESSAGE_MAGIC, MAX_MESSAGE_BYTES)
}

pub fn server_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    framed_len(bytes, MESSAGE_MAGIC, MAX_MESSAGE_BYTES)
}

pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>> {
    match message {
        ClientMessage::Acquire { expected, record } => {
            encode(MESSAGE_MAGIC, 1, MAX_MESSAGE_BYTES, |out| {
                encode_live(out, expected)?;
                encode_object(out, *record);
                Ok(())
            })
        }
        ClientMessage::Status => encode(MESSAGE_MAGIC, 2, MAX_MESSAGE_BYTES, |_| Ok(())),
        ClientMessage::Stop => encode(MESSAGE_MAGIC, 3, MAX_MESSAGE_BYTES, |_| Ok(())),
    }
}

pub fn decode_client_message(bytes: &[u8]) -> Result<ClientMessage> {
    let (tag, mut input) = decode_frame(bytes, MESSAGE_MAGIC, MAX_MESSAGE_BYTES)?;
    let message = match tag {
        1 => ClientMessage::Acquire {
            expected: decode_live(&mut input)?,
            record: decode_object(&mut input)?,
        },
        2 => ClientMessage::Status,
        3 => ClientMessage::Stop,
        value => {
            return Err(Error::InvalidTag {
                field: "client message",
                value,
            });
        }
    };
    input.finish()?;
    Ok(message)
}

pub fn encode_server_message(message: &ServerMessage) -> Result<Vec<u8>> {
    match message {
        ServerMessage::Lease(identity) => encode(MESSAGE_MAGIC, 64, MAX_MESSAGE_BYTES, |out| {
            encode_live(out, identity)
        }),
        ServerMessage::Status(identity) => encode(MESSAGE_MAGIC, 65, MAX_MESSAGE_BYTES, |out| {
            encode_live(out, identity)
        }),
        ServerMessage::Busy => encode(MESSAGE_MAGIC, 66, MAX_MESSAGE_BYTES, |_| Ok(())),
        ServerMessage::Stopped(tombstone) => encode(MESSAGE_MAGIC, 67, MAX_MESSAGE_BYTES, |out| {
            encode_tombstone(out, tombstone)
        }),
        ServerMessage::Failure(failure) => encode(MESSAGE_MAGIC, 68, MAX_MESSAGE_BYTES, |out| {
            out.u8(match failure.code {
                FailureCode::InvalidIdentity => 1,
                FailureCode::InvalidRequest => 2,
            });
            out.text("failure detail", &failure.detail, MAX_DETAIL_BYTES, true)
        }),
    }
}

pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    let (tag, mut input) = decode_frame(bytes, MESSAGE_MAGIC, MAX_MESSAGE_BYTES)?;
    let message = match tag {
        64 => ServerMessage::Lease(decode_live(&mut input)?),
        65 => ServerMessage::Status(decode_live(&mut input)?),
        66 => ServerMessage::Busy,
        67 => ServerMessage::Stopped(decode_tombstone(&mut input)?),
        68 => ServerMessage::Failure(Failure {
            code: match input.u8()? {
                1 => FailureCode::InvalidIdentity,
                2 => FailureCode::InvalidRequest,
                value => {
                    return Err(Error::InvalidTag {
                        field: "failure code",
                        value,
                    });
                }
            },
            detail: input.text("failure detail", MAX_DETAIL_BYTES, true)?,
        }),
        value => {
            return Err(Error::InvalidTag {
                field: "server message",
                value,
            });
        }
    };
    input.finish()?;
    Ok(message)
}

pub fn encode_record(record: &Record) -> Result<Vec<u8>> {
    match record {
        Record::Live(identity) => encode(RECORD_MAGIC, 1, MAX_RECORD_BYTES, |out| {
            encode_live(out, identity)
        }),
        Record::Tombstone(tombstone) => encode(RECORD_MAGIC, 2, MAX_RECORD_BYTES, |out| {
            encode_tombstone(out, tombstone)
        }),
    }
}

pub fn decode_record(bytes: &[u8]) -> Result<Record> {
    let (tag, mut input) = decode_frame(bytes, RECORD_MAGIC, MAX_RECORD_BYTES)?;
    let record = match tag {
        1 => Record::Live(decode_live(&mut input)?),
        2 => Record::Tombstone(decode_tombstone(&mut input)?),
        value => {
            return Err(Error::InvalidTag {
                field: "record",
                value,
            });
        }
    };
    input.finish()?;
    Ok(record)
}

fn encode_live(out: &mut Encoder, identity: &LiveIdentity) -> Result<()> {
    validate_live(identity)?;
    out.text("session ID", &identity.session_id, MAX_ID_BYTES, false)?;
    out.text("run ID", &identity.run_id, MAX_ID_BYTES, false)?;
    out.text(
        "component generation",
        &identity.component_generation,
        MAX_ID_BYTES,
        false,
    )?;
    out.u16(identity.record_generation);
    out.u16(identity.management_generation);
    out.u32(identity.process_id);
    out.u64(identity.process_start);
    out.u32(identity.uid);
    encode_endpoint(out, &identity.presentation)?;
    encode_endpoint(out, &identity.management)
}

fn decode_live(input: &mut Decoder<'_>) -> Result<LiveIdentity> {
    let identity = LiveIdentity {
        session_id: input.text("session ID", MAX_ID_BYTES, false)?,
        run_id: input.text("run ID", MAX_ID_BYTES, false)?,
        component_generation: input.text("component generation", MAX_ID_BYTES, false)?,
        record_generation: input.u16()?,
        management_generation: input.u16()?,
        process_id: input.u32()?,
        process_start: input.u64()?,
        uid: input.u32()?,
        presentation: decode_endpoint(input)?,
        management: decode_endpoint(input)?,
    };
    validate_live(&identity)?;
    Ok(identity)
}

fn validate_live(identity: &LiveIdentity) -> Result<()> {
    validate_text("session ID", &identity.session_id, MAX_ID_BYTES, false)?;
    validate_text("run ID", &identity.run_id, MAX_ID_BYTES, false)?;
    validate_text(
        "component generation",
        &identity.component_generation,
        MAX_ID_BYTES,
        false,
    )?;
    if identity.record_generation != RECORD_GENERATION {
        return Err(Error::UnsupportedVersion {
            version: identity.record_generation,
        });
    }
    if identity.management_generation != VERSION {
        return Err(Error::UnsupportedVersion {
            version: identity.management_generation,
        });
    }
    if identity.process_id == 0 {
        return Err(Error::InvalidValue {
            field: "process ID",
        });
    }
    if identity.process_start == 0 {
        return Err(Error::InvalidValue {
            field: "process start identity",
        });
    }
    validate_endpoint("presentation endpoint", &identity.presentation)?;
    validate_endpoint("management endpoint", &identity.management)?;
    if identity.presentation.path == identity.management.path
        || identity.presentation.object == identity.management.object
    {
        return Err(Error::InvalidValue {
            field: "distinct endpoints",
        });
    }
    Ok(())
}

fn encode_endpoint(out: &mut Encoder, endpoint: &EndpointIdentity) -> Result<()> {
    validate_endpoint("endpoint", endpoint)?;
    out.bytes("endpoint path", &endpoint.path, MAX_PATH_BYTES, false)?;
    encode_object(out, endpoint.object);
    Ok(())
}

fn decode_endpoint(input: &mut Decoder<'_>) -> Result<EndpointIdentity> {
    let endpoint = EndpointIdentity {
        path: input.bytes("endpoint path", MAX_PATH_BYTES, false)?,
        object: decode_object(input)?,
    };
    validate_endpoint("endpoint", &endpoint)?;
    Ok(endpoint)
}

fn validate_endpoint(field: &'static str, endpoint: &EndpointIdentity) -> Result<()> {
    validate_bytes("endpoint path", &endpoint.path, MAX_PATH_BYTES, false)?;
    if endpoint.object.inode == 0 {
        return Err(Error::InvalidValue { field });
    }
    Ok(())
}

fn encode_object(out: &mut Encoder, object: ObjectIdentity) {
    out.u64(object.device);
    out.u64(object.inode);
}

fn decode_object(input: &mut Decoder<'_>) -> Result<ObjectIdentity> {
    let object = ObjectIdentity {
        device: input.u64()?,
        inode: input.u64()?,
    };
    if object.inode == 0 {
        return Err(Error::InvalidValue {
            field: "object identity",
        });
    }
    Ok(object)
}

fn encode_tombstone(out: &mut Encoder, tombstone: &Tombstone) -> Result<()> {
    encode_live(out, &tombstone.identity)?;
    out.u8(match tombstone.reason {
        TerminationReason::NaturalExit => 1,
        TerminationReason::ExplicitStop => 2,
    });
    match tombstone.outcome {
        ProcessOutcome::ExitCode(code @ 0..=255) => {
            out.u8(1);
            out.i32(code);
        }
        ProcessOutcome::ExitCode(_) => {
            return Err(Error::InvalidValue { field: "exit code" });
        }
        ProcessOutcome::Signal(signal) if signal > 0 => {
            out.u8(2);
            out.i32(signal);
        }
        ProcessOutcome::Signal(_) => {
            return Err(Error::InvalidValue {
                field: "termination signal",
            });
        }
    }
    Ok(())
}

fn decode_tombstone(input: &mut Decoder<'_>) -> Result<Tombstone> {
    let identity = decode_live(input)?;
    let reason = match input.u8()? {
        1 => TerminationReason::NaturalExit,
        2 => TerminationReason::ExplicitStop,
        value => {
            return Err(Error::InvalidTag {
                field: "termination reason",
                value,
            });
        }
    };
    let outcome = match input.u8()? {
        1 => {
            let code = input.i32()?;
            if !(0..=255).contains(&code) {
                return Err(Error::InvalidValue { field: "exit code" });
            }
            ProcessOutcome::ExitCode(code)
        }
        2 => {
            let signal = input.i32()?;
            if signal <= 0 {
                return Err(Error::InvalidValue {
                    field: "termination signal",
                });
            }
            ProcessOutcome::Signal(signal)
        }
        value => {
            return Err(Error::InvalidTag {
                field: "process outcome",
                value,
            });
        }
    };
    Ok(Tombstone {
        identity,
        reason,
        outcome,
    })
}

fn encode(
    magic: [u8; 4],
    tag: u8,
    maximum: usize,
    payload: impl FnOnce(&mut Encoder) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut out = Encoder::default();
    payload(&mut out)?;
    let total = HEADER_BYTES
        .checked_add(out.bytes.len())
        .ok_or(Error::PayloadTooLarge {
            size: usize::MAX,
            maximum,
        })?;
    if total > maximum {
        return Err(Error::PayloadTooLarge {
            size: total,
            maximum,
        });
    }
    let payload_len = u32::try_from(out.bytes.len()).map_err(|_| Error::PayloadTooLarge {
        size: total,
        maximum,
    })?;
    let mut framed = Vec::with_capacity(total);
    framed.extend_from_slice(&magic);
    framed.extend_from_slice(&VERSION.to_le_bytes());
    framed.push(tag);
    framed.push(0);
    framed.extend_from_slice(&payload_len.to_le_bytes());
    framed.extend_from_slice(&out.bytes);
    Ok(framed)
}

fn framed_len(bytes: &[u8], magic: [u8; 4], maximum: usize) -> Result<Option<usize>> {
    if bytes.len() < HEADER_BYTES {
        return Ok(None);
    }
    if bytes[..4] != magic {
        return Err(Error::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(Error::UnsupportedVersion { version });
    }
    if bytes[7] != 0 {
        return Err(Error::InvalidFlags { value: bytes[7] });
    }
    let payload = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed header")) as usize;
    let total = HEADER_BYTES
        .checked_add(payload)
        .ok_or(Error::PayloadTooLarge {
            size: usize::MAX,
            maximum,
        })?;
    if total > maximum {
        return Err(Error::PayloadTooLarge {
            size: total,
            maximum,
        });
    }
    Ok(Some(total))
}

fn decode_frame(bytes: &[u8], magic: [u8; 4], maximum: usize) -> Result<(u8, Decoder<'_>)> {
    let total = framed_len(bytes, magic, maximum)?.ok_or(Error::Truncated)?;
    if bytes.len() < total {
        return Err(Error::Truncated);
    }
    if bytes.len() > total {
        return Err(Error::TrailingBytes {
            count: bytes.len() - total,
        });
    }
    Ok((bytes[6], Decoder::new(&bytes[HEADER_BYTES..])))
}

#[derive(Default)]
struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn text(
        &mut self,
        field: &'static str,
        value: &str,
        maximum: usize,
        empty: bool,
    ) -> Result<()> {
        self.bytes(field, value.as_bytes(), maximum, empty)
    }

    fn bytes(
        &mut self,
        field: &'static str,
        value: &[u8],
        maximum: usize,
        empty: bool,
    ) -> Result<()> {
        validate_bytes(field, value, maximum, empty)?;
        let length = u16::try_from(value.len()).map_err(|_| Error::ValueTooLarge {
            field,
            size: value.len(),
            maximum,
        })?;
        self.u16(length);
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(count).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("fixed integer"),
        ))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("fixed integer"),
        ))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("fixed integer"),
        ))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("fixed integer"),
        ))
    }

    fn bytes(&mut self, field: &'static str, maximum: usize, empty: bool) -> Result<Vec<u8>> {
        let length = self.u16()? as usize;
        if length > maximum {
            return Err(Error::ValueTooLarge {
                field,
                size: length,
                maximum,
            });
        }
        let value = self.take(length)?.to_vec();
        validate_bytes(field, &value, maximum, empty)?;
        Ok(value)
    }

    fn text(&mut self, field: &'static str, maximum: usize, empty: bool) -> Result<String> {
        let bytes = self.bytes(field, maximum, empty)?;
        String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8 { field })
    }

    fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::TrailingBytes {
                count: self.bytes.len() - self.offset,
            })
        }
    }
}

fn validate_text(field: &'static str, value: &str, maximum: usize, empty: bool) -> Result<()> {
    validate_bytes(field, value.as_bytes(), maximum, empty)
}

fn validate_bytes(field: &'static str, value: &[u8], maximum: usize, empty: bool) -> Result<()> {
    if !empty && value.is_empty() {
        return Err(Error::InvalidValue { field });
    }
    if value.len() > maximum {
        return Err(Error::ValueTooLarge {
            field,
            size: value.len(),
            maximum,
        });
    }
    Ok(())
}
