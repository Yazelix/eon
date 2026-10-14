#![forbid(unsafe_code)]

//! Canonical owned values and codec for Orbit presentation frames.
//!
//! This crate deliberately contains no terminal, transport, renderer, or
//! platform integration. Orbit maps authoritative terminal state into a
//! [`Frame`], and clients decode those same values at an exact pinned Orbit
//! revision.

use std::{fmt, str};

pub mod management;
pub mod session;

/// Maximum decoded cell count accepted by ORBF v2.
pub const MAX_CELLS: usize = 100_000;
/// Maximum encoded ORBF v2 payload size.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// ORBF frame discriminator.
pub const MAGIC: &[u8; 4] = b"ORBF";
/// The only protocol revision accepted by this package.
pub const VERSION: u16 = 2;
/// Number of RGB entries in an ORBF v2 palette.
pub const PALETTE_LEN: usize = 256;

const ROW_FLAG_MASK: u8 = 0b0000_0111;
const STYLE_FLAG_MASK: u16 = 0b0000_0011_1111_1111;
const STRING_LENGTH_BYTES: usize = std::mem::size_of::<u32>();
const FRAME_FIXED_BYTES: usize = 817;
const ROW_FIXED_BYTES: usize = 1;
const CELL_FIXED_BYTES: usize = 16;
const MIN_FRAME_BYTES: usize =
    FRAME_FIXED_BYTES + ROW_FIXED_BYTES + CELL_FIXED_BYTES + 4 * STRING_LENGTH_BYTES;

/// A protocol validation or revision-ordering failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The payload exceeds [`MAX_FRAME_BYTES`].
    FrameTooLarge { size: usize },
    /// The payload is not an ORBF frame.
    InvalidMagic,
    /// The payload declares an unsupported ORBF revision.
    UnsupportedVersion { version: u16 },
    /// The payload ends before a declared value is complete.
    Truncated,
    /// A complete frame is followed by undeclared bytes.
    TrailingBytes { count: usize },
    /// The dimensions are empty, overflow, or exceed [`MAX_CELLS`].
    InvalidDimensions { cols: u16, rows: u16 },
    /// A string field is not UTF-8.
    InvalidUtf8 { field: &'static str },
    /// A tagged field has an unknown or noncanonical value.
    InvalidTag { field: &'static str, value: u8 },
    /// Reserved flag bits are set.
    InvalidFlags { field: &'static str, value: u16 },
    /// An owned frame does not match its declared dimensions.
    InvalidShape { field: &'static str },
    /// A string cannot be represented by the ORBF v2 length prefix.
    StringTooLong { field: &'static str, length: usize },
    /// A complete frame revision is not strictly newer than the current one.
    RevisionNotNewer { current: u64, incoming: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge { size } => write!(
                formatter,
                "presentation frame is {size} bytes; maximum is {MAX_FRAME_BYTES}"
            ),
            Self::InvalidMagic => formatter.write_str("invalid presentation frame magic"),
            Self::UnsupportedVersion { version } => {
                write!(
                    formatter,
                    "unsupported presentation frame version {version}"
                )
            }
            Self::Truncated => formatter.write_str("truncated presentation frame"),
            Self::TrailingBytes { count } => {
                write!(formatter, "presentation frame has {count} trailing bytes")
            }
            Self::InvalidDimensions { cols, rows } => {
                write!(formatter, "invalid presentation dimensions {cols}x{rows}")
            }
            Self::InvalidUtf8 { field } => {
                write!(formatter, "presentation {field} is not UTF-8")
            }
            Self::InvalidTag { field, value } => {
                write!(formatter, "invalid presentation {field} tag {value}")
            }
            Self::InvalidFlags { field, value } => {
                write!(formatter, "invalid presentation {field} flags {value:#x}")
            }
            Self::InvalidShape { field } => {
                write!(formatter, "presentation frame has invalid {field} shape")
            }
            Self::StringTooLong { field, length } => {
                write!(
                    formatter,
                    "presentation {field} is too long: {length} bytes"
                )
            }
            Self::RevisionNotNewer { current, incoming } => write!(
                formatter,
                "presentation revision {incoming} is not newer than {current}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// ORBF operations return one concrete, dependency-free error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Terminal viewport dimensions in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub cols: u16,
    pub rows: u16,
}

/// The active authoritative terminal screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Primary,
    Alternate,
}

/// Capabilities carried by this complete frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Must be true in ORBF v2: frames carry hyperlink metadata.
    pub hyperlinks: bool,
    /// Must be false in ORBF v2: Kitty graphics are unsupported.
    pub kitty_graphics: bool,
}

/// An RGB color with eight-bit channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
}

/// Frame-wide colors and the active 256-color palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; PALETTE_LEN],
}

/// Cursor rendering shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    Block,
    Underline,
    BlockHollow,
}

/// Cursor position relative to the current viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorViewport {
    pub x: u16,
    pub y: u16,
    pub at_wide_tail: bool,
}

/// Complete cursor presentation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub visible: bool,
    pub blinking: bool,
    pub password_input: bool,
    pub shape: CursorShape,
    pub viewport: Option<CursorViewport>,
}

/// A cell-relative color reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleColor {
    None,
    Palette(u8),
    Rgb(Rgb),
}

/// Underline presentation style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// Complete presentation style for one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellStyle {
    pub foreground: StyleColor,
    pub background: StyleColor,
    pub underline_color: StyleColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub selected: bool,
    pub protected: bool,
    pub underline: Underline,
}

/// Width role of a cell in the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellWidth {
    Narrow,
    Wide,
    SpacerTail,
    SpacerHead,
}

/// One decoded presentation cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub width: CellWidth,
    pub style: CellStyle,
    pub text: String,
    pub hyperlink: String,
}

/// One decoded viewport row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub wrapped: bool,
    pub wrap_continuation: bool,
    pub kitty_virtual_placeholder: bool,
    pub cells: Vec<Cell>,
}

