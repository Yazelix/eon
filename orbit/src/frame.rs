//! Canonical frames and vertical previews read from libghostty.

use crate::errno;
use libghostty_vt::{
    Error as GhosttyError, RenderState, Terminal,
    kitty::graphics::{Graphics as Storage, Image, ImageFormat},
    render::{
        CellIterator, CursorVisualStyle as GhosttyCursorShape, ImageIterator,
        ImageLayer as GhosttyImageLayer, ImageProtocol, RowIterator, Snapshot,
    },
    screen::{CellContentTag, CellWide as GhosttyCellWidth, GridRef, Screen as GhosttyScreen},
    style::{RgbColor, Style, StyleColor as GhosttyStyleColor, Underline as GhosttyUnderline},
    terminal::{Point, PointCoordinate},
};
use orbit_protocol::{
    Capabilities, Cell as ProtocolCell, CellStyle, CellWidth, Colors, Cursor, CursorShape,
    CursorViewport, Dimensions, FRAME_BUDGET_BYTES, Frame, FrameSize, Graphics, ImageLayer,
    MAX_CELLS, MAX_FRAME_RESOURCES, MAX_PLACEMENTS, Placement, PlacementState, Rgb, Row,
    Screen as ProtocolScreen, ScrollPosition, SourceRect, StyleColor, Underline,
    session::{PixelFormat, PreviewOutcome, ResourceAnnounce, VerticalDirection},
};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self},
};

pub(crate) const SCROLLBACK_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// The Kitty graphics Unicode placeholder, which ORB-C16 frames carry blank.
const PLACEHOLDER: char = '\u{10EEEE}';

/// The most resources announced and not retired per attachment (ORB-C16).
const MAX_UNRETIRED: usize = 1024;

pub(crate) struct FrameExtractor {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    images: ImageIterator<'static>,
    /// ORB-C16's internal switch: frames carry graphics state and blank
    /// placeholder cells.
    graphics: bool,
    /// The last frame extraction's serial; each extraction takes the next.
    serial: u64,
    resources: Resources,
}

/// The current attachment's resources (ORB-C16).
struct Resources {
    /// The number the next new resource takes.
    next: u64,
    /// Numbers by libghostty image generation, which no other image, screen
    /// or terminal reuses: the resources the last frame referenced and the
    /// started ones not retired. A resource that leaves the frames before it
    /// starts is skipped and takes a fresh number when referenced again.
    numbers: HashMap<u64, u64>,
    /// The started resources not retired, by number.
    started: BTreeMap<u64, Started>,
    /// The last extraction's pending references that have not started,
    /// lowest number first; they become the candidates if its frame is
    /// queued.
    extracted: Vec<Candidate>,
    /// The last queued frame's pending references that have not started,
    /// which Bend's `Transfer` starts in number order.
    candidates: Vec<Candidate>,
    /// Whether a resource's last chunk was written since the last report.
    delivered: bool,
    /// Whether Bend has not heard of new candidates, a delivery, a start
    /// that left candidates, or a retirement.
    changed: bool,
    /// The plane last reported to Bend.
    reported: Plane,
}

/// A resource a queued frame references: its number, generation and Kitty
/// image id.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Candidate {
    number: u64,
    generation: u64,
    image: u32,
}

/// A resource whose announcement was queued.
struct Started {
    generation: u64,
    image: u32,
    /// The screen whose storage it was copied from.
    alternate: bool,
    /// Whether its last chunk was written.
    delivered: bool,
    /// The serial of the first extraction that found it gone from its
    /// screen's storage.
    gone_after: Option<u64>,
}

/// The resource plane as fact `Resources` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plane {
    pub(crate) delivered: bool,
    pub(crate) idle: bool,
    pub(crate) capped: bool,
    pub(crate) due: bool,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            next: 0,
            numbers: HashMap::new(),
            started: BTreeMap::new(),
            extracted: Vec::new(),
            candidates: Vec::new(),
            delivered: false,
            changed: false,
            // A new attachment has nothing in transfer, started or due.
            reported: Plane {
                delivered: false,
                idle: true,
                capped: false,
                due: false,
            },
        }
    }
}

impl Resources {
    /// The started resources whose retirement is due: gone after a frame
    /// serial that has been written, and not still queueing chunks.
    fn due(&self, written: u64, transferring: Option<u64>) -> impl Iterator<Item = u64> + '_ {
        self.started.iter().filter_map(move |(&number, started)| {
            (started.gone_after.is_some_and(|serial| serial <= written)
                && transferring != Some(number))
            .then_some(number)
        })
    }
}

impl FrameExtractor {
    pub(crate) fn new(graphics: bool) -> io::Result<Self> {
        Ok(Self {
            state: RenderState::new().map_err(|_| errno(libc::EIO))?,
            rows: RowIterator::new().map_err(|_| errno(libc::EIO))?,
            cells: CellIterator::new().map_err(|_| errno(libc::EIO))?,
            images: ImageIterator::new().map_err(|_| errno(libc::EIO))?,
            graphics,
            serial: 0,
            resources: Resources::default(),
        })
    }

    /// Starts a new attachment, which has no resources yet.
    pub(crate) fn attach(&mut self) {
        self.resources = Resources::default();
    }

    /// The last frame extraction's serial.
    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    /// The last extraction's frame was queued for the client: its pending
    /// references that have not started replace the candidates.
    pub(crate) fn queued(&mut self) {
        let resources = &mut self.resources;
        let extracted = std::mem::take(&mut resources.extracted);
        resources.changed |= !extracted.is_empty() && extracted != resources.candidates;
        resources.candidates = extracted;
    }

    /// Starts the lowest-numbered candidate that a current frame still
    /// numbers and storage still holds at its generation with its pixels:
    /// copies them and returns its announcement. A candidate whose copy fails
    /// starts nothing and loses its number, so a later frame gives it a
    /// fresh one.
    /// `retiring` counts the retirements queued but not yet written, whose
    /// resources still count toward the cap.
    pub(crate) fn start(
        &mut self,
        terminal: &Terminal<'static, '_>,
        retiring: usize,
    ) -> io::Result<Option<(ResourceAnnounce, Vec<u8>)>> {
        let alternate = matches!(
            terminal.active_screen().map_err(|_| errno(libc::EIO))?,
            GhosttyScreen::Alternate
        );
        let storage = terminal.kitty_graphics().map_err(|_| errno(libc::EIO))?;
        let resources = &mut self.resources;
        if resources.started.len() + retiring >= MAX_UNRETIRED {
            return Err(errno(libc::EPROTO));
        }
        while !resources.candidates.is_empty() {
            let candidate = resources.candidates.remove(0);
            let current = resources.numbers.get(&candidate.generation) == Some(&candidate.number)
                && !resources.started.contains_key(&candidate.number);
            if !current {
                continue;
            }
            if let Some(copied) = copy(&storage, candidate)? {
                resources.started.insert(
                    candidate.number,
                    Started {
                        generation: candidate.generation,
                        image: candidate.image,
                        alternate,
                        delivered: false,
                        gone_after: None,
                    },
                );
                resources.changed |= !resources.candidates.is_empty();
                return Ok(Some(copied));
            }
            resources.numbers.remove(&candidate.generation);
        }
        Ok(None)
    }

    /// A resource's last chunk was written.
    pub(crate) fn delivered(&mut self, resource: u64) {
        if let Some(started) = self.resources.started.get_mut(&resource) {
            started.delivered = true;
            self.resources.delivered = true;
            self.resources.changed = true;
        }
    }

