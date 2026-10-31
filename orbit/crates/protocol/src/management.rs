//! Private bounded lifecycle management values for one Orbit run.

use crate::session::MAX_METADATA_BYTES;
use std::fmt;

const MESSAGE_MAGIC: [u8; 4] = *b"ORBM";
const RECORD_MAGIC: [u8; 4] = *b"ORBR";

pub const VERSION: u16 = 4;
pub const RECORD_GENERATION: u16 = 1;
pub const HEADER_BYTES: usize = 12;
/// Every message's bound, except a `Visible` reply's and an `Input`
/// request's.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024;
pub const MAX_RECORD_BYTES: usize = 4 * 1024;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_DETAIL_BYTES: usize = 1024;
const MAX_PATH_BYTES: usize = 1024;
/// The most program status records a terminal holds (ORB-C15).
pub const MAX_PROGRAM_RECORDS: u16 = 256;
pub const MAX_PROGRAM_ID_BYTES: usize = 128;
pub const MAX_PROGRAM_APP_BYTES: usize = 32;
/// The reply's bounds on `title` and `msg`; Orbit truncates longer text at a
/// UTF-8 boundary.
pub const MAX_PROGRAM_TITLE_BYTES: usize = 96;
pub const MAX_PROGRAM_MESSAGE_BYTES: usize = 256;
/// The reply's bound on the terminal title, which Orbit cuts at a UTF-8
/// boundary.
pub const MAX_TERMINAL_TITLE_BYTES: usize = 64;
/// The bound on a `Visible` reply's text (ORB-C17).
pub const MAX_VISIBLE_TEXT_BYTES: usize = 1024 * 1024;
/// The bytes a `Visible` reply's payload takes besides its title, working
/// directory and text.
const VISIBLE_FIXED_BYTES: usize = 8 + 1 + 2 + 2 + 4 + 4 + 4 + 1 + 4;
/// The bound on a `Visible` reply, whose title and working directory together
/// stay within ORBS's `Metadata` bound.
pub const MAX_VISIBLE_MESSAGE_BYTES: usize =
    HEADER_BYTES + VISIBLE_FIXED_BYTES + MAX_METADATA_BYTES + MAX_VISIBLE_TEXT_BYTES;
/// The bound on a lease paste's text (ORB-C5).
pub const MAX_INPUT_PASTE_BYTES: usize = 64 * 1024;
/// The bound on an `Input` request: its tag and pasted text.
pub const MAX_INPUT_MESSAGE_BYTES: usize = HEADER_BYTES + 1 + MAX_INPUT_PASTE_BYTES;

const CLIENT_ACQUIRE: u8 = 1;
const CLIENT_STATUS: u8 = 2;
const CLIENT_STOP: u8 = 3;
const CLIENT_PROGRAM_STATUS: u8 = 4;
const CLIENT_READ_VISIBLE: u8 = 5;
const CLIENT_INPUT: u8 = 6;
const CLIENT_ATTACH_ALLOW: u8 = 7;
const SERVER_LEASE: u8 = 64;
const SERVER_STATUS: u8 = 65;
const SERVER_BUSY: u8 = 66;
const SERVER_STOPPED: u8 = 67;
const SERVER_FAILURE: u8 = 68;
const SERVER_PROGRAM_STATUS: u8 = 71;
const SERVER_VISIBLE: u8 = 72;
const SERVER_INPUT: u8 = 73;
const SERVER_ALLOWED: u8 = 74;
const SERVER_REFUSED: u8 = 75;

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

/// A program status state of OSC 7501 (ORB-C15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

/// What a blocked program needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramKind {
    Permission,
    Question,
    Auth,
}

/// One program status record and its neutral facts (ORB-C15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramRecord {
    /// The full id; empty for the root record.
    pub id: String,
    pub state: ProgramState,
    /// Only a blocked record has a kind.
    pub kind: Option<ProgramKind>,
    /// From 0 through 100.
    pub progress: Option<u8>,
    /// The record's app, or that of its nearest ancestor that has one.
    pub app: String,
    pub title: String,
    pub message: String,
    /// The update sequence of the record's last report.
    pub updated: u64,
    /// The update sequence of the report that created the record.
    pub created: u64,
    /// The update sequence at which its state last changed.
    pub changed: u64,
    /// The state before that change, or none for a new record.
    pub previous: Option<ProgramState>,
    /// Milliseconds since the last update and since the last state change,
    /// on Orbit's monotonic clock.
    pub updated_age_ms: u64,
    pub changed_age_ms: u64,
}

