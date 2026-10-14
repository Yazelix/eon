//! Versioned, bounded messages for one local Orbit client session.

use std::fmt;

use crate::{Frame, MAX_FRAME_BYTES, Row};

mod codec;
pub use codec::{
    client_message_len, decode_client_message, decode_server_message, encode_client_message,
    encode_server_message, server_message_len,
};

#[cfg(test)]
mod tests;

/// Local-session framing discriminator.
pub const MAGIC: &[u8; 4] = b"ORBS";
/// The only local-session revision understood by this package.
pub const VERSION: u16 = 13;
/// Fixed bytes before a message payload.
pub const HEADER_BYTES: usize = 12;
/// Largest payload accepted by the local-session decoder.
pub const MAX_PAYLOAD_BYTES: usize = 2 * MAX_FRAME_BYTES + 13;
/// Largest paste accepted as one semantic event.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;
/// Largest copied plain text returned as one semantic result.
pub const MAX_COPY_BYTES: usize = 1024 * 1024;
/// Largest text associated with one key event.
pub const MAX_KEY_TEXT_BYTES: usize = 4096;
/// Largest client-visible failure detail.
pub const MAX_FAILURE_BYTES: usize = 1024;
/// Largest complete title/CWD metadata payload.
pub const MAX_METADATA_BYTES: usize = 8 * 1024;
/// Largest absolute whole-row distance accepted in one viewport commit.
pub const MAX_SCROLL_ROWS: i16 = 1024;

/// A local-session validation or framing failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The message does not start with [`MAGIC`].
    InvalidMagic,
    /// The framing revision is unsupported.
    UnsupportedVersion { version: u16 },
    /// Reserved header flags are set.
    InvalidFlags { value: u8 },
    /// A payload exceeds its message-specific bound.
    PayloadTooLarge { size: usize, maximum: usize },
    /// The message ends before its declared payload does.
    Truncated,
    /// A complete message is followed by undeclared bytes.
    TrailingBytes { count: usize },
    /// A tag is unknown or invalid for the field being decoded.
    InvalidTag { field: &'static str, value: u8 },
    /// A non-tag scalar value is invalid for its field.
    InvalidValue { field: &'static str },
    /// A UTF-8 field is malformed.
    InvalidUtf8 { field: &'static str },
    /// Consumed modifiers are not a subset of active modifiers.
    ConsumedModifiersNotActive,
    /// Mouse coordinates are outside the terminal mapper's supported range.
    InvalidCoordinates,
    /// Surface dimensions are empty, inconsistent, or too large.
    InvalidSurfaceSize,
    /// An embedded ORBF frame is invalid.
    Frame(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("invalid local-session message magic"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported local-session version {version}")
            }
            Self::InvalidFlags { value } => {
                write!(formatter, "invalid local-session flags {value:#x}")
            }
            Self::PayloadTooLarge { size, maximum } => {
                write!(
                    formatter,
                    "message payload is {size} bytes; maximum is {maximum}"
                )
            }
            Self::Truncated => formatter.write_str("truncated local-session message"),
            Self::TrailingBytes { count } => {
                write!(
                    formatter,
                    "local-session message has {count} trailing bytes"
                )
            }
            Self::InvalidTag { field, value } => {
                write!(formatter, "invalid local-session {field} tag {value}")
            }
            Self::InvalidValue { field } => {
                write!(formatter, "invalid local-session {field}")
            }
            Self::InvalidUtf8 { field } => {
                write!(formatter, "local-session {field} is not UTF-8")
            }
            Self::ConsumedModifiersNotActive => {
                formatter.write_str("consumed key modifiers are not active")
            }
            Self::InvalidCoordinates => formatter.write_str("invalid mouse coordinates"),
            Self::InvalidSurfaceSize => formatter.write_str("invalid surface size"),
            Self::Frame(error) => write!(formatter, "invalid presentation frame: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Frame(error) => Some(error),
            _ => None,
        }
    }
}

impl From<crate::Error> for Error {
    fn from(value: crate::Error) -> Self {
        Self::Frame(value)
    }
}

/// Local-session codec result.
pub type Result<T> = std::result::Result<T, Error>;

/// A client-to-Orbit semantic message.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    /// Opens a session using the exact revision in the ORBS header.
    Hello,
    /// Opens one read-only title/CWD metadata observation.
    ObserveMetadata,
    /// One physical key event and its text meaning.
    Key(KeyEvent),
    /// One pointer event in surface pixels.
    Mouse(MouseEvent),
    /// A native focus transition.
    Focus(FocusEvent),
    /// Opaque pasted bytes; embedded newlines and NULs are preserved.
    Paste(Vec<u8>),
    /// A requested terminal and drawing-surface size.
    Resize(SurfaceSize),
    /// One routed left-pointer or copy action.
    Selection(SelectionAction),
    /// Preview current authority from a non-future presented frame revision.
    PreviewVertical {
        frame_revision: u64,
        direction: VerticalDirection,
    },
    /// Commit from a non-future presented revision; negative is toward older history.
    ScrollVertical { frame_revision: u64, rows: i16 },
    /// Return the primary viewport to live output without sending terminal input.
    ReturnToLive,
}

/// An Orbit-to-client session message.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    /// The client owns the single attachment at this exact ORBS revision.
    Attached,
    /// The client owns the single read-only metadata observation.
    ObservingMetadata,
    /// One authoritative title/CWD observation at a presentation revision.
    Metadata(Metadata),
    /// Another client already owns the single attachment.
    Busy,
    /// One canonical, complete Orbit presentation frame.
    Frame(Box<Frame>),
    /// Orbit accepted a semantic event.
    Accepted,
    /// Orbit finished one routed left-pointer sequence.
    SelectionFinished {
        /// The authoritative frame revision when the sequence finished.
        frame_revision: u64,
    },
    /// Orbit rejected a message without inventing terminal input.
    Failure(Failure),
    /// The authoritative child process exited.
    Exited { code: i32 },
    /// One complete bounded plain-text copy result and its native destination.
    CopiedText {
        location: ClipboardLocation,
        text: String,
    },
    /// One ordered terminal-emitted plain-text clipboard write.
    ClipboardWrite {
        location: ClipboardLocation,
        text: String,
    },
    /// Read-only routing and bounded-row result for a vertical preview.
    VerticalPreview(VerticalPreview),
    /// Typed result of one accepted vertical wheel event.
    WheelOutcome(WheelOutcome),
    /// Typed result of one bounded vertical viewport commit.
    ScrollOutcome(ScrollOutcome),
}

