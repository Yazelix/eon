//! The Reserved Stop (EON-C11): one request and its two replies, whose bytes
//! no EONW revision changes, so any build stops any window.
//!
//! Each message has the shape of an EONW header (`EONW`, little-endian
//! version, kind, zero flags, little-endian payload length) at version `0`,
//! which no EONW revision uses, so every supervisor recognises it before it
//! checks its own version, and every older one reads it whole and answers in
//! its own framing. A string is a little-endian `u16` byte length and its
//! bytes. This module shares no code with the versioned codecs: a later
//! revision's edit must never reach these bytes.

use std::{fmt, str};

/// The bytes before every EONW message's payload, in every revision.
pub const HEADER_BYTES: usize = 12;
/// The one length of a Stop request: the header and the generation.
pub const REQUEST_BYTES: usize = HEADER_BYTES + 2 + GENERATION_BYTES;
/// The largest reply this module encodes or decodes.
pub const MAX_REPLY_BYTES: usize = HEADER_BYTES + 64 * 1024;
/// The most terminals one Stopped reply names.
pub const MAX_TERMINALS: usize = 256;

/// The first EONW revision whose supervisors answer the Reserved Stop; a
/// supervisor of an older revision answers it in its own framing.
pub const FIRST_VERSION: u16 = 37;
const MAGIC: &[u8; 4] = b"EONW";
const RESERVED_VERSION: u16 = 0;
const REQUEST: u8 = 1;
const FAILURE: u8 = 130;
const STOPPED: u8 = 132;
/// `g1-` and 32 lowercase hex digits.
const GENERATION_BYTES: usize = 35;
const MAX_ID_BYTES: usize = 128;
const MAX_DETAIL_BYTES: usize = 1024;

/// A supervisor's answer to a Stop request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The generation stopped, and the IDs of the terminals it stopped.
    Stopped {
        generation: String,
        terminals: Vec<String>,
    },
    Failure {
        code: String,
        detail: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    TrailingBytes { count: usize },
    InvalidHeader,
    InvalidKind { value: u8 },
    InvalidLength { length: usize },
    InvalidValue { field: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated reserved Stop message"),
            Self::TrailingBytes { count } => {
                write!(
                    formatter,
                    "reserved Stop message has {count} trailing bytes"
                )
            }
            Self::InvalidHeader => formatter.write_str("invalid reserved Stop header"),
            Self::InvalidKind { value } => {
                write!(formatter, "invalid reserved Stop message kind {value}")
            }
            Self::InvalidLength { length } => {
                write!(formatter, "invalid reserved Stop message length {length}")
            }
            Self::InvalidValue { field } => write!(formatter, "invalid reserved Stop {field}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The EONW version that a message's first six bytes name, whatever its
/// revision, else none when they do not start an EONW message.
pub fn version(header: &[u8]) -> Option<u16> {
    match header {
        [b'E', b'O', b'N', b'W', low, high, ..] => Some(u16::from_le_bytes([*low, *high])),
        _ => None,
    }
}

/// Whether `header` starts a Reserved Stop message rather than a versioned one.
pub fn is_reserved(header: &[u8]) -> bool {
    version(header) == Some(RESERVED_VERSION)
}

/// The whole length of the Stop request that complete header `header` starts.
pub fn request_len(header: &[u8]) -> Result<usize> {
    let length = header_len(header, REQUEST)?;
    if length != REQUEST_BYTES {
        return Err(Error::InvalidLength { length });
    }
    Ok(length)
}

/// The whole length of the reply that complete header `header` starts, at
/// most [`MAX_REPLY_BYTES`].
pub fn reply_len(header: &[u8]) -> Result<usize> {
    let kind = *header.get(6).ok_or(Error::Truncated)?;
    if !matches!(kind, STOPPED | FAILURE) {
        return Err(Error::InvalidKind { value: kind });
    }
    let length = header_len(header, kind)?;
    if length > MAX_REPLY_BYTES {
        return Err(Error::InvalidLength { length });
    }
    Ok(length)
}

pub fn encode_request(generation: &str) -> Result<Vec<u8>> {
    validate_generation(generation)?;
    let mut payload = Vec::with_capacity(2 + GENERATION_BYTES);
    string(&mut payload, generation);
    Ok(frame(REQUEST, payload))
}

/// The generation that Stop request `bytes` names.
pub fn decode_request(bytes: &[u8]) -> Result<String> {
    request_len(bytes)?;
    let mut decoder = Decoder::new(unframe(bytes, REQUEST)?);
    let generation = decoder.string("generation", GENERATION_BYTES)?;
    validate_generation(&generation)?;
    decoder.finish()?;
    Ok(generation)
}

pub fn encode_reply(reply: &Reply) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match reply {
        Reply::Stopped {
            generation,
            terminals,
        } => {
            validate_generation(generation)?;
            if terminals.len() > MAX_TERMINALS {
                return Err(Error::InvalidValue { field: "terminals" });
            }
            string(&mut payload, generation);
            payload.extend_from_slice(&(terminals.len() as u16).to_le_bytes());
            for terminal in terminals {
                validate_id("terminal", terminal)?;
                string(&mut payload, terminal);
            }
            STOPPED
        }
        Reply::Failure { code, detail } => {
            validate_id("failure code", code)?;
            validate_detail(detail)?;
            string(&mut payload, code);
            string(&mut payload, detail);
            FAILURE
        }
    };
    Ok(frame(kind, payload))
}

pub fn decode_reply(bytes: &[u8]) -> Result<Reply> {
    reply_len(bytes)?;
    let kind = bytes[6];
    let mut decoder = Decoder::new(unframe(bytes, kind)?);
    let reply = if kind == STOPPED {
        let generation = decoder.string("generation", GENERATION_BYTES)?;
        validate_generation(&generation)?;
        let count = usize::from(decoder.number()?);
        if count > MAX_TERMINALS {
            return Err(Error::InvalidValue { field: "terminals" });
        }
        let terminals = (0..count)
            .map(|_| {
                let terminal = decoder.string("terminal", MAX_ID_BYTES)?;
                validate_id("terminal", &terminal)?;
                Ok(terminal)
            })
            .collect::<Result<_>>()?;
        Reply::Stopped {
            generation,
            terminals,
        }
    } else {
        let code = decoder.string("failure code", MAX_ID_BYTES)?;
        validate_id("failure code", &code)?;
        let detail = decoder.string("failure detail", MAX_DETAIL_BYTES)?;
        validate_detail(&detail)?;
        Reply::Failure { code, detail }
    };
    decoder.finish()?;
    Ok(reply)
}

fn header_len(header: &[u8], kind: u8) -> Result<usize> {
    if header.len() < HEADER_BYTES {
        return Err(Error::Truncated);
    }
    if !is_reserved(header) || header[7] != 0 {
        return Err(Error::InvalidHeader);
    }
    if header[6] != kind {
        return Err(Error::InvalidKind { value: header[6] });
    }
    let payload = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    Ok(HEADER_BYTES.saturating_add(payload as usize))
}

fn frame(kind: u8, payload: Vec<u8>) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER_BYTES + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&RESERVED_VERSION.to_le_bytes());
    message.push(kind);
    message.push(0);
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&payload);
    message
}