/// Authoritative position in current wrapped display rows, not logical lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollPosition {
    /// Distance from live bottom, bounded by `history_rows`; zero means live.
    pub rows_from_live: u64,
    /// Retained history excluding the active viewport. Both counts are zero
    /// on the alternate screen and may change after output, pruning or reflow.
    pub history_rows: u64,
}

/// One complete ORBF v2 presentation revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub revision: u64,
    pub dimensions: Dimensions,
    pub screen: Screen,
    pub title: String,
    pub working_directory: String,
    pub capabilities: Capabilities,
    pub colors: Colors,
    pub cursor: Cursor,
    pub scroll_position: ScrollPosition,
    pub rows: Vec<Row>,
}

impl Frame {
    /// Encode this frame as canonical ORBF v2 bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_frame(self)
    }
}

/// Incremental ORBF v2 byte accounting for bounded frame construction.
///
/// Producers use this before retaining variable-sized strings in an owned
/// [`Frame`]. The final value is also checked by [`encode_frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSize {
    bytes: usize,
}

impl FrameSize {
    /// Start accounting for a frame header and its variable metadata.
    pub fn new(
        title: &str,
        working_directory: &str,
        has_cursor_color: bool,
        has_cursor_viewport: bool,
    ) -> Result<Self> {
        let title = encoded_string_size("title", title)?;
        let working_directory = encoded_string_size("working directory", working_directory)?;
        let bytes = FRAME_FIXED_BYTES
            .checked_add(title)
            .and_then(|size| size.checked_add(working_directory))
            .and_then(|size| size.checked_add(usize::from(has_cursor_color) * 3))
            .and_then(|size| size.checked_add(usize::from(has_cursor_viewport) * 5))
            .ok_or(Error::FrameTooLarge { size: usize::MAX })?;
        Self::from_bytes(bytes)
    }

    /// Reserve one encoded row header.
    pub fn add_row(&mut self) -> Result<()> {
        self.add(ROW_FIXED_BYTES)
    }

    /// Reserve one encoded cell before retaining its strings.
    pub fn add_cell(&mut self, text: &str, hyperlink: &str) -> Result<()> {
        let text = encoded_string_size("cell text", text)?;
        let hyperlink = encoded_string_size("cell hyperlink", hyperlink)?;
        let size = CELL_FIXED_BYTES
            .checked_add(text)
            .and_then(|value| value.checked_add(hyperlink))
            .ok_or(Error::FrameTooLarge { size: usize::MAX })?;
        self.add(size)
    }

    /// Return the exact encoded size accounted so far.
    pub fn bytes(self) -> usize {
        self.bytes
    }

    fn from_bytes(bytes: usize) -> Result<Self> {
        if bytes > MAX_FRAME_BYTES {
            Err(Error::FrameTooLarge { size: bytes })
        } else {
            Ok(Self { bytes })
        }
    }

    fn add(&mut self, value: usize) -> Result<()> {
        let bytes = self
            .bytes
            .checked_add(value)
            .ok_or(Error::FrameTooLarge { size: usize::MAX })?;
        *self = Self::from_bytes(bytes)?;
        Ok(())
    }
}

/// Retains frames with valid capabilities, scroll position, cell-width topology,
/// and strictly increasing revisions.
#[derive(Debug, Default)]
pub struct FrameReducer {
    current: Option<Frame>,
}

impl FrameReducer {
    /// Return the accepted current frame, if any.
    pub fn current(&self) -> Option<&Frame> {
        self.current.as_ref()
    }

    /// Validate canonical capabilities, scroll position and cell widths before admission.
    pub fn push(&mut self, frame: Frame) -> Result<&Frame> {
        validate_capabilities(frame.capabilities)?;
        validate_scroll_position(frame.scroll_position, frame.screen)?;
        for row in &frame.rows {
            validate_cell_width_topology(row)?;
        }
        if let Some(current) = &self.current
            && frame.revision <= current.revision
        {
            return Err(Error::RevisionNotNewer {
                current: current.revision,
                incoming: frame.revision,
            });
        }
        self.current = Some(frame);
        Ok(self.current.as_ref().expect("frame was stored"))
    }

    /// Decode and accept one complete ORBF v2 frame.
    pub fn decode_and_push(&mut self, bytes: &[u8]) -> Result<&Frame> {
        self.push(decode_frame(bytes)?)
    }

    /// Consume the reducer and return its current frame.
    pub fn into_current(self) -> Option<Frame> {
        self.current
    }
}

/// Encode one owned frame as canonical ORBF v2 bytes.
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>> {
    let encoded_size = validate_frame(frame)?;
    let mut encoder = Encoder(Vec::with_capacity(encoded_size));
    encoder.bytes(MAGIC)?;
    encoder.u16(VERSION)?;
    encoder.u64(frame.revision)?;
    encoder.u16(frame.dimensions.cols)?;
    encoder.u16(frame.dimensions.rows)?;
    encoder.u8(screen_tag(frame.screen))?;
    encoder.string("title", &frame.title)?;
    encoder.string("working directory", &frame.working_directory)?;
    encoder.boolean(frame.capabilities.hyperlinks)?;
    encoder.boolean(frame.capabilities.kitty_graphics)?;
    encoder.rgb(frame.colors.background)?;
    encoder.rgb(frame.colors.foreground)?;
    encoder.boolean(frame.colors.cursor.is_some())?;
    if let Some(cursor) = frame.colors.cursor {
        encoder.rgb(cursor)?;
    }
    for color in frame.colors.palette {
        encoder.rgb(color)?;
    }
    encoder.boolean(frame.cursor.visible)?;
    encoder.boolean(frame.cursor.blinking)?;
    encoder.boolean(frame.cursor.password_input)?;
    encoder.u8(cursor_shape_tag(frame.cursor.shape))?;
    encoder.boolean(frame.cursor.viewport.is_some())?;
    if let Some(cursor) = frame.cursor.viewport {
        encoder.u16(cursor.x)?;
        encoder.u16(cursor.y)?;
        encoder.boolean(cursor.at_wide_tail)?;
    }
    encoder.u64(frame.scroll_position.rows_from_live)?;
    encoder.u64(frame.scroll_position.history_rows)?;

    for row in &frame.rows {
        encode_row(&mut encoder, row)?;
    }
    debug_assert_eq!(encoder.0.len(), encoded_size);
    Ok(encoder.0)
}