    /// Forgets the resources whose retirement is due, given the highest
    /// frame serial written and the resource still queueing chunks, and
    /// returns them for their retirements.
    pub(crate) fn retire(&mut self, written: u64, transferring: Option<u64>) -> Vec<u64> {
        let resources = &mut self.resources;
        let due: Vec<_> = resources.due(written, transferring).collect();
        for number in &due {
            if let Some(started) = resources.started.remove(number) {
                resources.numbers.remove(&started.generation);
            }
        }
        resources.changed |= !due.is_empty();
        due
    }

    /// The plane to report to Bend while the switch is on and it changed
    /// since the last report: new candidates, a delivery, a start that left
    /// candidates, a retirement, or a change in whether a resource may start
    /// (`room`), the cap is reached or a retirement is due. A resource counts
    /// toward the cap until its retirement is written, so `retiring` counts
    /// the retirements queued but not yet written.
    pub(crate) fn plane(
        &mut self,
        written: u64,
        transferring: Option<u64>,
        room: bool,
        retiring: usize,
    ) -> Option<Plane> {
        let resources = &mut self.resources;
        let plane = Plane {
            delivered: resources.delivered,
            idle: room,
            capped: resources.started.len() + retiring >= MAX_UNRETIRED,
            due: resources.due(written, transferring).next().is_some(),
        };
        let quiet = Plane {
            delivered: false,
            ..plane
        };
        if !self.graphics || !resources.changed && quiet == resources.reported {
            return None;
        }
        resources.delivered = false;
        resources.changed = false;
        resources.reported = quiet;
        Some(plane)
    }

    pub(crate) fn frame(
        &mut self,
        revision: u64,
        terminal: &Terminal<'static, '_>,
    ) -> io::Result<Frame> {
        self.frame_within(revision, terminal, FRAME_BUDGET_BYTES)
    }

    /// A frame that fits `budget` bytes: every cell's first codepoint, then
    /// full cell text and then links in reading order while they fit. From
    /// the first cell whose text does not fit, cells keep only their first
    /// codepoint, and from the first link that does not fit, cells have no
    /// link. A program's output never ends its terminal (ORB-C6).
    fn frame_within(
        &mut self,
        revision: u64,
        terminal: &Terminal<'static, '_>,
        budget: usize,
    ) -> io::Result<Frame> {
        self.serial += 1;
        let snapshot = self.state.update(terminal).map_err(|_| errno(libc::EIO))?;
        let cols = snapshot.cols().map_err(|_| errno(libc::EIO))?;
        let rows = snapshot.rows().map_err(|_| errno(libc::EIO))?;
        if usize::from(cols) * usize::from(rows) > MAX_CELLS {
            return Err(errno(libc::EOVERFLOW));
        }

        let colors = snapshot.colors().map_err(|_| errno(libc::EIO))?;
        let screen = match terminal.active_screen().map_err(|_| errno(libc::EIO))? {
            GhosttyScreen::Primary => ProtocolScreen::Primary,
            GhosttyScreen::Alternate => ProtocolScreen::Alternate,
        };
        let scrollbar = terminal.scrollbar().map_err(|_| errno(libc::EIO))?;
        if scrollbar.len != u64::from(rows) {
            return Err(errno(libc::EPROTO));
        }
        let history_rows = scrollbar
            .total
            .checked_sub(scrollbar.len)
            .ok_or_else(|| errno(libc::EPROTO))?;
        let cursor = Cursor {
            visible: snapshot.cursor_visible().map_err(|_| errno(libc::EIO))?,
            blinking: snapshot.cursor_blinking().map_err(|_| errno(libc::EIO))?,
            password_input: snapshot
                .cursor_password_input()
                .map_err(|_| errno(libc::EIO))?,
            shape: cursor_shape(
                snapshot
                    .cursor_visual_style()
                    .map_err(|_| errno(libc::EIO))?,
            )?,
            viewport: snapshot
                .cursor_viewport()
                .map_err(|_| errno(libc::EIO))?
                .map(|cursor| CursorViewport {
                    x: cursor.x,
                    y: cursor.y,
                    at_wide_tail: cursor.at_wide_tail,
                }),
        };
        let (title, working_directory) = title_and_directory(terminal);
        let mut size = FrameSize::new(
            title,
            working_directory,
            colors.cursor.is_some(),
            cursor.viewport.is_some(),
        )
        .map_err(|_| errno(libc::EOVERFLOW))?;
        let title = title.to_owned();
        let working_directory = working_directory.to_owned();
        let mut row_iter = self.rows.update(&snapshot).map_err(|_| errno(libc::EIO))?;
        let mut text = String::new();
        let mut frame_rows = Vec::with_capacity(usize::from(rows));
        let mut text_bytes = 0;
        let mut link_bytes = 0;
        for y in 0..rows {
            size.add_row().map_err(|_| errno(libc::EOVERFLOW))?;
            let row = row_iter.next().ok_or_else(|| errno(libc::EPROTO))?;
            let raw_row = row.raw_row().map_err(|_| errno(libc::EIO))?;
            let mut cell_iter = self.cells.update(row).map_err(|_| errno(libc::EIO))?;
            let mut frame_cells = Vec::with_capacity(usize::from(cols));
            for x in 0..cols {
                let cell = cell_iter.next().ok_or_else(|| errno(libc::EPROTO))?;
                let raw_cell = cell.raw_cell().map_err(|_| errno(libc::EIO))?;
                let mut style = cell.style().map_err(|_| errno(libc::EIO))?;
                if let Some(background) = cell.bg_color().map_err(|_| errno(libc::EIO))? {
                    style.bg_color = GhosttyStyleColor::Rgb(background);
                }
                let grapheme_count = cell.graphemes_len().map_err(|_| errno(libc::EIO))?;
                if grapheme_count as usize > FRAME_BUDGET_BYTES / size_of::<char>() {
                    return Err(errno(libc::EOVERFLOW));
                }
                text.clear();
                cell.graphemes_utf8(&mut text)
                    .map_err(|_| errno(libc::EOVERFLOW))?;
                let placeholder = self.graphics && text.starts_with(PLACEHOLDER);
                if placeholder {
                    text.clear();
                }
                // The cell reserves its first codepoint and an empty link, so
                // its other codepoints and its link add only their bytes.
                // Later cells only shrink the room, so once the text or the
                // links read pass the budget, none after them can fit.
                let base = first_codepoint_len(&text);
                size.add_cell(&text[..base], "")
                    .map_err(|_| errno(libc::EOVERFLOW))?;
                text_bytes += text.len() - base;
                if size.bytes() + text_bytes > budget {
                    text.truncate(base);
                }
                let hyperlink = if !placeholder
                    && size.bytes() + link_bytes <= budget
                    && raw_cell.has_hyperlink().map_err(|_| errno(libc::EIO))?
                {
                    hyperlink(terminal, x, y)?
                } else {
                    String::new()
                };
                link_bytes += hyperlink.len();
                let style = cell_style(
                    style,
                    cell.is_selected().map_err(|_| errno(libc::EIO))?,
                    raw_cell.is_protected().map_err(|_| errno(libc::EIO))?,
                )?;
                frame_cells.push(ProtocolCell {
                    width: cell_width(raw_cell.wide().map_err(|_| errno(libc::EIO))?),
                    style: if placeholder { blank(style) } else { style },
                    text: text.clone(),
                    hyperlink,
                });
            }
            if cell_iter.next().is_some() {
                return Err(errno(libc::EPROTO));
            }
            frame_rows.push(Row {
                wrapped: raw_row.is_wrapped().map_err(|_| errno(libc::EIO))?,
                wrap_continuation: raw_row
                    .is_wrap_continuation()
                    .map_err(|_| errno(libc::EIO))?,
                cells: frame_cells,
            });
        }
        if row_iter.next().is_some() {
            return Err(errno(libc::EPROTO));
        }
        let mut bytes = size.bytes();
        text_bytes = 0;
        for cell in frame_rows.iter_mut().flat_map(|row| &mut row.cells) {
            let base = first_codepoint_len(&cell.text);
            text_bytes += cell.text.len() - base;
            if size.bytes() + text_bytes <= budget {
                bytes = size.bytes() + text_bytes;
            } else {
                cell.text.truncate(base);
            }
        }
        link_bytes = 0;
        for cell in frame_rows.iter_mut().flat_map(|row| &mut row.cells) {
            link_bytes += cell.hyperlink.len();
            if bytes + link_bytes > budget {
                cell.hyperlink.clear();
            }
        }
        let graphics = if self.graphics {
            Some(graphics(
                &mut self.images,
                &mut self.resources,
                &snapshot,
                terminal,
                self.serial,
            )?)
        } else {
            None
        };
        Ok(Frame {
            revision,
            dimensions: Dimensions { cols, rows },
            screen,
            title,
            working_directory,
            capabilities: Capabilities { hyperlinks: true },
            colors: Colors {
                background: rgb(colors.background),
                foreground: rgb(colors.foreground),
                cursor: colors.cursor.map(rgb),
                palette: colors.palette.map(rgb),
            },
            cursor,
            scroll_position: ScrollPosition {
                rows_from_live: history_rows
                    .checked_sub(scrollbar.offset)
                    .ok_or_else(|| errno(libc::EPROTO))?,
                history_rows,
            },
            rows: frame_rows,
            graphics,
        })
    }
}

