#![forbid(unsafe_code)]

//! Eon-owned EONW workspace protocol: the versioned codec, and the Reserved
//! Stop, whose bytes and the six-byte prefix of every message (`EONW` and
//! the little-endian version) no revision changes.

pub mod stop;
pub mod v37;