/// One bounded, read-only terminal metadata observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub revision: u64,
    pub title: String,
    pub working_directory: String,
}

/// One row direction in the authoritative scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalDirection {
    Up,
    Down,
}

/// One revision-bound, read-only vertical preview result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerticalPreview {
    pub frame_revision: u64,
    pub direction: VerticalDirection,
    pub outcome: PreviewOutcome,
}

/// How Orbit would route a vertical wheel from the previewed frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewOutcome {
    TerminalRouted,
    Viewport {
        cols: u16,
        edge_reached: bool,
        /// Canonical rows ordered nearest-first in the requested direction.
        rows: Vec<Row>,
    },
}

/// Authoritative result of one accepted vertical wheel event.
#[derive(Clone, Debug, PartialEq)]
pub enum WheelOutcome {
    TerminalRouted,
    Viewport { applied_rows: i8, frame: Box<Frame> },
}

/// Authoritative result of one bounded vertical viewport commit.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollOutcome {
    TerminalOwned {
        requested_rows: i16,
    },
    Viewport {
        requested_rows: i16,
        applied_rows: i16,
        frame: Box<Frame>,
        next: PreviewOutcome,
    },
}

/// Platform-neutral native clipboard destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardLocation {
    Standard,
    Selection,
    Primary,
}

/// Physical key action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
    Repeat,
}

/// Stable physical key identity matching Orbit's terminal-aware mapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalKey(u16);