impl FrameExtractor {
    /// One canonical row from libghostty's screen coordinates, for history beyond the viewport.
    fn row(&self, terminal: &Terminal<'static, '_>, screen_y: u32, cols: u16) -> io::Result<Row> {
        let at = |x| {
            terminal
                .grid_ref(Point::Screen(PointCoordinate { x, y: screen_y }))
                .map_err(|_| errno(libc::EIO))
        };
        let raw_row = at(0)?.row().map_err(|_| errno(libc::EIO))?;
        let palette = terminal.color_palette().map_err(|_| errno(libc::EIO))?;
        let mut chars = vec!['\0'; 8];
        let mut cells = Vec::with_capacity(usize::from(cols));
        for x in 0..cols {
            let reference = at(x)?;
            let raw_cell = reference.cell().map_err(|_| errno(libc::EIO))?;
            let mut style = reference.style().map_err(|_| errno(libc::EIO))?;
            style.bg_color = match raw_cell.content_tag().map_err(|_| errno(libc::EIO))? {
                CellContentTag::BgColorPalette => GhosttyStyleColor::Rgb(
                    palette.get(raw_cell.bg_color_palette().map_err(|_| errno(libc::EIO))?),
                ),
                CellContentTag::BgColorRgb => {
                    GhosttyStyleColor::Rgb(raw_cell.bg_color_rgb().map_err(|_| errno(libc::EIO))?)
                }
                CellContentTag::Codepoint | CellContentTag::CodepointGrapheme => {
                    match style.bg_color {
                        GhosttyStyleColor::Palette(index) => {
                            GhosttyStyleColor::Rgb(palette.get(index))
                        }
                        background => background,
                    }
                }
            };
            let length = match reference.graphemes(&mut chars) {
                Ok(length) => length,
                Err(GhosttyError::OutOfSpace { required })
                    if required <= FRAME_BUDGET_BYTES / size_of::<char>() =>
                {
                    chars.resize(required, '\0');
                    reference
                        .graphemes(&mut chars)
                        .map_err(|_| errno(libc::EIO))?
                }
                Err(GhosttyError::OutOfSpace { .. }) => return Err(errno(libc::EOVERFLOW)),
                Err(_) => return Err(errno(libc::EIO)),
            };
            let style = cell_style(
                style,
                false,
                raw_cell.is_protected().map_err(|_| errno(libc::EIO))?,
            )?;
            if self.graphics && chars[..length].first() == Some(&PLACEHOLDER) {
                cells.push(ProtocolCell {
                    width: cell_width(raw_cell.wide().map_err(|_| errno(libc::EIO))?),
                    style: blank(style),
                    text: String::new(),
                    hyperlink: String::new(),
                });
                continue;
            }
            let hyperlink = if raw_cell.has_hyperlink().map_err(|_| errno(libc::EIO))? {
                hyperlink_uri(&reference)?
            } else {
                String::new()
            };
            cells.push(ProtocolCell {
                width: cell_width(raw_cell.wide().map_err(|_| errno(libc::EIO))?),
                style,
                text: chars[..length].iter().collect(),
                hyperlink,
            });
        }
        Ok(Row {
            wrapped: raw_row.is_wrapped().map_err(|_| errno(libc::EIO))?,
            wrap_continuation: raw_row
                .is_wrap_continuation()
                .map_err(|_| errno(libc::EIO))?,
            cells,
        })
    }
}

/// The terminal's title and working directory, each empty when libghostty
/// cannot read it or it is not UTF-8: an OSC 7 directory can be invalid UTF-8,
/// and so is a title cut inside a character at libghostty's 1024 bytes. A
/// program's bad metadata never ends its terminal.
pub(crate) fn title_and_directory<'a>(terminal: &'a Terminal<'static, '_>) -> (&'a str, &'a str) {
    let text = |bytes: libghostty_vt::error::Result<&'a [u8]>| {
        std::str::from_utf8(bytes.unwrap_or_default()).unwrap_or_default()
    };
    (text(terminal.title()), text(terminal.pwd()))
}

/// Up to one viewport of rows beyond the viewport, nearest first, bounded like a frame.
pub(crate) fn viewport_preview(
    terminal: &Terminal<'static, '_>,
    extractor: &FrameExtractor,
    direction: VerticalDirection,
) -> io::Result<PreviewOutcome> {
    let scrollbar = terminal.scrollbar().map_err(|_| errno(libc::EIO))?;
    let cols = terminal.cols().map_err(|_| errno(libc::EIO))?;
    let window = u64::from(terminal.rows().map_err(|_| errno(libc::EIO))?);
    let below = scrollbar.offset.saturating_add(scrollbar.len);
    let mut size = FrameSize::new("", "", false, false).map_err(|_| errno(libc::EOVERFLOW))?;
    let mut rows = Vec::new();
    let mut complete = true;
    for index in 0..window {
        let target = match direction {
            VerticalDirection::Up => scrollbar.offset.checked_sub(index + 1),
            VerticalDirection::Down => below.checked_add(index).filter(|y| *y < scrollbar.total),
        };
        let Some(target) = target else {
            break;
        };
        let row = extractor.row(
            terminal,
            u32::try_from(target).map_err(|_| errno(libc::EOVERFLOW))?,
            cols,
        )?;
        let mut next = size;
        let sized = next.add_row().and_then(|()| {
            row.cells
                .iter()
                .try_for_each(|cell| next.add_cell(&cell.text, &cell.hyperlink))
        });
        if sized.is_err() {
            complete = false;
            break;
        }
        size = next;
        rows.push(row);
    }
    let edge_reached = complete
        && match direction {
            VerticalDirection::Up => scrollbar.offset <= window,
            VerticalDirection::Down => below.saturating_add(window) >= scrollbar.total,
        };
    Ok(PreviewOutcome::Viewport {
        cols,
        edge_reached,
        rows,
    })
}