/// Decode and strictly validate one complete ORBF v2 payload.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(Error::FrameTooLarge { size: bytes.len() });
    }
    let mut decoder = Decoder { bytes, position: 0 };
    if decoder.take(MAGIC.len())? != MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != VERSION {
        return Err(Error::UnsupportedVersion { version });
    }

    let revision = decoder.u64()?;
    let dimensions = Dimensions {
        cols: decoder.u16()?,
        rows: decoder.u16()?,
    };
    validate_dimensions(dimensions)?;
    let screen = decode_screen(decoder.u8()?)?;
    let title = decoder.string("title")?;
    let working_directory = decoder.string("working directory")?;
    let capabilities = Capabilities {
        hyperlinks: decoder.boolean("hyperlink capability")?,
        kitty_graphics: decoder.boolean("Kitty graphics capability")?,
    };
    validate_capabilities(capabilities)?;
    let background = decoder.rgb()?;
    let foreground = decoder.rgb()?;
    let color_cursor = if decoder.boolean("cursor color presence")? {
        Some(decoder.rgb()?)
    } else {
        None
    };
    let mut palette = [Rgb::BLACK; PALETTE_LEN];
    for color in &mut palette {
        *color = decoder.rgb()?;
    }
    let cursor = Cursor {
        visible: decoder.boolean("cursor visibility")?,
        blinking: decoder.boolean("cursor blinking")?,
        password_input: decoder.boolean("cursor password input")?,
        shape: decode_cursor_shape(decoder.u8()?)?,
        viewport: if decoder.boolean("cursor viewport presence")? {
            let viewport = CursorViewport {
                x: decoder.u16()?,
                y: decoder.u16()?,
                at_wide_tail: decoder.boolean("cursor wide-tail state")?,
            };
            if viewport.x >= dimensions.cols || viewport.y >= dimensions.rows {
                return Err(Error::InvalidShape {
                    field: "cursor viewport",
                });
            }
            Some(viewport)
        } else {
            None
        },
    };

    let scroll_position = ScrollPosition {
        rows_from_live: decoder.u64()?,
        history_rows: decoder.u64()?,
    };
    validate_scroll_position(scroll_position, screen)?;
    let mut rows = Vec::with_capacity(usize::from(dimensions.rows));
    for _ in 0..dimensions.rows {
        rows.push(decode_row(&mut decoder, dimensions.cols)?);
    }
    if decoder.position != bytes.len() {
        return Err(Error::TrailingBytes {
            count: bytes.len() - decoder.position,
        });
    }
    Ok(Frame {
        revision,
        dimensions,
        screen,
        title,
        working_directory,
        capabilities,
        colors: Colors {
            background,
            foreground,
            cursor: color_cursor,
            palette,
        },
        cursor,
        scroll_position,
        rows,
    })
}

#[cfg(test)]
pub(crate) fn encode_canonical_row(row: &Row, cols: u16) -> Result<Vec<u8>> {
    encode_canonical_rows(std::slice::from_ref(row), cols)
}

pub(crate) fn encode_canonical_rows(rows: &[Row], cols: u16) -> Result<Vec<u8>> {
    validate_row_columns(cols)?;
    validate_row_count(rows.len(), cols)?;
    let mut size = FrameSize::from_bytes(0)?;
    for row in rows {
        validate_row(row, cols)?;
        size.add_row()?;
        for cell in &row.cells {
            size.add_cell(&cell.text, &cell.hyperlink)?;
        }
    }
    let mut encoder = Encoder(Vec::with_capacity(size.bytes()));
    for row in rows {
        encode_row(&mut encoder, row)?;
    }
    Ok(encoder.0)
}

#[cfg(test)]
pub(crate) fn decode_canonical_row(bytes: &[u8], cols: u16) -> Result<Row> {
    Ok(decode_canonical_rows(bytes, cols, 1)?
        .pop()
        .expect("one canonical row was requested"))
}

pub(crate) fn decode_canonical_rows(bytes: &[u8], cols: u16, row_count: u16) -> Result<Vec<Row>> {
    validate_row_columns(cols)?;
    validate_row_count(usize::from(row_count), cols)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(Error::FrameTooLarge { size: bytes.len() });
    }
    let mut decoder = Decoder { bytes, position: 0 };
    let mut rows = Vec::with_capacity(usize::from(row_count));
    for _ in 0..row_count {
        rows.push(decode_row(&mut decoder, cols)?);
    }
    if decoder.position != bytes.len() {
        return Err(Error::TrailingBytes {
            count: bytes.len() - decoder.position,
        });
    }
    Ok(rows)
}

fn encode_row(encoder: &mut Encoder, row: &Row) -> Result<()> {
    let flags = u8::from(row.wrapped)
        | (u8::from(row.wrap_continuation) << 1)
        | (u8::from(row.kitty_virtual_placeholder) << 2);
    encoder.u8(flags)?;
    for cell in &row.cells {
        encoder.u8(cell_width_tag(cell.width))?;
        encoder.style(cell.style)?;
        encoder.string("cell text", &cell.text)?;
        encoder.string("cell hyperlink", &cell.hyperlink)?;
    }
    Ok(())
}