impl PhysicalKey {
    /// Highest physical-key identity in local-session v1.
    pub const MAX_RAW: u16 = 175;
    pub const UNIDENTIFIED: Self = Self(0);
    pub const BACKQUOTE: Self = Self(1);
    pub const BACKSLASH: Self = Self(2);
    pub const BRACKET_LEFT: Self = Self(3);
    pub const BRACKET_RIGHT: Self = Self(4);
    pub const COMMA: Self = Self(5);
    pub const DIGIT_0: Self = Self(6);
    pub const DIGIT_1: Self = Self(7);
    pub const DIGIT_2: Self = Self(8);
    pub const DIGIT_3: Self = Self(9);
    pub const DIGIT_4: Self = Self(10);
    pub const DIGIT_5: Self = Self(11);
    pub const DIGIT_6: Self = Self(12);
    pub const DIGIT_7: Self = Self(13);
    pub const DIGIT_8: Self = Self(14);
    pub const DIGIT_9: Self = Self(15);
    pub const EQUAL: Self = Self(16);
    pub const INTL_BACKSLASH: Self = Self(17);
    pub const INTL_RO: Self = Self(18);
    pub const INTL_YEN: Self = Self(19);
    pub const A: Self = Self(20);
    pub const B: Self = Self(21);
    pub const C: Self = Self(22);
    pub const D: Self = Self(23);
    pub const E: Self = Self(24);
    pub const F: Self = Self(25);
    pub const G: Self = Self(26);
    pub const H: Self = Self(27);
    pub const I: Self = Self(28);
    pub const J: Self = Self(29);
    pub const K: Self = Self(30);
    pub const L: Self = Self(31);
    pub const M: Self = Self(32);
    pub const N: Self = Self(33);
    pub const O: Self = Self(34);
    pub const P: Self = Self(35);
    pub const Q: Self = Self(36);
    pub const R: Self = Self(37);
    pub const S: Self = Self(38);
    pub const T: Self = Self(39);
    pub const U: Self = Self(40);
    pub const V: Self = Self(41);
    pub const W: Self = Self(42);
    pub const X: Self = Self(43);
    pub const Y: Self = Self(44);
    pub const Z: Self = Self(45);
    pub const MINUS: Self = Self(46);
    pub const PERIOD: Self = Self(47);
    pub const QUOTE: Self = Self(48);
    pub const SEMICOLON: Self = Self(49);
    pub const SLASH: Self = Self(50);
    pub const ALT_LEFT: Self = Self(51);
    pub const ALT_RIGHT: Self = Self(52);
    pub const BACKSPACE: Self = Self(53);
    pub const CAPS_LOCK: Self = Self(54);
    pub const CONTEXT_MENU: Self = Self(55);
    pub const CONTROL_LEFT: Self = Self(56);
    pub const CONTROL_RIGHT: Self = Self(57);
    pub const ENTER: Self = Self(58);
    pub const META_LEFT: Self = Self(59);
    pub const META_RIGHT: Self = Self(60);
    pub const SHIFT_LEFT: Self = Self(61);
    pub const SHIFT_RIGHT: Self = Self(62);
    pub const SPACE: Self = Self(63);
    pub const TAB: Self = Self(64);
    pub const CONVERT: Self = Self(65);
    pub const KANA_MODE: Self = Self(66);
    pub const NON_CONVERT: Self = Self(67);
    pub const DELETE: Self = Self(68);
    pub const END: Self = Self(69);
    pub const HELP: Self = Self(70);
    pub const HOME: Self = Self(71);
    pub const INSERT: Self = Self(72);
    pub const PAGE_DOWN: Self = Self(73);
    pub const PAGE_UP: Self = Self(74);
    pub const ARROW_DOWN: Self = Self(75);
    pub const ARROW_LEFT: Self = Self(76);
    pub const ARROW_RIGHT: Self = Self(77);
    pub const ARROW_UP: Self = Self(78);
    pub const NUM_LOCK: Self = Self(79);
    pub const NUMPAD_0: Self = Self(80);
    pub const NUMPAD_1: Self = Self(81);
    pub const NUMPAD_2: Self = Self(82);
    pub const NUMPAD_3: Self = Self(83);
    pub const NUMPAD_4: Self = Self(84);
    pub const NUMPAD_5: Self = Self(85);
    pub const NUMPAD_6: Self = Self(86);
    pub const NUMPAD_7: Self = Self(87);
    pub const NUMPAD_8: Self = Self(88);
    pub const NUMPAD_9: Self = Self(89);
    pub const NUMPAD_ADD: Self = Self(90);
    pub const NUMPAD_BACKSPACE: Self = Self(91);
    pub const NUMPAD_CLEAR: Self = Self(92);
    pub const NUMPAD_CLEAR_ENTRY: Self = Self(93);
    pub const NUMPAD_COMMA: Self = Self(94);
    pub const NUMPAD_DECIMAL: Self = Self(95);
    pub const NUMPAD_DIVIDE: Self = Self(96);
    pub const NUMPAD_ENTER: Self = Self(97);
    pub const NUMPAD_EQUAL: Self = Self(98);
    pub const NUMPAD_MEMORY_ADD: Self = Self(99);
    pub const NUMPAD_MEMORY_CLEAR: Self = Self(100);
    pub const NUMPAD_MEMORY_RECALL: Self = Self(101);
    pub const NUMPAD_MEMORY_STORE: Self = Self(102);
    pub const NUMPAD_MEMORY_SUBTRACT: Self = Self(103);
    pub const NUMPAD_MULTIPLY: Self = Self(104);
    pub const NUMPAD_PAREN_LEFT: Self = Self(105);
    pub const NUMPAD_PAREN_RIGHT: Self = Self(106);
    pub const NUMPAD_SUBTRACT: Self = Self(107);
    pub const NUMPAD_SEPARATOR: Self = Self(108);
    pub const NUMPAD_UP: Self = Self(109);
    pub const NUMPAD_DOWN: Self = Self(110);
    pub const NUMPAD_RIGHT: Self = Self(111);
    pub const NUMPAD_LEFT: Self = Self(112);
    pub const NUMPAD_BEGIN: Self = Self(113);
    pub const NUMPAD_HOME: Self = Self(114);
    pub const NUMPAD_END: Self = Self(115);
    pub const NUMPAD_INSERT: Self = Self(116);
    pub const NUMPAD_DELETE: Self = Self(117);
    pub const NUMPAD_PAGE_UP: Self = Self(118);
    pub const NUMPAD_PAGE_DOWN: Self = Self(119);
    pub const ESCAPE: Self = Self(120);
    pub const F1: Self = Self(121);
    pub const F2: Self = Self(122);
    pub const F3: Self = Self(123);
    pub const F4: Self = Self(124);
    pub const F5: Self = Self(125);
    pub const F6: Self = Self(126);
    pub const F7: Self = Self(127);
    pub const F8: Self = Self(128);
    pub const F9: Self = Self(129);
    pub const F10: Self = Self(130);
    pub const F11: Self = Self(131);
    pub const F12: Self = Self(132);
    pub const F13: Self = Self(133);
    pub const F14: Self = Self(134);
    pub const F15: Self = Self(135);
    pub const F16: Self = Self(136);
    pub const F17: Self = Self(137);
    pub const F18: Self = Self(138);
    pub const F19: Self = Self(139);
    pub const F20: Self = Self(140);
    pub const F21: Self = Self(141);
    pub const F22: Self = Self(142);
    pub const F23: Self = Self(143);
    pub const F24: Self = Self(144);
    pub const F25: Self = Self(145);
    pub const FN: Self = Self(146);
    pub const FN_LOCK: Self = Self(147);
    pub const PRINT_SCREEN: Self = Self(148);
    pub const SCROLL_LOCK: Self = Self(149);
    pub const PAUSE: Self = Self(150);
    pub const BROWSER_BACK: Self = Self(151);
    pub const BROWSER_FAVORITES: Self = Self(152);
    pub const BROWSER_FORWARD: Self = Self(153);
    pub const BROWSER_HOME: Self = Self(154);
    pub const BROWSER_REFRESH: Self = Self(155);
    pub const BROWSER_SEARCH: Self = Self(156);
    pub const BROWSER_STOP: Self = Self(157);
    pub const EJECT: Self = Self(158);
    pub const LAUNCH_APP_1: Self = Self(159);
    pub const LAUNCH_APP_2: Self = Self(160);
    pub const LAUNCH_MAIL: Self = Self(161);
    pub const MEDIA_PLAY_PAUSE: Self = Self(162);
    pub const MEDIA_SELECT: Self = Self(163);
    pub const MEDIA_STOP: Self = Self(164);
    pub const MEDIA_TRACK_NEXT: Self = Self(165);
    pub const MEDIA_TRACK_PREVIOUS: Self = Self(166);
    pub const POWER: Self = Self(167);
    pub const SLEEP: Self = Self(168);
    pub const AUDIO_VOLUME_DOWN: Self = Self(169);
    pub const AUDIO_VOLUME_MUTE: Self = Self(170);
    pub const AUDIO_VOLUME_UP: Self = Self(171);
    pub const WAKE_UP: Self = Self(172);
    pub const COPY: Self = Self(173);
    pub const CUT: Self = Self(174);
    pub const PASTE: Self = Self(175);