/// ORB-C16's graphics state for the snapshot's viewport: libghostty's
/// placements by layer, z-index and Kitty image id, libghostty's order
/// breaking ties, up to the first entry that would pass either cap. An entry
/// within the resource limits is ready once its resource was delivered, and
/// pending until then. A started resource of the active screen that its
/// storage no longer holds is gone after this extraction's `serial`.
fn graphics(
    images: &mut ImageIterator<'static>,
    resources: &mut Resources,
    snapshot: &Snapshot<'static, '_>,
    terminal: &Terminal<'static, '_>,
    serial: u64,
) -> io::Result<Graphics> {
    let storage = terminal.kitty_graphics().map_err(|_| errno(libc::EIO))?;
    let alternate = matches!(
        terminal.active_screen().map_err(|_| errno(libc::EIO))?,
        GhosttyScreen::Alternate
    );
    for started in resources.started.values_mut() {
        if started.alternate == alternate && started.gone_after.is_none() {
            let held = match storage.image(started.image) {
                Some(image) => {
                    image.generation().map_err(|_| errno(libc::EIO))? == started.generation
                }
                None => false,
            };
            if !held {
                started.gone_after = Some(serial);
            }
        }
    }
    let mut entries = Vec::new();
    let mut iteration = images.update(snapshot).map_err(|_| errno(libc::EIO))?;
    while let Some(placement) = iteration.next() {
        // Another protocol, including one this crate does not know, is not
        // ORB-C16's.
        let Ok(ImageProtocol::Kitty) = placement.protocol() else {
            continue;
        };
        let id = placement.image_id().map_err(|_| errno(libc::EIO))?;
        let state = match storage.image(id) {
            Some(image) if announcement(&image, 0, 0)?.is_some() => PlacementState::Pending,
            _ => PlacementState::Unavailable,
        };
        let entry = Placement {
            resource: 0,
            layer: image_layer(placement.layer().map_err(|_| errno(libc::EIO))?),
            z: placement.z().map_err(|_| errno(libc::EIO))?,
            col: placement.viewport_col().map_err(|_| errno(libc::EIO))?,
            row: placement.viewport_row().map_err(|_| errno(libc::EIO))?,
            grid_cols: placement.grid_cols().map_err(|_| errno(libc::EIO))?,
            grid_rows: placement.grid_rows().map_err(|_| errno(libc::EIO))?,
            offset_x: placement.cell_offset_x().map_err(|_| errno(libc::EIO))?,
            offset_y: placement.cell_offset_y().map_err(|_| errno(libc::EIO))?,
            width: placement.pixel_width().map_err(|_| errno(libc::EIO))?,
            height: placement.pixel_height().map_err(|_| errno(libc::EIO))?,
            source: SourceRect {
                x: placement.source_x().map_err(|_| errno(libc::EIO))?,
                y: placement.source_y().map_err(|_| errno(libc::EIO))?,
                width: placement.source_width().map_err(|_| errno(libc::EIO))?,
                height: placement.source_height().map_err(|_| errno(libc::EIO))?,
            },
            state,
        };
        let generation = placement.image_generation().map_err(|_| errno(libc::EIO))?;
        entries.push((id, generation, entry));
    }
    // A stable sort keeps libghostty's order among equal keys.
    entries.sort_by_key(|(id, _, entry)| (entry.layer, entry.z, *id));
    let mut numbers = HashMap::new();
    let mut extracted = BTreeMap::new();
    let mut placements = Vec::with_capacity(entries.len().min(MAX_PLACEMENTS));
    let mut placements_omitted = false;
    for (image, generation, mut entry) in entries {
        if placements.len() == MAX_PLACEMENTS
            || !numbers.contains_key(&generation) && numbers.len() == MAX_FRAME_RESOURCES
        {
            placements_omitted = true;
            break;
        }
        entry.resource = *numbers.entry(generation).or_insert_with(|| {
            resources
                .numbers
                .get(&generation)
                .copied()
                .unwrap_or_else(|| {
                    let number = resources.next;
                    resources.next += 1;
                    number
                })
        });
        if entry.state == PlacementState::Pending {
            match resources.started.get(&entry.resource) {
                Some(started) if started.delivered => entry.state = PlacementState::Ready,
                Some(_) => {}
                None => {
                    extracted.insert(
                        entry.resource,
                        Candidate {
                            number: entry.resource,
                            generation,
                            image,
                        },
                    );
                }
            }
        }
        placements.push(entry);
    }
    // A started resource keeps its number until it retires.
    for (&number, started) in &resources.started {
        numbers.insert(started.generation, number);
    }
    resources.numbers = numbers;
    resources.extracted = extracted.into_values().collect();
    // libghostty's geometry uses these cells; with no client size yet it
    // has none, and the list is empty.
    let cell = |pixels: libghostty_vt::error::Result<u32>, cells| {
        pixels
            .map(|pixels| (pixels / u32::from(cells)).max(1))
            .map_err(|_| errno(libc::EIO))
    };
    Ok(Graphics {
        cell_width: cell(
            terminal.width_px(),
            terminal.cols().map_err(|_| errno(libc::EIO))?,
        )?,
        cell_height: cell(
            terminal.height_px(),
            terminal.rows().map_err(|_| errno(libc::EIO))?,
        )?,
        placements_omitted,
        placements,
    })
}

/// An image's announcement as this resource, or None when it breaks
/// ORB-C16's resource limits: 8-bit RGB or RGBA, 1 to 4,096 pixels on each
/// side and at most 10,000,000 decoded bytes.
fn announcement(
    image: &Image<'_>,
    resource: u64,
    generation: u64,
) -> io::Result<Option<ResourceAnnounce>> {
    let format = match image.format() {
        Ok(ImageFormat::Rgb) => PixelFormat::Rgb,
        Ok(ImageFormat::Rgba) => PixelFormat::Rgba,
        _ => return Ok(None),
    };
    let width = image.width().map_err(|_| errno(libc::EIO))?;
    let height = image.height().map_err(|_| errno(libc::EIO))?;
    Ok(ResourceAnnounce::new(
        resource, generation, format, width, height,
    ))
}

/// A candidate's announcement and a copy of its pixels, if storage still
/// holds its image at its generation with all of its pixels.
fn copy(
    storage: &Storage<'_>,
    candidate: Candidate,
) -> io::Result<Option<(ResourceAnnounce, Vec<u8>)>> {
    let Some(image) = storage.image(candidate.image) else {
        return Ok(None);
    };
    if image.generation().map_err(|_| errno(libc::EIO))? != candidate.generation {
        return Ok(None);
    }
    let Some(announce) = announcement(&image, candidate.number, candidate.generation)? else {
        return Ok(None);
    };
    Ok(match image.data().map_err(|_| errno(libc::EIO))? {
        Some(pixels) if pixels.len() == announce.byte_length as usize => {
            Some((announce, pixels.to_vec()))
        }
        _ => None,
    })
}

fn image_layer(layer: GhosttyImageLayer) -> ImageLayer {
    match layer {
        GhosttyImageLayer::BelowBackground => ImageLayer::BelowBackground,
        GhosttyImageLayer::BelowText => ImageLayer::BelowText,
        GhosttyImageLayer::AboveText => ImageLayer::AboveText,
    }
}