fn decode_row(decoder: &mut Decoder<'_>, cols: u16) -> Result<Row> {
    let flags = decoder.u8()?;
    if flags & !ROW_FLAG_MASK != 0 {
        return Err(Error::InvalidFlags {
            field: "row",
            value: u16::from(flags),
        });
    }
    let mut cells = Vec::with_capacity(usize::from(cols));
    for _ in 0..cols {
        cells.push(Cell {
            width: decode_cell_width(decoder.u8()?)?,
            style: decoder.style()?,
            text: decoder.string("cell text")?,
            hyperlink: decoder.string("cell hyperlink")?,
        });
    }
    let row = Row {
        wrapped: flags & 1 != 0,
        wrap_continuation: flags & (1 << 1) != 0,
        kitty_virtual_placeholder: flags & (1 << 2) != 0,
        cells,
    };
    validate_cell_width_topology(&row)?;
    Ok(row)
}

fn validate_row_columns(cols: u16) -> Result<()> {
    if cols == 0 || usize::from(cols) > MAX_CELLS {
        Err(Error::InvalidDimensions { cols, rows: 1 })
    } else {
        Ok(())
    }
}

fn validate_row_count(rows: usize, cols: u16) -> Result<()> {
    let cells = rows.checked_mul(usize::from(cols));
    if matches!(cells, Some(count) if count <= MAX_CELLS) {
        Ok(())
    } else {
        Err(Error::InvalidDimensions {
            cols,
            rows: u16::try_from(rows).unwrap_or(u16::MAX),
        })
    }
}

fn validate_row(row: &Row, cols: u16) -> Result<()> {
    validate_row_columns(cols)?;
    if row.cells.len() != usize::from(cols) {
        return Err(Error::InvalidShape { field: "cell" });
    }
    validate_cell_width_topology(row)
}

fn validate_cell_width_topology(row: &Row) -> Result<()> {
    for (index, cell) in row.cells.iter().enumerate() {
        let valid = match cell.width {
            CellWidth::Narrow => true,
            CellWidth::Wide => row
                .cells
                .get(index + 1)
                .is_some_and(|cell| cell.width == CellWidth::SpacerTail),
            CellWidth::SpacerTail => index > 0 && row.cells[index - 1].width == CellWidth::Wide,
            CellWidth::SpacerHead => index > 0 && index + 1 == row.cells.len() && row.wrapped,
        };
        if !valid {
            return Err(Error::InvalidShape {
                field: "cell width topology",
            });
        }
    }
    Ok(())
}

fn validate_dimensions(dimensions: Dimensions) -> Result<usize> {
    let count = usize::from(dimensions.cols)
        .checked_mul(usize::from(dimensions.rows))
        .ok_or(Error::InvalidDimensions {
            cols: dimensions.cols,
            rows: dimensions.rows,
        })?;
    if count == 0 || count > MAX_CELLS {
        return Err(Error::InvalidDimensions {
            cols: dimensions.cols,
            rows: dimensions.rows,
        });
    }
    Ok(count)
}

fn validate_frame(frame: &Frame) -> Result<usize> {
    validate_dimensions(frame.dimensions)?;
    validate_capabilities(frame.capabilities)?;
    validate_scroll_position(frame.scroll_position, frame.screen)?;
    if frame.rows.len() != usize::from(frame.dimensions.rows) {
        return Err(Error::InvalidShape { field: "row" });
    }
    if frame.cursor.viewport.is_some_and(|cursor| {
        cursor.x >= frame.dimensions.cols || cursor.y >= frame.dimensions.rows
    }) {
        return Err(Error::InvalidShape {
            field: "cursor viewport",
        });
    }

    let mut size = FrameSize::new(
        &frame.title,
        &frame.working_directory,
        frame.colors.cursor.is_some(),
        frame.cursor.viewport.is_some(),
    )?;
    for row in &frame.rows {
        validate_row(row, frame.dimensions.cols)?;
        size.add_row()?;
        for cell in &row.cells {
            size.add_cell(&cell.text, &cell.hyperlink)?;
        }
    }
    Ok(size.bytes())
}

fn validate_scroll_position(position: ScrollPosition, screen: Screen) -> Result<()> {
    if position.rows_from_live > position.history_rows
        || (screen == Screen::Alternate && position.history_rows != 0)
    {
        Err(Error::InvalidShape {
            field: "scroll position",
        })
    } else {
        Ok(())
    }
}

fn validate_capabilities(capabilities: Capabilities) -> Result<()> {
    if !capabilities.hyperlinks || capabilities.kitty_graphics {
        Err(Error::InvalidShape {
            field: "capabilities",
        })
    } else {
        Ok(())
    }
}

fn encoded_string_size(field: &'static str, value: &str) -> Result<usize> {
    u32::try_from(value.len()).map_err(|_| Error::StringTooLong {
        field,
        length: value.len(),
    })?;
    STRING_LENGTH_BYTES
        .checked_add(value.len())
        .ok_or(Error::FrameTooLarge { size: usize::MAX })
}

fn screen_tag(screen: Screen) -> u8 {
    match screen {
        Screen::Primary => 0,
        Screen::Alternate => 1,
    }
}

fn decode_screen(tag: u8) -> Result<Screen> {
    match tag {
        0 => Ok(Screen::Primary),
        1 => Ok(Screen::Alternate),
        value => Err(Error::InvalidTag {
            field: "screen",
            value,
        }),
    }
}

fn cursor_shape_tag(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Bar => 0,
        CursorShape::Block => 1,
        CursorShape::Underline => 2,
        CursorShape::BlockHollow => 3,
    }
}

fn decode_cursor_shape(tag: u8) -> Result<CursorShape> {
    match tag {
        0 => Ok(CursorShape::Bar),
        1 => Ok(CursorShape::Block),
        2 => Ok(CursorShape::Underline),
        3 => Ok(CursorShape::BlockHollow),
        value => Err(Error::InvalidTag {
            field: "cursor shape",
            value,
        }),
    }
}