/// The `ProgramStatus` reply: the terminal's record count, its background
/// process count, its latest update sequence, its title, and as many records
/// as fit in 4 KiB in ORB-C15's fill order; `omitted` holds the creation
/// sequence of each of the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatus {
    pub count: u16,
    /// The terminal's background processes, at most 255; 0 while it holds
    /// no records.
    pub background: u8,
    pub update: u64,
    /// The terminal's window title, sanitized; empty when it has none.
    pub terminal_title: String,
    pub omitted: Vec<u64>,
    pub records: Vec<ProgramRecord>,
}

/// Which screen of the terminal is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Primary,
    Alternate,
}

/// The `Visible` reply (ORB-C17): one snapshot of the active screen's live
/// area as lossy plain text at one presentation revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleText {
    pub revision: u64,
    pub screen: Screen,
    pub rows: u16,
    pub columns: u16,
    /// The terminal's title, as ORBS's `Metadata` carries it.
    pub title: String,
    /// The terminal's raw working directory.
    pub directory: Vec<u8>,
    /// How many leading rows the 1 MiB bound left out.
    pub omitted_rows: u32,
    /// Always true: the text drops styles, colours, hyperlinks, images,
    /// cursor and selection.
    pub lossy: bool,
    /// One line per display row, top to bottom, joined by line feeds; it
    /// holds no other control character.
    pub text: String,
}

/// One lease input request (ORB-C5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseInput {
    /// Nonempty UTF-8 text of at most [`MAX_INPUT_PASTE_BYTES`].
    Paste(String),
    Enter,
}

/// Why Orbit refused a request under the lease, which stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A whole request that did not decode, or one sent while another was in
    /// flight, until its answer was fully written.
    InvalidRequest,
    /// A paste holds a line feed or carriage return while the program has not
    /// enabled bracketed paste.
    UnbracketedNewline,
    /// A paste holds a control character other than tab, line feed and
    /// carriage return.
    InvalidInput,
    /// The PTY input queue cannot take the input whole.
    InputFull,
    /// Extracting the visible text failed.
    ReadFailed,
    /// The PTY has closed, so it takes no input, while the child may still
    /// run.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    Acquire {
        expected: LiveIdentity,
        record: ObjectIdentity,
    },
    Status,
    Stop,
    ProgramStatus,
    ReadVisible,
    Input(LeaseInput),
    /// Names the one process, by process ID and process start identity, that
    /// a run with management admits as its interactive client (ORB-C3).
    AttachAllow {
        process_id: u32,
        process_start: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    Lease(LiveIdentity),
    Status(LiveIdentity),
    Busy,
    Stopped(Tombstone),
    Failure(Failure),
    ProgramStatus(ProgramStatus),
    Visible(VisibleText),
    /// The paste or Enter was written; `bracketed` tells whether the paste
    /// was wrapped in bracketed-paste markers, and is false for Enter.
    Input {
        bracketed: bool,
    },
    /// The named process was recorded.
    Allowed,
    Refused(Refusal),
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

/// A client message's bound, by its kind.
fn client_bound(kind: u8) -> usize {
    match kind {
        CLIENT_INPUT => MAX_INPUT_MESSAGE_BYTES,
        _ => MAX_MESSAGE_BYTES,
    }
}

/// A server message's bound, by its kind.
fn server_bound(kind: u8) -> usize {
    match kind {
        SERVER_VISIBLE => MAX_VISIBLE_MESSAGE_BYTES,
        _ => MAX_MESSAGE_BYTES,
    }
}

pub fn client_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    framed_len(bytes, MESSAGE_MAGIC, client_bound)
}

pub fn server_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    framed_len(bytes, MESSAGE_MAGIC, server_bound)
}

pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>> {
    let client = |tag, payload: &dyn Fn(&mut Encoder) -> Result<()>| {
        encode(MESSAGE_MAGIC, tag, client_bound(tag), payload)
    };
    match message {
        ClientMessage::Acquire { expected, record } => client(CLIENT_ACQUIRE, &|out| {
            encode_live(out, expected)?;
            encode_object(out, *record);
            Ok(())
        }),
        ClientMessage::Status => client(CLIENT_STATUS, &|_| Ok(())),
        ClientMessage::Stop => client(CLIENT_STOP, &|_| Ok(())),
        ClientMessage::ProgramStatus => client(CLIENT_PROGRAM_STATUS, &|_| Ok(())),
        ClientMessage::ReadVisible => client(CLIENT_READ_VISIBLE, &|_| Ok(())),
        ClientMessage::Input(input) => client(CLIENT_INPUT, &|out| {
            match input {
                LeaseInput::Paste(text) => {
                    validate_paste(text.as_bytes())?;
                    out.u8(1);
                    out.bytes.extend_from_slice(text.as_bytes());
                }
                LeaseInput::Enter => out.u8(2),
            }
            Ok(())
        }),
        ClientMessage::AttachAllow {
            process_id,
            process_start,
        } => client(CLIENT_ATTACH_ALLOW, &|out| {
            validate_process(*process_id, *process_start)?;
            out.u32(*process_id);
            out.u64(*process_start);
            Ok(())
        }),
    }
}