/// A placeholder cell's style, keeping only what decides the background a
/// client paints: its colors, inverse, selection and protection (ORB-C16).
fn blank(style: CellStyle) -> CellStyle {
    CellStyle {
        foreground: style.foreground,
        background: style.background,
        underline_color: StyleColor::None,
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        inverse: style.inverse,
        invisible: false,
        strikethrough: false,
        overline: false,
        selected: style.selected,
        protected: style.protected,
        underline: Underline::None,
    }
}

fn first_codepoint_len(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

fn hyperlink(terminal: &Terminal<'static, '_>, x: u16, y: u16) -> io::Result<String> {
    let reference = terminal
        .grid_ref(Point::Viewport(PointCoordinate { x, y: u32::from(y) }))
        .map_err(|_| errno(libc::EIO))?;
    hyperlink_uri(&reference)
}

/// The cell's hyperlink URI, empty when it is not valid UTF-8: libghostty
/// stores OSC 8 bytes unchecked, and a program's bad link never ends its
/// terminal. A lossy decode would link somewhere the program never named.
fn hyperlink_uri(reference: &GridRef<'_>) -> io::Result<String> {
    let mut bytes = vec![0; 256];
    let length = match reference.hyperlink_uri(&mut bytes) {
        Ok(length) => length,
        Err(GhosttyError::OutOfSpace { required }) if required <= FRAME_BUDGET_BYTES => {
            bytes.resize(required, 0);
            reference
                .hyperlink_uri(&mut bytes)
                .map_err(|_| errno(libc::EIO))?
        }
        Err(GhosttyError::OutOfSpace { .. }) => return Err(errno(libc::EOVERFLOW)),
        Err(_) => return Err(errno(libc::EIO)),
    };
    bytes.truncate(length);
    Ok(String::from_utf8(bytes).unwrap_or_default())
}

fn rgb(color: RgbColor) -> Rgb {
    Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn cursor_shape(shape: GhosttyCursorShape) -> io::Result<CursorShape> {
    match shape {
        GhosttyCursorShape::Bar => Ok(CursorShape::Bar),
        GhosttyCursorShape::Block => Ok(CursorShape::Block),
        GhosttyCursorShape::Underline => Ok(CursorShape::Underline),
        GhosttyCursorShape::BlockHollow => Ok(CursorShape::BlockHollow),
        _ => Err(errno(libc::EPROTO)),
    }
}

fn cell_width(width: GhosttyCellWidth) -> CellWidth {
    match width {
        GhosttyCellWidth::Narrow => CellWidth::Narrow,
        GhosttyCellWidth::Wide => CellWidth::Wide,
        GhosttyCellWidth::SpacerTail => CellWidth::SpacerTail,
        GhosttyCellWidth::SpacerHead => CellWidth::SpacerHead,
    }
}

fn style_color(color: GhosttyStyleColor) -> StyleColor {
    match color {
        GhosttyStyleColor::None => StyleColor::None,
        GhosttyStyleColor::Palette(index) => StyleColor::Palette(index.0),
        GhosttyStyleColor::Rgb(color) => StyleColor::Rgb(rgb(color)),
    }
}

fn underline(value: GhosttyUnderline) -> io::Result<Underline> {
    match value {
        GhosttyUnderline::None => Ok(Underline::None),
        GhosttyUnderline::Single => Ok(Underline::Single),
        GhosttyUnderline::Double => Ok(Underline::Double),
        GhosttyUnderline::Curly => Ok(Underline::Curly),
        GhosttyUnderline::Dotted => Ok(Underline::Dotted),
        GhosttyUnderline::Dashed => Ok(Underline::Dashed),
        _ => Err(errno(libc::EPROTO)),
    }
}

fn cell_style(style: Style, selected: bool, protected: bool) -> io::Result<CellStyle> {
    Ok(CellStyle {
        foreground: style_color(style.fg_color),
        background: style_color(style.bg_color),
        underline_color: style_color(style.underline_color),
        bold: style.bold,
        italic: style.italic,
        faint: style.faint,
        blink: style.blink,
        inverse: style.inverse,
        invisible: style.invisible,
        strikethrough: style.strikethrough,
        overline: style.overline,
        selected,
        protected,
        underline: underline(style.underline)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::terminal::ScrollViewport;
    use orbit_protocol::session::{self, MAX_RESOURCE_SIDE, ServerMessage, VerticalPreview};

    /// Transmits a white RGB image of `width` by `height` pixels as Kitty
    /// image `id`, with the control keys `keys`.
    fn transmit(
        terminal: &mut Terminal<'static, '_>,
        id: u32,
        width: u32,
        height: u32,
        keys: &str,
    ) {
        // Three 0xFF bytes are one "////" in base64.
        let pixels = "////".repeat((width * height) as usize);
        terminal.vt_write(
            format!("\x1b_Gf=24,s={width},v={height},i={id},q=2,{keys};{pixels}\x1b\\").as_bytes(),
        );
    }

    fn graphics(frame: &Frame) -> &Graphics {
        frame.graphics.as_ref().expect("the switch is on")
    }

    /// Yazi's Kitty stream: a virtual placement, then rows of Unicode
    /// placeholder cells whose foreground names the image and whose
    /// diacritics name its row. Each row is one pending entry with
    /// libghostty's geometry, and its cells are blank in frames and previews
    /// (ORB-C16). Identities hold while frames reference a resource, and a
    /// new generation or a skipped resource takes a fresh one.
    #[test]
    fn placeholder_rows_are_pending_entries_over_blank_cells() {
        let mut terminal = Terminal::new(10, 4).unwrap();
        let mut extractor = FrameExtractor::new(true).unwrap();
        // With no client size libghostty has no cells to place images in.
        transmit(&mut terminal, 7, 30, 40, "a=T,U=1,c=3,r=2");
        let before_size = extractor.frame(1, &terminal).unwrap();
        assert_eq!(
            (
                graphics(&before_size).cell_width,
                graphics(&before_size).placements.len()
            ),
            (1, 0)
        );
        terminal.resize(10, 4, 10, 20).unwrap();
        // Two rows of three cells at column 2, under a background, an
        // underline and a link.
        let rows = "\x1b]8;;https://example.test\x1b\\\x1b[38;5;7;48;2;1;2;3;4m\x1b[2;3H\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{10EEEE}\x1b[3;3H\u{10EEEE}\u{30D}\u{305}\u{10EEEE}\u{10EEEE}\x1b[0m\x1b]8;;\x1b\\";
        terminal.vt_write(rows.as_bytes());
        let frame = extractor.frame(2, &terminal).unwrap();
        let entry = |row, source_y| Placement {
            resource: 0,
            layer: ImageLayer::BelowText,
            z: -1,
            col: 2,
            row,
            grid_cols: 3,
            grid_rows: 1,
            offset_x: 0,
            offset_y: 0,
            width: 30,
            height: 20,
            source: SourceRect {
                x: 0,
                y: source_y,
                width: 30,
                height: 20,
            },
            state: PlacementState::Pending,
        };
        assert_eq!(
            *graphics(&frame),
            Graphics {
                cell_width: 10,
                cell_height: 20,
                placements_omitted: false,
                placements: vec![entry(1, 0), entry(2, 20)],
            }
        );
        let blank_cells = |row: &Row| {
            row.cells[2..5].iter().all(|cell| {
                cell.text.is_empty()
                    && cell.style.background == StyleColor::Rgb(Rgb { r: 1, g: 2, b: 3 })
                    && cell.style.underline == Underline::None
                    && cell.hyperlink.is_empty()
            })
        };
        assert!(blank_cells(&frame.rows[1]) && blank_cells(&frame.rows[2]));
        assert!(blank_cells(&extractor.row(&terminal, 1, 10).unwrap()));
        session::encode_server_message(&ServerMessage::Frame(Box::new(frame))).unwrap();

        // Pixels sent again under the same id are a new generation.
        transmit(&mut terminal, 7, 30, 40, "a=T,U=1,c=3,r=2");
        let replaced = extractor.frame(3, &terminal).unwrap();
        let resources = |frame: &Frame| -> Vec<u64> {
            graphics(frame)
                .placements
                .iter()
                .map(|entry| entry.resource)
                .collect()
        };
        assert_eq!(resources(&replaced), [1, 1]);
        assert_eq!(
            graphics(&extractor.frame(4, &terminal).unwrap()).placements,
            graphics(&replaced).placements
        );
        // Erased cells leave the image stored but unreferenced, so printing
        // them again starts over with a fresh identity.
        terminal.vt_write(b"\x1b[2;3H   \x1b[3;3H   ");
        assert!(
            graphics(&extractor.frame(5, &terminal).unwrap())
                .placements
                .is_empty()
        );
        terminal.vt_write(rows.as_bytes());
        assert_eq!(resources(&extractor.frame(6, &terminal).unwrap()), [2, 2]);
        // A new attachment numbers its resources afresh.
        extractor.attach();
        assert_eq!(resources(&extractor.frame(7, &terminal).unwrap()), [0, 0]);

        // With the switch off frames carry no graphics and keep the cells.
        let off = FrameExtractor::new(false)
            .unwrap()
            .frame(8, &terminal)
            .unwrap();
        assert!(off.graphics.is_none());
        let kept = &off.rows[1].cells[2];
        assert!(kept.text.starts_with(PLACEHOLDER) && kept.hyperlink == "https://example.test");
    }

    /// Fixed placements are listed by layer, z-index and image id, and an
    /// image past the resource limits is unavailable. Past 256 resources or
    /// 4,096 entries the list stops and says so, and the terminal goes on
    /// (ORB-C16).
    #[test]
    fn placements_are_ordered_and_capped() {
        let mut terminal = Terminal::new(10, 4).unwrap();
        terminal.resize(10, 4, 10, 20).unwrap();
        let mut extractor = FrameExtractor::new(true).unwrap();
        // Image 3 is two pixels wide, which tells it from image 1.
        for (id, width, z) in [(3, 2, 5), (1, 1, 5), (2, 1, -1), (4, 1, i32::MIN)] {
            transmit(&mut terminal, id, width, 1, &format!("a=T,C=1,z={z}"));
        }
        transmit(&mut terminal, 5, MAX_RESOURCE_SIDE + 1, 1, "a=T,C=1");
        let frame = extractor.frame(1, &terminal).unwrap();
        let order: Vec<_> = graphics(&frame)
            .placements
            .iter()
            .map(|entry| (entry.layer, entry.z, entry.width, entry.state))
            .collect();
        let pending = PlacementState::Pending;
        assert_eq!(
            order,
            [
                (ImageLayer::BelowBackground, i32::MIN, 1, pending),
                (ImageLayer::BelowText, -1, 1, pending),
                (ImageLayer::AboveText, 0, 4097, PlacementState::Unavailable),
                (ImageLayer::AboveText, 5, 1, pending),
                (ImageLayer::AboveText, 5, 2, pending),
            ]
        );
        assert!(!graphics(&frame).placements_omitted);

        // 257 more images: the list stops at the 256th resource.
        terminal.vt_write(b"\x1bc");
        terminal.resize(10, 4, 10, 20).unwrap();
        for id in 1..=257 {
            transmit(&mut terminal, id, 1, 1, "a=T,C=1");
        }
        let frame = extractor.frame(2, &terminal).unwrap();
        assert_eq!(graphics(&frame).placements.len(), MAX_FRAME_RESOURCES);
        assert!(graphics(&frame).placements_omitted);
        frame.encode().unwrap();

        // 4,097 placements of one image: the list stops at 4,096 entries.
        terminal.vt_write(b"\x1bc");
        terminal.resize(10, 4, 10, 20).unwrap();
        transmit(&mut terminal, 1, 1, 1, "a=t");
        for placement in 1..=4097 {
            terminal.vt_write(format!("\x1b_Ga=p,i=1,p={placement},C=1,q=2\x1b\\").as_bytes());
        }
        let frame = extractor.frame(3, &terminal).unwrap();
        assert_eq!(graphics(&frame).placements.len(), MAX_PLACEMENTS);
        assert!(graphics(&frame).placements_omitted);
        frame.encode().unwrap();
    }

    fn states(frame: &Frame) -> Vec<(u64, PlacementState)> {
        graphics(frame)
            .placements
            .iter()
            .map(|entry| (entry.resource, entry.state))
            .collect()
    }

    /// Only a queued frame's pending references can start. An entry is
    /// ready once its resource's last chunk was written, and a resource that
    /// left storage retires only once a frame extracted after that is
    /// written, and not while its chunks are still queueing (ORB-C16).
    #[test]
    fn resources_are_ready_once_delivered_and_retire_after_a_written_frame() {
        let mut terminal = Terminal::new(10, 4).unwrap();
        terminal.resize(10, 4, 10, 20).unwrap();
        let mut extractor = FrameExtractor::new(true).unwrap();
        transmit(&mut terminal, 1, 2, 1, "a=T,C=1");
        let pending = PlacementState::Pending;
        assert_eq!(
            states(&extractor.frame(1, &terminal).unwrap()),
            [(0, pending)]
        );
        // A frame built but never queued makes nothing startable.
        assert!(extractor.start(&terminal, 0).unwrap().is_none());
        assert_eq!(
            states(&extractor.frame(2, &terminal).unwrap()),
            [(0, pending)]
        );
        extractor.queued();
        let (announce, pixels) = extractor.start(&terminal, 0).unwrap().unwrap();
        assert_eq!((announce.resource, pixels), (0, vec![0xff; 6]));
        assert_eq!(
            states(&extractor.frame(3, &terminal).unwrap()),
            [(0, pending)]
        );
        extractor.delivered(0);
        let ready = PlacementState::Ready;
        assert_eq!(
            states(&extractor.frame(4, &terminal).unwrap()),
            [(0, ready)]
        );
        let plane = extractor.plane(0, None, true, 0).unwrap();
        assert!(plane.delivered && plane.idle && !plane.capped && !plane.due);
        assert_eq!(extractor.plane(0, None, true, 0), None);

        // Deleting the image and its pixels: it is gone after the next
        // extraction, which no longer references it.
        terminal.vt_write(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\");
        assert!(states(&extractor.frame(5, &terminal).unwrap()).is_empty());
        let gone = extractor.serial();
        assert!(extractor.retire(gone - 1, None).is_empty());
        assert!(extractor.retire(gone, Some(0)).is_empty());
        assert_eq!(
            extractor.plane(gone, None, true, 0).map(|plane| plane.due),
            Some(true)
        );
        assert_eq!(extractor.retire(gone, None), [0]);
        assert_eq!(
            extractor.plane(gone, None, true, 0).map(|plane| plane.due),
            Some(false)
        );
    }

    /// A resource counts toward the 1,024 cap until its retirement is
    /// written, not only until it is queued, so a slow client cannot gather
    /// more announced resources than the cap (ORB-C16).
    #[test]
    fn retirements_count_toward_the_cap_until_written() {
        let mut terminal = Terminal::new(10, 4).unwrap();
        terminal.resize(10, 4, 10, 20).unwrap();
        let mut extractor = FrameExtractor::new(true).unwrap();
        transmit(&mut terminal, 1, 2, 1, "a=T,C=1");
        extractor.frame(1, &terminal).unwrap();
        extractor.queued();
        // libghostty's generations are process-wide, so the fixture's are
        // taken from the top of the range, where no real image reaches.
        for number in u64::MAX - MAX_UNRETIRED as u64..u64::MAX {
            extractor.resources.started.insert(
                number,
                Started {
                    generation: number,
                    image: 2,
                    alternate: false,
                    delivered: true,
                    gone_after: Some(1),
                },
            );
        }
        let capped = |extractor: &mut FrameExtractor, retiring| {
            extractor.resources.changed = true;
            extractor.plane(1, None, true, retiring).unwrap().capped
        };
        assert!(capped(&mut extractor, 0));
        assert_eq!(extractor.retire(1, None).len(), MAX_UNRETIRED);
        // Every retirement is queued and none is written yet.
        assert!(capped(&mut extractor, MAX_UNRETIRED));
        assert!(extractor.start(&terminal, MAX_UNRETIRED).is_err());
        // One retirement written leaves room for one start.
        assert!(!capped(&mut extractor, MAX_UNRETIRED - 1));
        let (announce, _) = extractor
            .start(&terminal, MAX_UNRETIRED - 1)
            .unwrap()
            .unwrap();
        assert_eq!(announce.resource, 0);
    }

    /// A copy that fails starts nothing, and the next frame gives the
    /// resource a fresh number (ORB-C16). The alternate screen's storage does
    /// not hold the primary screen's image.
    #[test]
    fn a_failed_copy_takes_a_fresh_number() {
        let mut terminal = Terminal::new(10, 4).unwrap();
        terminal.resize(10, 4, 10, 20).unwrap();
        let mut extractor = FrameExtractor::new(true).unwrap();
        transmit(&mut terminal, 1, 2, 1, "a=T,C=1");
        let pending = PlacementState::Pending;
        assert_eq!(
            states(&extractor.frame(1, &terminal).unwrap()),
            [(0, pending)]
        );
        extractor.queued();
        terminal.vt_write(b"\x1b[?1049h");
        assert!(extractor.start(&terminal, 0).unwrap().is_none());
        terminal.vt_write(b"\x1b[?1049l");
        assert_eq!(
            states(&extractor.frame(2, &terminal).unwrap()),
            [(1, pending)]
        );
    }

    #[test]
    fn terminal_frame_uses_the_canonical_protocol_owner() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"\x1b]8;;https://example.test\x1b\\READY\x1b]8;;\x1b\\");
        let frame = FrameExtractor::new(false)
            .unwrap()
            .frame(1, &terminal)
            .unwrap();
        assert_eq!(
            (frame.revision, frame.dimensions.cols, frame.dimensions.rows),
            (1, 8, 2)
        );
        assert!(frame.capabilities.hyperlinks);
        assert_eq!(frame.rows[0].cells[0].text, "R");
        assert_eq!(frame.rows[0].cells[0].hyperlink, "https://example.test");
        let encoded =
            session::encode_server_message(&ServerMessage::Frame(Box::new(frame))).unwrap();
        assert!(matches!(
            session::decode_server_message(&encoded).unwrap(),
            ServerMessage::Frame(frame) if frame.revision == 1
        ));
    }

    /// A frame whose links exceed the budget keeps every cell's text, with
    /// links kept in reading order until the first that does not fit
    /// (ORB-C6). Each linked cell carries its URI, so one 2 KB link over
    /// most of a 100x24 screen is enough.
    #[test]
    fn hyperlinks_over_the_frame_budget_keep_the_frame() {
        let mut terminal = Terminal::new(100, 24).unwrap();
        let uri = format!("https://example.test/{}", "x".repeat(2000));
        terminal
            .vt_write(format!("\x1b]8;;{uri}\x1b\\{}\x1b]8;;\x1b\\", "L".repeat(2399)).as_bytes());
        let frame = FrameExtractor::new(false)
            .unwrap()
            .frame(1, &terminal)
            .unwrap();
        let cells: Vec<_> = frame
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .take(2399)
            .collect();
        assert!(cells.iter().all(|cell| cell.text == "L"));
        let kept = cells
            .iter()
            .take_while(|cell| cell.hyperlink == uri)
            .count();
        assert!(kept > 0 && kept < cells.len());
        assert!(cells[kept..].iter().all(|cell| cell.hyperlink.is_empty()));
        assert!(frame.encode().unwrap().len() + uri.len() > FRAME_BUDGET_BYTES);
    }

    /// A frame whose cell text exceeds its budget keeps every cell's first
    /// codepoint and full text in reading order while it fits, before any
    /// link (ORB-C6). A full screen of cells with 64 combining marks passes
    /// the 4 MiB budget; a small budget keeps the debug build fast.
    #[test]
    fn cell_text_over_the_frame_budget_keeps_first_codepoints() {
        let mut terminal = Terminal::new(10, 5).unwrap();
        let marked = format!("a{}", "\u{20D0}".repeat(64));
        let uri = format!("https://example.test/{}", "x".repeat(130));
        terminal.vt_write(
            format!(
                "\x1b]8;;{uri}\x1b\\{marked}\x1b]8;;\x1b\\{}",
                marked.repeat(48)
            )
            .as_bytes(),
        );
        let mut extractor = FrameExtractor::new(false).unwrap();
        let whole = extractor
            .frame(1, &terminal)
            .unwrap()
            .encode()
            .unwrap()
            .len();
        let extra = marked.len() - 1;
        let budget = whole - 49 * extra - uri.len() + 3 * extra + 100;
        let frame = extractor.frame_within(1, &terminal, budget).unwrap();
        let cells: Vec<_> = frame.rows.iter().flat_map(|row| &row.cells).collect();
        assert!(cells[..3].iter().all(|cell| cell.text == marked));
        assert!(cells[3..49].iter().all(|cell| cell.text == "a"));
        assert!(cells.iter().all(|cell| cell.hyperlink.is_empty()));
        assert!(frame.encode().unwrap().len() <= budget);
    }

    /// Rows read directly from libghostty's screen equal the canonical frame's
    /// rows, for rich text that keeps protection, spacer-head and wide-cell
    /// width roles, colors and a hyperlink, before and after a resize reflows
    /// it (ORB-C6).
    #[test]
    fn direct_rows_match_rich_canonical_frame_rows() {
        let mut terminal = Terminal::new(12, 4).unwrap();
        terminal.set_scrollback_max_bytes(Some(100)).unwrap();
        terminal.vt_write(
            "\x1b]8;;https://example.test\x1b\\\x1b[1\"q\x1b[1;38;2;12;34;56;48;5;17mAe\u{301}界1234567界wrap-me-long\x1b[0m\x1b[0\"q\x1b]8;;\x1b\\\r\n\x1b[48;2;5;6;7m\x1b[2K\x1b[0m\r\nplain\r\ntail-1\r\ntail-2\r\ntail-3"
                .as_bytes(),
        );
        let mut extractor = FrameExtractor::new(false).unwrap();
        let mut frame_at_top = |terminal: &mut Terminal<'static, '_>, revision| {
            terminal.scroll_viewport(ScrollViewport::Top);
            let frame = extractor.frame(revision, terminal).unwrap();
            session::encode_server_message(&ServerMessage::Frame(Box::new(frame.clone()))).unwrap();
            for (y, expected) in frame.rows.iter().enumerate() {
                let direct = extractor
                    .row(terminal, y as u32, frame.dimensions.cols)
                    .unwrap();
                assert_eq!(direct, *expected, "row {y} at revision {revision}");
            }
            frame
        };

        let frame = frame_at_top(&mut terminal, 1);
        let cells = || frame.rows.iter().flat_map(|row| &row.cells);
        assert!(frame.rows.iter().any(|row| row.wrapped));
        assert!(cells().any(|cell| cell.hyperlink == "https://example.test"));
        assert!(cells().any(|cell| cell.style.protected));
        assert!(cells().any(|cell| {
            cell.style.foreground
                == StyleColor::Rgb(Rgb {
                    r: 12,
                    g: 34,
                    b: 56,
                })
        }));
        assert!(
            cells().any(|cell| cell.style.background == StyleColor::Rgb(Rgb { r: 0, g: 0, b: 95 }))
        );
        assert!(
            cells().any(|cell| cell.style.background == StyleColor::Rgb(Rgb { r: 5, g: 6, b: 7 }))
        );
        assert!(cells().any(|cell| cell.text == "e\u{301}"));
        assert!(cells().any(|cell| cell.width == CellWidth::Wide));
        assert!(cells().any(|cell| cell.width == CellWidth::SpacerTail));
        assert!(cells().any(|cell| cell.width == CellWidth::SpacerHead));

        // Narrower columns reflow the wrapped rich text; its rows still match
        // and keep their protection and hyperlink.
        terminal.resize(9, 4, 8, 16).unwrap();
        let reflowed = frame_at_top(&mut terminal, 2);
        assert_eq!(reflowed.dimensions.cols, 9);
        let cells = || reflowed.rows.iter().flat_map(|row| &row.cells);
        assert!(reflowed.rows.iter().filter(|row| row.wrapped).count() >= 2);
        assert!(cells().any(|cell| cell.style.protected));
        assert!(cells().any(|cell| cell.hyperlink == "https://example.test"));
    }

    /// A frame's scroll position follows libghostty's viewport through output
    /// below a held viewport, the alternate screen, reflow, byte-budget
    /// eviction, `CSI 3 J` and reset (ORB-C8).
    #[test]
    fn scroll_position_follows_reflow_eviction_and_clears() {
        let mut terminal = Terminal::new(8, 4).unwrap();
        terminal.set_scrollback_max_bytes(Some(64 * 1024)).unwrap();
        let mut extractor = FrameExtractor::new(false).unwrap();
        let mut frame = |terminal: &Terminal<'static, '_>| extractor.frame(0, terminal).unwrap();
        let position = |rows_from_live, history_rows| ScrollPosition {
            rows_from_live,
            history_rows,
        };
        for line in 0..100 {
            terminal.vt_write(format!("{line:07}\r\n").as_bytes());
        }
        assert_eq!(frame(&terminal).scroll_position, position(0, 97));

        // Output below a held viewport keeps its rows and deepens it.
        terminal.scroll_viewport(ScrollViewport::Delta(-3));
        let held = frame(&terminal);
        assert_eq!(held.scroll_position, position(3, 97));
        terminal.vt_write(b"0000100\r\n0000101\r\n");
        let output = frame(&terminal);
        assert_eq!(output.rows, held.rows);
        assert_eq!(output.scroll_position, position(5, 99));

        // The alternate screen has no history; leaving it restores the primary viewport.
        terminal.vt_write(b"\x1b[?1049hAlternate");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
        terminal.vt_write(b"\x1b[?1049l");
        let primary = frame(&terminal);
        assert_eq!(primary.scroll_position, output.scroll_position);
        assert_eq!(primary.rows, output.rows);

        // Narrower columns reflow history into more rows.
        terminal.resize(4, 4, 8, 16).unwrap();
        let reflowed = frame(&terminal).scroll_position;
        assert!(reflowed.history_rows > primary.scroll_position.history_rows);
        assert!(reflowed.rows_from_live > primary.scroll_position.rows_from_live);

        // Output past the byte budget evicts the oldest rows under a viewport
        // held at the top. Wider rows fill libghostty's pages in fewer rows,
        // each of which costs about a millisecond in a debug build.
        terminal.resize(80, 4, 8, 16).unwrap();
        terminal.scroll_viewport(ScrollViewport::Top);
        let oldest = frame(&terminal);
        for line in 102..1_602 {
            terminal.vt_write(format!("{line:07}\r\n").as_bytes());
        }
        let evicted = frame(&terminal);
        assert!(evicted.scroll_position.history_rows < 1_500);
        assert_eq!(
            evicted.scroll_position.rows_from_live,
            evicted.scroll_position.history_rows
        );
        assert_ne!(evicted.rows, oldest.rows);

        // CSI 3 J and reset leave no history.
        terminal.scroll_viewport(ScrollViewport::Bottom);
        assert_eq!(frame(&terminal).scroll_position.rows_from_live, 0);
        terminal.vt_write(b"\x1b[3J");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
        terminal.vt_write(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        terminal.scroll_viewport(ScrollViewport::Delta(-1));
        let refilled = frame(&terminal).scroll_position;
        assert!(refilled.history_rows > 0 && refilled.rows_from_live == 1);
        terminal.vt_write(b"\x1bc");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
    }

    #[test]
    fn vertical_preview_reads_history_nearest_first() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .set_scrollback_max_bytes(Some(SCROLLBACK_BUDGET_BYTES))
            .unwrap();
        terminal.vt_write(
            b"\x1b]8;;https://example.test\x1b\\one\x1b]8;;\x1b\\\r\n\x1b]8;;http://x/\xff\x1b\\two\x1b]8;;\x1b\\\r\nthree\r\nlongerthan8\r\nlast",
        );
        let extractor = FrameExtractor::new(false).unwrap();
        let text = |row: &Row| {
            row.cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        };
        let up = viewport_preview(&terminal, &extractor, VerticalDirection::Up).unwrap();
        let encoded =
            session::encode_server_message(&ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: 3,
                direction: VerticalDirection::Up,
                outcome: up,
            }))
            .unwrap();
        let ServerMessage::VerticalPreview(VerticalPreview {
            outcome:
                PreviewOutcome::Viewport {
                    cols: 8,
                    edge_reached: false,
                    rows,
                },
            ..
        }) = session::decode_server_message(&encoded).unwrap()
        else {
            panic!("preview was not a canonical viewport window");
        };
        let rows: Vec<_> = rows.iter().map(|row| (text(row), row.wrapped)).collect();
        assert_eq!(rows, [("longerth".into(), true), ("three".into(), false)]);

        let oldest = extractor.row(&terminal, 0, 8).unwrap();
        assert_eq!(text(&oldest), "one");
        assert_eq!(oldest.cells[0].hyperlink, "https://example.test");
        // A URI that is not UTF-8 is no link, and the text stays.
        let invalid = extractor.row(&terminal, 1, 8).unwrap();
        assert_eq!(
            (text(&invalid).as_str(), invalid.cells[0].hyperlink.as_str()),
            ("two", "")
        );
        assert_eq!(
            viewport_preview(&terminal, &extractor, VerticalDirection::Down).unwrap(),
            PreviewOutcome::Viewport {
                cols: 8,
                edge_reached: true,
                rows: Vec::new(),
            }
        );
    }
}