    /// Constructs a physical key if it belongs to the v1 identity set.
    pub const fn from_raw(value: u16) -> Option<Self> {
        if value <= Self::MAX_RAW {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns the stable v1 identity.
    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// Modifier bits, including lock and left/right-side information.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(u16);

impl Modifiers {
    pub const SHIFT: Self = Self(1);
    pub const CTRL: Self = Self(2);
    pub const ALT: Self = Self(4);
    pub const SUPER: Self = Self(8);
    pub const CAPS_LOCK: Self = Self(16);
    pub const NUM_LOCK: Self = Self(32);
    pub const SHIFT_SIDE: Self = Self(64);
    pub const CTRL_SIDE: Self = Self(128);
    pub const ALT_SIDE: Self = Self(256);
    pub const SUPER_SIDE: Self = Self(512);
    pub const MASK: u16 = 1023;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(value: u16) -> Option<Self> {
        let valid_sides = (value & Self::SHIFT_SIDE.0 == 0 || value & Self::SHIFT.0 != 0)
            && (value & Self::CTRL_SIDE.0 == 0 || value & Self::CTRL.0 != 0)
            && (value & Self::ALT_SIDE.0 == 0 || value & Self::ALT.0 != 0)
            && (value & Self::SUPER_SIDE.0 == 0 || value & Self::SUPER.0 != 0);
        if value & !Self::MASK == 0 && valid_sides {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Complete semantic information for one key event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub key: PhysicalKey,
    pub modifiers: Modifiers,
    pub consumed_modifiers: Modifiers,
    pub composing: bool,
    pub text: Option<String>,
    pub unshifted_codepoint: Option<char>,
}

/// Mouse action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// Mouse button identity. Four through Seven are wheel directions and are
/// valid only with [`MouseAction::Press`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Unknown,
    Left,
    Middle,
    Right,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Eleven,
}

/// Pointer event in surface pixels within the terminal mapper's `u16` domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    /// `None` is valid only for motion with no pressed button.
    pub button: Option<MouseButton>,
    pub modifiers: Modifiers,
    pub x: f32,
    pub y: f32,
}

/// Native focus transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEvent {
    Gained,
    Lost,
}

/// Terminal grid and drawing-surface measurements within the mapper's `u16` pixel domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceSize {
    pub cols: u16,
    pub rows: u16,
    pub screen_width: u32,
    pub screen_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub padding_left: u32,
    pub padding_right: u32,
}

/// Pointer position in surface pixels within the terminal mapper's `u16` domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionPosition {
    pub x: f32,
    pub y: f32,
}

/// Routed left-pointer gestures and explicit copy actions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectionAction {
    /// Press against the exact complete frame the client used for hit testing.
    Begin {
        frame_revision: u64,
        position: SelectionPosition,
        /// Monotonic event-delivery time since the client-defined origin.
        time_ns: u64,
        modifiers: Modifiers,
    },
    /// Update the active left-pointer sequence.
    Update {
        position: SelectionPosition,
        modifiers: Modifiers,
    },
    /// Finish the active left-pointer sequence.
    Finish {
        position: SelectionPosition,
        modifiers: Modifiers,
    },
    /// Advance one row toward older history while extending the active host selection.
    AutoscrollUp { position: SelectionPosition },
    /// Cancel active gesture state and selected presentation.
    Cancel,
    /// Request the last successfully frozen plain text.
    Copy,
}

/// Stable class of client-visible failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCode {
    InvalidInput,
    Protocol,
    Terminal,
}

/// Bounded client-visible failure detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: FailureCode,
    pub detail: String,
}