/// The payload of complete message `bytes`, whose header names `kind`.
fn unframe(bytes: &[u8], kind: u8) -> Result<&[u8]> {
    let declared = header_len(bytes, kind)?;
    if bytes.len() < declared {
        return Err(Error::Truncated);
    }
    if bytes.len() > declared {
        return Err(Error::TrailingBytes {
            count: bytes.len() - declared,
        });
    }
    Ok(&bytes[HEADER_BYTES..])
}

fn string(payload: &mut Vec<u8>, value: &str) {
    payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
    payload.extend_from_slice(value.as_bytes());
}

fn validate_generation(value: &str) -> Result<()> {
    let valid = value.len() == GENERATION_BYTES
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidValue {
            field: "generation",
        })
    }
}

fn validate_id(field: &'static str, value: &str) -> Result<()> {
    let valid = (1..=MAX_ID_BYTES).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidValue { field })
    }
}

fn validate_detail(value: &str) -> Result<()> {
    if (1..=MAX_DETAIL_BYTES).contains(&value.len()) {
        Ok(())
    } else {
        Err(Error::InvalidValue {
            field: "failure detail",
        })
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < count {
            return Err(Error::Truncated);
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn number(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn string(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        let length = usize::from(self.number()?);
        if length > maximum {
            return Err(Error::InvalidValue { field });
        }
        let bytes = self.take(length)?;
        str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidValue { field })
    }

    fn finish(&self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(Error::TrailingBytes {
                count: self.bytes.len(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATION: &str = "g1-0123456789abcdef0123456789abcdef";

    fn generation_bytes() -> Vec<u8> {
        let mut bytes = vec![0x23, 0x00];
        bytes.extend_from_slice(GENERATION.as_bytes());
        bytes
    }

    /// The Reserved Stop's bytes (EON-C11). These literals never change: a
    /// failure here means the change breaks every other build's Stop.
    #[test]
    fn reserved_stop_keeps_its_bytes() {
        let mut request = vec![
            0x45, 0x4f, 0x4e, 0x57, 0x00, 0x00, 0x01, 0x00, 0x25, 0x00, 0x00, 0x00,
        ];
        request.extend(generation_bytes());
        assert_eq!(request.len(), 49);
        assert_eq!(encode_request(GENERATION).unwrap(), request);
        assert_eq!(decode_request(&request).unwrap(), GENERATION);
        assert_eq!(request_len(&request[..HEADER_BYTES]), Ok(49));

        let stopped = Reply::Stopped {
            generation: GENERATION.into(),
            terminals: vec!["terminal-1".into(), "t.2_x".into()],
        };
        let mut bytes = vec![
            0x45, 0x4f, 0x4e, 0x57, 0x00, 0x00, 0x84, 0x00, 0x3a, 0x00, 0x00, 0x00,
        ];
        bytes.extend(generation_bytes());
        bytes.extend([0x02, 0x00, 0x0a, 0x00]);
        bytes.extend(b"terminal-1");
        bytes.extend([0x05, 0x00]);
        bytes.extend(b"t.2_x");
        assert_eq!(encode_reply(&stopped).unwrap(), bytes);
        assert_eq!(decode_reply(&bytes).unwrap(), stopped);
        assert_eq!(reply_len(&bytes[..HEADER_BYTES]), Ok(bytes.len()));

        let failure = Reply::Failure {
            code: "generation-mismatch".into(),
            detail: "not ours".into(),
        };
        let mut bytes = vec![
            0x45, 0x4f, 0x4e, 0x57, 0x00, 0x00, 0x82, 0x00, 0x1f, 0x00, 0x00, 0x00, 0x13, 0x00,
        ];
        bytes.extend(b"generation-mismatch");
        bytes.extend([0x08, 0x00]);
        bytes.extend(b"not ours");
        assert_eq!(encode_reply(&failure).unwrap(), bytes);
        assert_eq!(decode_reply(&bytes).unwrap(), failure);

        // Every revision's first six bytes name its version.
        assert_eq!(version(b"EONW\x25\x00\x82\x00"), Some(37));
        assert_eq!(version(&request), Some(0));
        assert_eq!(version(b"EONX\x00\x00"), None);
        assert!(is_reserved(&request) && !is_reserved(b"EONW\x25\x00"));
    }

    #[test]
    fn reserved_stop_rejects_forged_bounds() {
        let request = encode_request(GENERATION).unwrap();
        let forged = |edit: &dyn Fn(&mut Vec<u8>)| {
            let mut bytes = request.clone();
            edit(&mut bytes);
            decode_request(&bytes)
        };
        assert_eq!(
            forged(&|bytes| bytes[4] = 37),
            Err(Error::InvalidHeader),
            "a versioned header"
        );
        assert_eq!(forged(&|bytes| bytes[7] = 1), Err(Error::InvalidHeader));
        assert_eq!(
            forged(&|bytes| bytes[6] = STOPPED),
            Err(Error::InvalidKind { value: STOPPED })
        );
        assert_eq!(
            forged(&|bytes| bytes.push(0)),
            Err(Error::TrailingBytes { count: 1 })
        );
        assert_eq!(forged(&|bytes| bytes.truncate(48)), Err(Error::Truncated));
        assert_eq!(
            forged(&|bytes| bytes[8] = 0xff),
            Err(Error::InvalidLength { length: 267 })
        );
        assert_eq!(
            forged(&|bytes| bytes[14] = b'G'),
            Err(Error::InvalidValue {
                field: "generation"
            })
        );
        for generation in ["", "g1-0123", "g1-0123456789ABCDEF0123456789abcdef"] {
            assert!(encode_request(generation).is_err(), "{generation}");
        }

        let terminals = vec!["t".to_owned(); MAX_TERMINALS + 1];
        let too_many = Reply::Stopped {
            generation: GENERATION.into(),
            terminals,
        };
        assert!(encode_reply(&too_many).is_err());
        let mut bytes = encode_reply(&Reply::Stopped {
            generation: GENERATION.into(),
            terminals: Vec::new(),
        })
        .unwrap();
        // A count over the bound, whatever follows it.
        let count = HEADER_BYTES + 2 + GENERATION_BYTES;
        bytes[count..count + 2].copy_from_slice(&257u16.to_le_bytes());
        assert_eq!(
            decode_reply(&bytes),
            Err(Error::InvalidValue { field: "terminals" })
        );
        for (code, detail) in [("", "x"), ("a b", "x"), ("code", "")] {
            let failure = Reply::Failure {
                code: code.into(),
                detail: detail.into(),
            };
            assert!(encode_reply(&failure).is_err(), "{code:?} {detail:?}");
        }
        let oversized = Reply::Failure {
            code: "c".into(),
            detail: "x".repeat(MAX_DETAIL_BYTES + 1),
        };
        assert!(encode_reply(&oversized).is_err());
        let mut header = encode_reply(&Reply::Failure {
            code: "c".into(),
            detail: "x".into(),
        })
        .unwrap();
        header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            reply_len(&header),
            Err(Error::InvalidLength { .. })
        ));
        assert_eq!(
            reply_len(&request),
            Err(Error::InvalidKind { value: REQUEST })
        );
    }
}