fn cell_width_tag(width: CellWidth) -> u8 {
    match width {
        CellWidth::Narrow => 0,
        CellWidth::Wide => 1,
        CellWidth::SpacerTail => 2,
        CellWidth::SpacerHead => 3,
    }
}

fn decode_cell_width(tag: u8) -> Result<CellWidth> {
    match tag {
        0 => Ok(CellWidth::Narrow),
        1 => Ok(CellWidth::Wide),
        2 => Ok(CellWidth::SpacerTail),
        3 => Ok(CellWidth::SpacerHead),
        value => Err(Error::InvalidTag {
            field: "cell width",
            value,
        }),
    }
}

fn underline_tag(underline: Underline) -> u8 {
    match underline {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Curly => 3,
        Underline::Dotted => 4,
        Underline::Dashed => 5,
    }
}

fn decode_underline(tag: u8) -> Result<Underline> {
    match tag {
        0 => Ok(Underline::None),
        1 => Ok(Underline::Single),
        2 => Ok(Underline::Double),
        3 => Ok(Underline::Curly),
        4 => Ok(Underline::Dotted),
        5 => Ok(Underline::Dashed),
        value => Err(Error::InvalidTag {
            field: "underline",
            value,
        }),
    }
}

struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let size = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or(Error::FrameTooLarge { size: usize::MAX })?;
        if size > MAX_FRAME_BYTES {
            return Err(Error::FrameTooLarge { size });
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }

    fn boolean(&mut self, value: bool) -> Result<()> {
        self.u8(u8::from(value))
    }

    fn u8(&mut self, value: u8) -> Result<()> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn string(&mut self, field: &'static str, value: &str) -> Result<()> {
        let length = u32::try_from(value.len()).map_err(|_| Error::StringTooLong {
            field,
            length: value.len(),
        })?;
        self.bytes(&length.to_le_bytes())?;
        self.bytes(value.as_bytes())
    }

    fn rgb(&mut self, color: Rgb) -> Result<()> {
        self.bytes(&[color.r, color.g, color.b])
    }

    fn style(&mut self, style: CellStyle) -> Result<()> {
        self.style_color(style.foreground)?;
        self.style_color(style.background)?;
        self.style_color(style.underline_color)?;
        let flags = u16::from(style.bold)
            | (u16::from(style.italic) << 1)
            | (u16::from(style.faint) << 2)
            | (u16::from(style.blink) << 3)
            | (u16::from(style.inverse) << 4)
            | (u16::from(style.invisible) << 5)
            | (u16::from(style.strikethrough) << 6)
            | (u16::from(style.overline) << 7)
            | (u16::from(style.selected) << 8)
            | (u16::from(style.protected) << 9);
        self.u16(flags)?;
        self.u8(underline_tag(style.underline))
    }

    fn style_color(&mut self, color: StyleColor) -> Result<()> {
        let (tag, value) = match color {
            StyleColor::None => (0, [0, 0, 0]),
            StyleColor::Palette(index) => (1, [index, 0, 0]),
            StyleColor::Rgb(color) => (2, [color.r, color.g, color.b]),
        };
        self.u8(tag)?;
        self.bytes(&value)
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(length).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.position..end).ok_or(Error::Truncated)?;
        self.position = end;
        Ok(value)
    }

    fn boolean(&mut self, field: &'static str) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(Error::InvalidTag { field, value }),
        }
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("slice has two bytes"),
        ))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("slice has four bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("slice has eight bytes"),
        ))
    }

    fn string(&mut self, field: &'static str) -> Result<String> {
        let length = usize::try_from(self.u32()?).expect("u32 fits usize");
        let value = str::from_utf8(self.take(length)?).map_err(|_| Error::InvalidUtf8 { field })?;
        Ok(value.to_owned())
    }

    fn rgb(&mut self) -> Result<Rgb> {
        let bytes = self.take(3)?;
        Ok(Rgb {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
        })
    }

    fn style(&mut self) -> Result<CellStyle> {
        let foreground = self.style_color("foreground color")?;
        let background = self.style_color("background color")?;
        let underline_color = self.style_color("underline color")?;
        let flags = self.u16()?;
        if flags & !STYLE_FLAG_MASK != 0 {
            return Err(Error::InvalidFlags {
                field: "cell style",
                value: flags,
            });
        }
        Ok(CellStyle {
            foreground,
            background,
            underline_color,
            bold: flags & 1 != 0,
            italic: flags & (1 << 1) != 0,
            faint: flags & (1 << 2) != 0,
            blink: flags & (1 << 3) != 0,
            inverse: flags & (1 << 4) != 0,
            invisible: flags & (1 << 5) != 0,
            strikethrough: flags & (1 << 6) != 0,
            overline: flags & (1 << 7) != 0,
            selected: flags & (1 << 8) != 0,
            protected: flags & (1 << 9) != 0,
            underline: decode_underline(self.u8()?)?,
        })
    }

    fn style_color(&mut self, field: &'static str) -> Result<StyleColor> {
        let tag = self.u8()?;
        let value: [u8; 3] = self.take(3)?.try_into().expect("slice has three bytes");
        match tag {
            0 if value == [0, 0, 0] => Ok(StyleColor::None),
            1 if value[1..] == [0, 0] => Ok(StyleColor::Palette(value[0])),
            2 => Ok(StyleColor::Rgb(Rgb {
                r: value[0],
                g: value[1],
                b: value[2],
            })),
            value => Err(Error::InvalidTag { field, value }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN_OFFSET: usize = 18;
    const TITLE_LENGTH_OFFSET: usize = 19;
    const HYPERLINK_CAPABILITY_OFFSET: usize = 27;
    const KITTY_GRAPHICS_CAPABILITY_OFFSET: usize = 28;
    const CURSOR_COLOR_PRESENCE_OFFSET: usize = 35;
    const CURSOR_SHAPE_OFFSET: usize = 807;
    const CURSOR_VIEWPORT_PRESENCE_OFFSET: usize = 808;
    const ROW_FLAGS_OFFSET: usize = 825;
    const CELL_WIDTH_OFFSET: usize = 826;
    const FOREGROUND_TAG_OFFSET: usize = 827;
    const STYLE_FLAGS_OFFSET: usize = 839;
    const UNDERLINE_OFFSET: usize = 841;

    fn plain_style() -> CellStyle {
        CellStyle {
            foreground: StyleColor::None,
            background: StyleColor::None,
            underline_color: StyleColor::None,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected: false,
            protected: false,
            underline: Underline::None,
        }
    }

    fn minimal_frame(revision: u64) -> Frame {
        Frame {
            revision,
            dimensions: Dimensions { cols: 1, rows: 1 },
            screen: Screen::Primary,
            scroll_position: ScrollPosition::default(),
            title: String::new(),
            working_directory: String::new(),
            capabilities: Capabilities {
                hyperlinks: true,
                kitty_graphics: false,
            },
            colors: Colors {
                background: Rgb::BLACK,
                foreground: Rgb::BLACK,
                cursor: None,
                palette: [Rgb::BLACK; PALETTE_LEN],
            },
            cursor: Cursor {
                visible: false,
                blinking: false,
                password_input: false,
                shape: CursorShape::Bar,
                viewport: None,
            },
            rows: vec![Row {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: vec![Cell {
                    width: CellWidth::Narrow,
                    style: plain_style(),
                    text: String::new(),
                    hyperlink: String::new(),
                }],
            }],
        }
    }

    fn frame_with_widths(revision: u64, wrapped: bool, widths: &[CellWidth]) -> Frame {
        let mut frame = minimal_frame(revision);
        frame.dimensions.cols = u16::try_from(widths.len()).expect("test row width fits u16");
        frame.rows[0].wrapped = wrapped;
        let cell = frame.rows[0].cells[0].clone();
        frame.rows[0].cells = widths
            .iter()
            .map(|width| Cell {
                width: *width,
                ..cell.clone()
            })
            .collect();
        frame
    }

    fn rich_frame() -> Frame {
        let mut palette = [Rgb::BLACK; PALETTE_LEN];
        for (index, color) in palette.iter_mut().enumerate() {
            let index = u8::try_from(index).expect("palette index");
            *color = Rgb {
                r: index,
                g: index.wrapping_mul(3),
                b: u8::MAX.wrapping_sub(index),
            };
        }
        let decorated = CellStyle {
            foreground: StyleColor::Rgb(Rgb {
                r: 12,
                g: 34,
                b: 56,
            }),
            background: StyleColor::Palette(17),
            underline_color: StyleColor::Rgb(Rgb { r: 7, g: 8, b: 9 }),
            bold: true,
            italic: true,
            faint: true,
            blink: true,
            inverse: true,
            invisible: true,
            strikethrough: true,
            overline: true,
            selected: true,
            protected: true,
            underline: Underline::Curly,
        };
        Frame {
            revision: 42,
            dimensions: Dimensions { cols: 2, rows: 2 },
            screen: Screen::Alternate,
            scroll_position: ScrollPosition::default(),
            title: "rich 🪐".into(),
            working_directory: "file:///tmp/orbit".into(),
            capabilities: Capabilities {
                hyperlinks: true,
                kitty_graphics: false,
            },
            colors: Colors {
                background: Rgb { r: 1, g: 2, b: 3 },
                foreground: Rgb { r: 4, g: 5, b: 6 },
                cursor: Some(Rgb { r: 7, g: 8, b: 9 }),
                palette,
            },
            cursor: Cursor {
                visible: true,
                blinking: true,
                password_input: true,
                shape: CursorShape::BlockHollow,
                viewport: Some(CursorViewport {
                    x: 1,
                    y: 1,
                    at_wide_tail: true,
                }),
            },
            rows: vec![
                Row {
                    wrapped: true,
                    wrap_continuation: false,
                    kitty_virtual_placeholder: true,
                    cells: vec![
                        Cell {
                            width: CellWidth::Narrow,
                            style: decorated,
                            text: "e\u{301}".into(),
                            hyperlink: "https://example.test".into(),
                        },
                        Cell {
                            width: CellWidth::SpacerHead,
                            style: decorated,
                            text: String::new(),
                            hyperlink: String::new(),
                        },
                    ],
                },
                Row {
                    wrapped: false,
                    wrap_continuation: true,
                    kitty_virtual_placeholder: false,
                    cells: vec![
                        Cell {
                            width: CellWidth::Wide,
                            style: decorated,
                            text: "界".into(),
                            hyperlink: String::new(),
                        },
                        Cell {
                            width: CellWidth::SpacerTail,
                            style: decorated,
                            text: String::new(),
                            hyperlink: String::new(),
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn scroll_position_is_canonical_at_every_acceptance_boundary() {
        let mut canonical = minimal_frame(0);
        canonical.scroll_position = ScrollPosition {
            rows_from_live: 4,
            history_rows: 20,
        };
        let bytes = canonical.encode().unwrap();
        assert_eq!(decode_frame(&bytes).unwrap(), canonical);
        let mut reducer = FrameReducer::default();
        reducer.push(canonical.clone()).unwrap();
        let error = Error::InvalidShape {
            field: "scroll position",
        };
        for (screen, position) in [
            (
                Screen::Primary,
                ScrollPosition {
                    rows_from_live: 21,
                    history_rows: 20,
                },
            ),
            (
                Screen::Primary,
                ScrollPosition {
                    rows_from_live: u64::MAX,
                    history_rows: 0,
                },
            ),
            (Screen::Alternate, canonical.scroll_position),
            (
                Screen::Alternate,
                ScrollPosition {
                    rows_from_live: 0,
                    history_rows: 20,
                },
            ),
        ] {
            let mut invalid = canonical.clone();
            invalid.revision = 1;
            invalid.screen = screen;
            invalid.scroll_position = position;
            assert_eq!(invalid.encode().unwrap_err(), error);
            assert_eq!(reducer.push(invalid).unwrap_err(), error);
            assert_eq!(reducer.current(), Some(&canonical));
            let mut invalid = bytes.clone();
            invalid[SCREEN_OFFSET] = screen_tag(screen);
            invalid[809..817].copy_from_slice(&position.rows_from_live.to_le_bytes());
            invalid[817..825].copy_from_slice(&position.history_rows.to_le_bytes());
            assert_eq!(decode_frame(&invalid).unwrap_err(), error);
        }
        for screen in [Screen::Primary, Screen::Alternate] {
            let mut frame = minimal_frame(1);
            frame.screen = screen;
            assert_eq!(decode_frame(&frame.encode().unwrap()).unwrap(), frame);
        }
        let mut old = bytes;
        old[4..6].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            decode_frame(&old),
            Err(Error::UnsupportedVersion { version: 1 })
        );
    }

    #[test]
    fn rich_frame_round_trips_byte_for_byte() {
        let expected = rich_frame();
        let bytes = encode_frame(&expected).unwrap();
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), VERSION);
        let decoded = decode_frame(&bytes).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(encode_frame(&decoded).unwrap(), bytes);
    }

    #[test]
    fn orbf_v2_capabilities_are_truthful_at_every_acceptance_boundary() {
        let canonical = minimal_frame(0);
        let error = Error::InvalidShape {
            field: "capabilities",
        };
        let mut reducer = FrameReducer::default();
        reducer.push(canonical.clone()).unwrap();
        let bytes = canonical.encode().unwrap();
        for (hyperlinks, kitty_graphics) in [(false, false), (false, true), (true, true)] {
            let mut frame = canonical.clone();
            frame.revision = 1;
            frame.capabilities = Capabilities {
                hyperlinks,
                kitty_graphics,
            };
            assert_eq!(frame.encode().unwrap_err(), error);
            assert_eq!(reducer.push(frame).unwrap_err(), error);
            assert_eq!(reducer.current(), Some(&canonical));

            let mut invalid = bytes.clone();
            invalid[HYPERLINK_CAPABILITY_OFFSET] = u8::from(hyperlinks);
            invalid[KITTY_GRAPHICS_CAPABILITY_OFFSET] = u8::from(kitty_graphics);
            assert_eq!(decode_frame(&invalid).unwrap_err(), error);
        }
        assert_eq!(decode_frame(&bytes).unwrap(), canonical);
    }

    #[test]
    fn orbf_v2_cell_width_topology_is_validated_at_every_acceptance_boundary() {
        use CellWidth::{Narrow, SpacerHead, SpacerTail, Wide};

        let canonical = frame_with_widths(0, false, &[Narrow; 4]);
        let error = Error::InvalidShape {
            field: "cell width topology",
        };
        let invalid: &[(bool, &[CellWidth])] = &[
            (false, &[Wide, Wide, Narrow, Narrow]),
            (false, &[Wide, Narrow, Narrow, Narrow]),
            (false, &[Narrow, Narrow, Narrow, Wide]),
            (false, &[SpacerTail, Narrow, Narrow, Narrow]),
            (false, &[Narrow, SpacerTail, Narrow, Narrow]),
            (true, &[Narrow, SpacerHead, Narrow, Narrow]),
            (false, &[Narrow, Narrow, Narrow, SpacerHead]),
            (true, &[SpacerHead]),
        ];
        let cell_bytes = CELL_FIXED_BYTES + 2 * STRING_LENGTH_BYTES;
        let mut reducer = FrameReducer::default();
        reducer.push(canonical.clone()).unwrap();

        for &(wrapped, widths) in invalid {
            let frame = frame_with_widths(1, wrapped, widths);
            let cols = frame.dimensions.cols;
            assert_eq!(frame.encode().unwrap_err(), error);
            assert_eq!(
                encode_canonical_row(&frame.rows[0], cols).unwrap_err(),
                error
            );
            assert_eq!(reducer.push(frame).unwrap_err(), error);
            assert_eq!(reducer.current(), Some(&canonical));

            let mut invalid_bytes = frame_with_widths(0, false, &vec![Narrow; widths.len()])
                .encode()
                .unwrap();
            invalid_bytes[ROW_FLAGS_OFFSET] = u8::from(wrapped);
            for (index, &width) in widths.iter().enumerate() {
                invalid_bytes[CELL_WIDTH_OFFSET + index * cell_bytes] = cell_width_tag(width);
            }
            assert_eq!(decode_frame(&invalid_bytes).unwrap_err(), error);
            assert_eq!(
                decode_canonical_row(&invalid_bytes[ROW_FLAGS_OFFSET..], cols).unwrap_err(),
                error
            );
        }

        for frame in [
            canonical,
            frame_with_widths(1, false, &[Wide, SpacerTail, Narrow, Narrow]),
            frame_with_widths(2, true, &[Narrow, Narrow, Narrow, SpacerHead]),
        ] {
            let bytes = frame.encode().unwrap();
            assert_eq!(decode_frame(&bytes).unwrap(), frame);
            let row = encode_canonical_row(&frame.rows[0], frame.dimensions.cols).unwrap();
            assert_eq!(
                decode_canonical_row(&row, frame.dimensions.cols).unwrap(),
                frame.rows[0]
            );
        }
    }

    #[test]
    fn incremental_size_matches_encoding_and_rejects_oversized_cells() {
        let frame = rich_frame();
        let mut size = FrameSize::new(
            &frame.title,
            &frame.working_directory,
            frame.colors.cursor.is_some(),
            frame.cursor.viewport.is_some(),
        )
        .unwrap();
        for row in &frame.rows {
            size.add_row().unwrap();
            for cell in &row.cells {
                size.add_cell(&cell.text, &cell.hyperlink).unwrap();
            }
        }
        assert_eq!(size.bytes(), frame.encode().unwrap().len());

        let mut size = FrameSize::new("", "", false, false).unwrap();
        let before = size;
        let oversized = "x".repeat(MAX_FRAME_BYTES);
        assert!(matches!(
            size.add_cell(&oversized, ""),
            Err(Error::FrameTooLarge { .. })
        ));
        assert_eq!(size, before);
    }

    #[test]
    fn every_truncated_rich_frame_is_rejected() {
        let bytes = rich_frame().encode().unwrap();
        for length in 0..bytes.len() {
            assert!(decode_frame(&bytes[..length]).is_err(), "accepted {length}");
        }
    }

    #[test]
    fn malformed_tags_flags_dimensions_and_lengths_are_rejected() {
        let canonical = minimal_frame(0).encode().unwrap();
        assert_eq!(canonical.len(), 850);

        let mut cases = Vec::new();
        let mut wrong_magic = canonical.clone();
        wrong_magic[0] = b'X';
        cases.push(wrong_magic);
        let mut wrong_version = canonical.clone();
        wrong_version[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
        cases.push(wrong_version);
        let mut zero_cols = canonical.clone();
        zero_cols[14..16].copy_from_slice(&0_u16.to_le_bytes());
        cases.push(zero_cols);
        let mut hostile_dimensions = canonical.clone();
        hostile_dimensions[14..16].copy_from_slice(&u16::MAX.to_le_bytes());
        hostile_dimensions[16..18].copy_from_slice(&u16::MAX.to_le_bytes());
        cases.push(hostile_dimensions);
        let mut wrong_screen = canonical.clone();
        wrong_screen[SCREEN_OFFSET] = 2;
        cases.push(wrong_screen);
        let mut wrong_capability = canonical.clone();
        wrong_capability[HYPERLINK_CAPABILITY_OFFSET] = 2;
        cases.push(wrong_capability);
        let mut wrong_cursor_color = canonical.clone();
        wrong_cursor_color[CURSOR_COLOR_PRESENCE_OFFSET] = 2;
        cases.push(wrong_cursor_color);
        let mut wrong_cursor_shape = canonical.clone();
        wrong_cursor_shape[CURSOR_SHAPE_OFFSET] = 4;
        cases.push(wrong_cursor_shape);
        let mut wrong_cursor_presence = canonical.clone();
        wrong_cursor_presence[CURSOR_VIEWPORT_PRESENCE_OFFSET] = 2;
        cases.push(wrong_cursor_presence);
        let mut wrong_row_flags = canonical.clone();
        wrong_row_flags[ROW_FLAGS_OFFSET] = 1 << 7;
        cases.push(wrong_row_flags);
        let mut wrong_width = canonical.clone();
        wrong_width[CELL_WIDTH_OFFSET] = 4;
        cases.push(wrong_width);
        let mut wrong_color = canonical.clone();
        wrong_color[FOREGROUND_TAG_OFFSET] = 3;
        cases.push(wrong_color);
        let mut wrong_style_flags = canonical.clone();
        wrong_style_flags[STYLE_FLAGS_OFFSET..STYLE_FLAGS_OFFSET + 2]
            .copy_from_slice(&0x0400_u16.to_le_bytes());
        cases.push(wrong_style_flags);
        let mut wrong_underline = canonical.clone();
        wrong_underline[UNDERLINE_OFFSET] = 6;
        cases.push(wrong_underline);
        let mut hostile_title = canonical.clone();
        hostile_title[TITLE_LENGTH_OFFSET..TITLE_LENGTH_OFFSET + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(hostile_title);
        let mut trailing = canonical.clone();
        trailing.push(0);
        cases.push(trailing);

        for bytes in cases {
            assert!(decode_frame(&bytes).is_err());
        }
        assert_eq!(
            decode_frame(&vec![0; MAX_FRAME_BYTES + 1]),
            Err(Error::FrameTooLarge {
                size: MAX_FRAME_BYTES + 1
            })
        );
    }

    #[test]
    fn noncanonical_color_padding_is_rejected() {
        let mut bytes = minimal_frame(0).encode().unwrap();
        bytes[FOREGROUND_TAG_OFFSET + 1] = 1;
        assert!(matches!(
            decode_frame(&bytes),
            Err(Error::InvalidTag {
                field: "foreground color",
                ..
            })
        ));
    }

    #[test]
    fn owned_shapes_are_validated_before_encoding() {
        let mut frame = minimal_frame(0);
        frame.rows.clear();
        assert_eq!(frame.encode(), Err(Error::InvalidShape { field: "row" }));

        let mut frame = minimal_frame(0);
        frame.rows[0].cells.clear();
        assert_eq!(frame.encode(), Err(Error::InvalidShape { field: "cell" }));

        let mut frame = minimal_frame(0);
        frame.cursor.viewport = Some(CursorViewport {
            x: 1,
            y: 0,
            at_wide_tail: false,
        });
        assert_eq!(
            frame.encode(),
            Err(Error::InvalidShape {
                field: "cursor viewport"
            })
        );
    }

    #[test]
    fn reducer_accepts_only_strictly_newer_complete_frames() {
        let mut reducer = FrameReducer::default();
        reducer.push(minimal_frame(7)).unwrap();
        assert_eq!(reducer.current().unwrap().revision, 7);
        assert_eq!(
            reducer.push(minimal_frame(7)),
            Err(Error::RevisionNotNewer {
                current: 7,
                incoming: 7
            })
        );
        assert_eq!(
            reducer.push(minimal_frame(6)),
            Err(Error::RevisionNotNewer {
                current: 7,
                incoming: 6
            })
        );
        reducer
            .decode_and_push(&minimal_frame(8).encode().unwrap())
            .unwrap();
        assert_eq!(reducer.into_current().unwrap().revision, 8);
    }
}