pub fn decode_client_message(bytes: &[u8]) -> Result<ClientMessage> {
    let (tag, mut input) = decode_frame(bytes, MESSAGE_MAGIC, client_bound)?;
    let message = match tag {
        CLIENT_ACQUIRE => ClientMessage::Acquire {
            expected: decode_live(&mut input)?,
            record: decode_object(&mut input)?,
        },
        CLIENT_STATUS => ClientMessage::Status,
        CLIENT_STOP => ClientMessage::Stop,
        CLIENT_PROGRAM_STATUS => ClientMessage::ProgramStatus,
        CLIENT_READ_VISIBLE => ClientMessage::ReadVisible,
        CLIENT_INPUT => ClientMessage::Input(match input.u8()? {
            1 => {
                let bytes = input.rest();
                validate_paste(bytes)?;
                LeaseInput::Paste(String::from_utf8(bytes.to_vec()).map_err(|_| {
                    Error::InvalidUtf8 {
                        field: "paste text",
                    }
                })?)
            }
            2 => LeaseInput::Enter,
            value => {
                return Err(Error::InvalidTag {
                    field: "lease input",
                    value,
                });
            }
        }),
        CLIENT_ATTACH_ALLOW => {
            let (process_id, process_start) = (input.u32()?, input.u64()?);
            validate_process(process_id, process_start)?;
            ClientMessage::AttachAllow {
                process_id,
                process_start,
            }
        }
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
    let server = |tag, payload: &dyn Fn(&mut Encoder) -> Result<()>| {
        encode(MESSAGE_MAGIC, tag, server_bound(tag), payload)
    };
    match message {
        ServerMessage::Lease(identity) => server(SERVER_LEASE, &|out| encode_live(out, identity)),
        ServerMessage::Status(identity) => server(SERVER_STATUS, &|out| encode_live(out, identity)),
        ServerMessage::Busy => server(SERVER_BUSY, &|_| Ok(())),
        ServerMessage::Stopped(tombstone) => {
            server(SERVER_STOPPED, &|out| encode_tombstone(out, tombstone))
        }
        ServerMessage::Failure(failure) => server(SERVER_FAILURE, &|out| {
            out.u8(match failure.code {
                FailureCode::InvalidIdentity => 1,
                FailureCode::InvalidRequest => 2,
            });
            out.text("failure detail", &failure.detail, MAX_DETAIL_BYTES, true)
        }),
        ServerMessage::ProgramStatus(status) => server(SERVER_PROGRAM_STATUS, &|out| {
            encode_program_status(out, status)
        }),
        ServerMessage::Visible(visible) => {
            server(SERVER_VISIBLE, &|out| encode_visible(out, visible))
        }
        ServerMessage::Input { bracketed } => server(SERVER_INPUT, &|out| {
            out.u8(u8::from(*bracketed));
            Ok(())
        }),
        ServerMessage::Allowed => server(SERVER_ALLOWED, &|_| Ok(())),
        ServerMessage::Refused(refusal) => server(SERVER_REFUSED, &|out| {
            out.u8(match refusal {
                Refusal::InvalidRequest => 1,
                Refusal::UnbracketedNewline => 2,
                Refusal::InvalidInput => 3,
                Refusal::InputFull => 4,
                Refusal::ReadFailed => 5,
                Refusal::Closed => 6,
            });
            Ok(())
        }),
    }
}

pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    let (tag, mut input) = decode_frame(bytes, MESSAGE_MAGIC, server_bound)?;
    let message = match tag {
        SERVER_LEASE => ServerMessage::Lease(decode_live(&mut input)?),
        SERVER_STATUS => ServerMessage::Status(decode_live(&mut input)?),
        SERVER_BUSY => ServerMessage::Busy,
        SERVER_STOPPED => ServerMessage::Stopped(decode_tombstone(&mut input)?),
        SERVER_FAILURE => ServerMessage::Failure(Failure {
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
        SERVER_PROGRAM_STATUS => ServerMessage::ProgramStatus(decode_program_status(&mut input)?),
        SERVER_VISIBLE => ServerMessage::Visible(decode_visible(&mut input)?),
        SERVER_INPUT => ServerMessage::Input {
            bracketed: decode_bool(&mut input, "bracketed")?,
        },
        SERVER_ALLOWED => ServerMessage::Allowed,
        SERVER_REFUSED => ServerMessage::Refused(match input.u8()? {
            1 => Refusal::InvalidRequest,
            2 => Refusal::UnbracketedNewline,
            3 => Refusal::InvalidInput,
            4 => Refusal::InputFull,
            5 => Refusal::ReadFailed,
            6 => Refusal::Closed,
            value => {
                return Err(Error::InvalidTag {
                    field: "refusal",
                    value,
                });
            }
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

fn validate_paste(text: &[u8]) -> Result<()> {
    validate_bytes("paste text", text, MAX_INPUT_PASTE_BYTES, false)
}

fn validate_process(process_id: u32, process_start: u64) -> Result<()> {
    if process_id == 0 {
        return Err(Error::InvalidValue {
            field: "process ID",
        });
    }
    if process_start == 0 {
        return Err(Error::InvalidValue {
            field: "process start identity",
        });
    }
    Ok(())
}

fn decode_bool(input: &mut Decoder<'_>, field: &'static str) -> Result<bool> {
    match input.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(Error::InvalidTag { field, value }),
    }
}

fn validate_visible(visible: &VisibleText) -> Result<()> {
    if !visible.lossy {
        return Err(Error::InvalidValue { field: "lossy" });
    }
    let metadata = visible.title.len() + visible.directory.len();
    if metadata > MAX_METADATA_BYTES {
        return Err(Error::ValueTooLarge {
            field: "visible title and directory",
            size: metadata,
            maximum: MAX_METADATA_BYTES,
        });
    }
    validate_bytes(
        "visible text",
        visible.text.as_bytes(),
        MAX_VISIBLE_TEXT_BYTES,
        true,
    )?;
    if !visible.text.chars().all(|c| c == '\n' || !c.is_control()) {
        return Err(Error::InvalidValue {
            field: "visible text",
        });
    }
    Ok(())
}

fn encode_visible(out: &mut Encoder, visible: &VisibleText) -> Result<()> {
    validate_visible(visible)?;
    out.u64(visible.revision);
    out.u8(match visible.screen {
        Screen::Primary => 1,
        Screen::Alternate => 2,
    });
    out.u16(visible.rows);
    out.u16(visible.columns);
    out.long_bytes(visible.title.as_bytes());
    out.long_bytes(&visible.directory);
    out.u32(visible.omitted_rows);
    out.u8(u8::from(visible.lossy));
    out.long_bytes(visible.text.as_bytes());
    Ok(())
}

fn decode_visible(input: &mut Decoder<'_>) -> Result<VisibleText> {
    let revision = input.u64()?;
    let screen = match input.u8()? {
        1 => Screen::Primary,
        2 => Screen::Alternate,
        value => {
            return Err(Error::InvalidTag {
                field: "screen",
                value,
            });
        }
    };
    let rows = input.u16()?;
    let columns = input.u16()?;
    let title = input.long_text("visible title", MAX_METADATA_BYTES)?;
    let directory = input.long_bytes("visible directory", MAX_METADATA_BYTES)?;
    let omitted_rows = input.u32()?;
    let lossy = decode_bool(input, "lossy")?;
    let text = input.long_text("visible text", MAX_VISIBLE_TEXT_BYTES)?;
    let visible = VisibleText {
        revision,
        screen,
        rows,
        columns,
        title,
        directory,
        omitted_rows,
        lossy,
        text,
    };
    validate_visible(&visible)?;
    Ok(visible)
}

pub fn encode_record(record: &Record) -> Result<Vec<u8>> {
    match record {
        Record::Live(identity) => encode(RECORD_MAGIC, 1, MAX_RECORD_BYTES, &|out| {
            encode_live(out, identity)
        }),
        Record::Tombstone(tombstone) => encode(RECORD_MAGIC, 2, MAX_RECORD_BYTES, &|out| {
            encode_tombstone(out, tombstone)
        }),
    }
}

pub fn decode_record(bytes: &[u8]) -> Result<Record> {
    let (tag, mut input) = decode_frame(bytes, RECORD_MAGIC, |_| MAX_RECORD_BYTES)?;
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

/// The bytes one record takes in a reply.
pub fn program_record_len(record: &ProgramRecord) -> usize {
    // id, app, title and message lengths; state, kind, progress and previous;
    // five sequences and ages.
    4 * 2
        + record.id.len()
        + record.app.len()
        + record.title.len()
        + record.message.len()
        + 4
        + 5 * 8
}

/// The bytes a reply's message takes before its terminal title's text, its
/// records and omitted creation sequences.
pub const PROGRAM_STATUS_HEADER_BYTES: usize = HEADER_BYTES + 2 + 1 + 8 + 2 + 2;

/// The bytes one omitted record's creation sequence takes in a reply.
pub const PROGRAM_OMITTED_BYTES: usize = 8;

fn encode_program_status(out: &mut Encoder, status: &ProgramStatus) -> Result<()> {
    validate_program_counts(status.count, status.omitted.len(), status.records.len())?;
    out.u16(status.count);
    out.u8(status.background);
    out.u64(status.update);
    out.text(
        "terminal title",
        &status.terminal_title,
        MAX_TERMINAL_TITLE_BYTES,
        true,
    )?;
    out.u16(status.omitted.len() as u16);
    for &created in &status.omitted {
        out.u64(created);
    }
    for record in &status.records {
        if record.progress.is_some_and(|progress| progress > 100) {
            return Err(Error::InvalidValue { field: "progress" });
        }
        if record.kind.is_some() && record.state != ProgramState::Blocked {
            return Err(Error::InvalidValue {
                field: "program kind",
            });
        }
        out.text("program id", &record.id, MAX_PROGRAM_ID_BYTES, true)?;
        out.u8(state_tag(record.state));
        out.u8(match record.kind {
            None => 0,
            Some(ProgramKind::Permission) => 1,
            Some(ProgramKind::Question) => 2,
            Some(ProgramKind::Auth) => 3,
        });
        out.u8(record.progress.unwrap_or(u8::MAX));
        out.text("program app", &record.app, MAX_PROGRAM_APP_BYTES, true)?;
        out.text(
            "program title",
            &record.title,
            MAX_PROGRAM_TITLE_BYTES,
            true,
        )?;
        out.text(
            "program message",
            &record.message,
            MAX_PROGRAM_MESSAGE_BYTES,
            true,
        )?;
        out.u64(record.updated);
        out.u64(record.created);
        out.u64(record.changed);
        out.u8(record.previous.map_or(0, state_tag));
        out.u64(record.updated_age_ms);
        out.u64(record.changed_age_ms);
    }
    Ok(())
}

fn decode_program_status(input: &mut Decoder<'_>) -> Result<ProgramStatus> {
    let count = input.u16()?;
    let background = input.u8()?;
    let update = input.u64()?;
    let terminal_title = input.text("terminal title", MAX_TERMINAL_TITLE_BYTES, true)?;
    let omitted = input.u16()?;
    let included = usize::from(count.checked_sub(omitted).ok_or(Error::InvalidValue {
        field: "program record counts",
    })?);
    validate_program_counts(count, usize::from(omitted), included)?;
    let omitted = (0..omitted)
        .map(|_| input.u64())
        .collect::<Result<Vec<_>>>()?;
    let mut records = Vec::with_capacity(included);
    for _ in 0..included {
        let id = input.text("program id", MAX_PROGRAM_ID_BYTES, true)?;
        let state = decode_state(input.u8()?)?;
        let kind = match input.u8()? {
            0 => None,
            1 => Some(ProgramKind::Permission),
            2 => Some(ProgramKind::Question),
            3 => Some(ProgramKind::Auth),
            value => {
                return Err(Error::InvalidTag {
                    field: "program kind",
                    value,
                });
            }
        };
        if kind.is_some() && state != ProgramState::Blocked {
            return Err(Error::InvalidValue {
                field: "program kind",
            });
        }
        let progress = match input.u8()? {
            u8::MAX => None,
            progress @ 0..=100 => Some(progress),
            _ => return Err(Error::InvalidValue { field: "progress" }),
        };
        records.push(ProgramRecord {
            id,
            state,
            kind,
            progress,
            app: input.text("program app", MAX_PROGRAM_APP_BYTES, true)?,
            title: input.text("program title", MAX_PROGRAM_TITLE_BYTES, true)?,
            message: input.text("program message", MAX_PROGRAM_MESSAGE_BYTES, true)?,
            updated: input.u64()?,
            created: input.u64()?,
            changed: input.u64()?,
            previous: match input.u8()? {
                0 => None,
                tag => Some(decode_state(tag)?),
            },
            updated_age_ms: input.u64()?,
            changed_age_ms: input.u64()?,
        });
    }
    Ok(ProgramStatus {
        count,
        background,
        update,
        terminal_title,
        omitted,
        records,
    })
}

fn validate_program_counts(count: u16, omitted: usize, included: usize) -> Result<()> {
    if count > MAX_PROGRAM_RECORDS || omitted + included != usize::from(count) {
        return Err(Error::InvalidValue {
            field: "program record counts",
        });
    }
    Ok(())
}

fn state_tag(state: ProgramState) -> u8 {
    match state {
        ProgramState::Idle => 1,
        ProgramState::Working => 2,
        ProgramState::Done => 3,
        ProgramState::Blocked => 4,
        ProgramState::Error => 5,
    }
}

fn decode_state(tag: u8) -> Result<ProgramState> {
    Ok(match tag {
        1 => ProgramState::Idle,
        2 => ProgramState::Working,
        3 => ProgramState::Done,
        4 => ProgramState::Blocked,
        5 => ProgramState::Error,
        value => {
            return Err(Error::InvalidTag {
                field: "program state",
                value,
            });
        }
    })
}

fn encode(
    magic: [u8; 4],
    tag: u8,
    maximum: usize,
    payload: &dyn Fn(&mut Encoder) -> Result<()>,
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

fn framed_len(bytes: &[u8], magic: [u8; 4], bound: fn(u8) -> usize) -> Result<Option<usize>> {
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
    let maximum = bound(bytes[6]);
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

fn decode_frame(bytes: &[u8], magic: [u8; 4], bound: fn(u8) -> usize) -> Result<(u8, Decoder<'_>)> {
    let total = framed_len(bytes, magic, bound)?.ok_or(Error::Truncated)?;
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

    /// Bytes behind a 32-bit length, for values a caller already bounded.
    fn long_bytes(&mut self, value: &[u8]) {
        self.u32(u32::try_from(value.len()).expect("bounded value fits u32"));
        self.bytes.extend_from_slice(value);
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

    fn long_bytes(&mut self, field: &'static str, maximum: usize) -> Result<Vec<u8>> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(Error::ValueTooLarge {
                field,
                size: length,
                maximum,
            });
        }
        Ok(self.take(length)?.to_vec())
    }

    fn long_text(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        String::from_utf8(self.long_bytes(field, maximum)?)
            .map_err(|_| Error::InvalidUtf8 { field })
    }

    /// The bytes left, which the caller takes whole.
    fn rest(&mut self) -> &'a [u8] {
        let rest = &self.bytes[self.offset..];
        self.offset = self.bytes.len();
        rest
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
