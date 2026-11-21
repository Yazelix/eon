//! The canonical frames the private Rune candidate entry steps through. They are proof
//! inputs, not product settings: each step is a pure function of its index, built from
//! `orbit_protocol` types and sent through the session codec, so the model sees what a live
//! attachment would send (eon-yp6vg.7.7).

use orbit_protocol::{
    Capabilities, Cell, CellStyle, CellWidth, Colors, Cursor, CursorShape, CursorViewport,
    Dimensions, Frame, Graphics, ImageLayer, Placement, PlacementState, Rgb, Row, Screen,
    ScrollPosition, SourceRect, StyleColor, Underline,
    session::{PixelFormat, RESOURCE_CHUNK_BYTES, ResourceAnnounce, ResourceChunk, ServerMessage},
};

/// The steps, in order. A new cycle starts again at the first as a fresh attachment.
pub(crate) const STEPS: [&str; 5] = ["text", "cell", "full", "resize", "image"];

const IMAGE_RESOURCE: u64 = 1;
const IMAGE_SIDE: u32 = 64;
const BACKGROUND: Rgb = Rgb {
    r: 0x10,
    g: 0x13,
    b: 0x1a,
};
const FOREGROUND: Rgb = Rgb {
    r: 0xf0,
    g: 0xf1,
    b: 0xf5,
};

/// The messages of step `step`, in the order Orbit sends them.
pub(crate) fn messages(step: usize) -> Vec<ServerMessage> {
    match step {
        0 => vec![ServerMessage::Attached, frame(1, text_grid(80, 24))],
        1 => {
            let mut rows = text_grid(80, 24);
            rows[5].cells[10] = cell("Z", plain().bold());
            vec![frame(2, rows)]
        }
        2 => vec![frame(3, full_grid(80, 24))],
        3 => vec![frame(4, text_grid(100, 30))],
        4 => {
            let pixels = image_pixels();
            let announce =
                ResourceAnnounce::new(IMAGE_RESOURCE, 1, PixelFormat::Rgba, IMAGE_SIDE, IMAGE_SIDE)
                    .expect("the fixture image fits the resource limits");
            let mut messages = vec![ServerMessage::ResourceAnnounce(announce)];
            messages.extend(
                pixels
                    .chunks(RESOURCE_CHUNK_BYTES as usize)
                    .enumerate()
                    .map(|(index, bytes)| {
                        ServerMessage::ResourceChunk(ResourceChunk {
                            resource: IMAGE_RESOURCE,
                            index: index as u32,
                            bytes: bytes.to_vec(),
                        })
                    }),
            );
            let ServerMessage::Frame(mut image) = frame(5, text_grid(100, 30)) else {
                unreachable!()
            };
            // Over default-background cells, crossing the grid's right edge.
            image.graphics = Some(Graphics {
                cell_width: 10,
                cell_height: 20,
                placements_omitted: false,
                placements: vec![Placement {
                    resource: IMAGE_RESOURCE,
                    layer: ImageLayer::AboveText,
                    z: 0,
                    col: 94,
                    row: 20,
                    grid_cols: 10,
                    grid_rows: 5,
                    offset_x: 0,
                    offset_y: 0,
                    width: 100,
                    height: 100,
                    source: SourceRect {
                        x: 0,
                        y: 0,
                        width: IMAGE_SIDE,
                        height: IMAGE_SIDE,
                    },
                    state: PlacementState::Ready,
                }],
            });
            messages.push(ServerMessage::Frame(image));
            messages
        }
        _ => panic!("fixture step {step} is past the last"),
    }
}

/// A 64-pixel checkerboard whose right half is half transparent.
fn image_pixels() -> Vec<u8> {
    (0..IMAGE_SIDE)
        .flat_map(|y| {
            (0..IMAGE_SIDE).flat_map(move |x| {
                let alpha = if x < IMAGE_SIDE / 2 { 255 } else { 128 };
                if (x / 8 + y / 8) % 2 == 0 {
                    [30, 90, 230, alpha]
                } else {
                    [240, 220, 40, alpha]
                }
            })
        })
        .collect()
}

fn frame(revision: u64, rows: Vec<Row>) -> ServerMessage {
    let mut palette = [Rgb::BLACK; 256];
    for (index, color) in [
        (1, (0xe0, 0x50, 0x50)),
        (2, (0x22, 0xcc, 0x88)),
        (3, (0xe0, 0xc0, 0x40)),
        (4, (0x50, 0x80, 0xf0)),
        (5, (0xc0, 0x60, 0xe0)),
        (6, (0x40, 0xc8, 0xd8)),
    ] {
        palette[index] = Rgb {
            r: color.0,
            g: color.1,
            b: color.2,
        };
    }
    let cols = rows[0].cells.len() as u16;
    ServerMessage::Frame(Box::new(Frame {
        revision,
        dimensions: Dimensions {
            cols,
            rows: rows.len() as u16,
        },
        screen: Screen::Primary,
        scroll_position: ScrollPosition::default(),
        title: "Rune candidate".into(),
        working_directory: "/tmp".into(),
        capabilities: Capabilities { hyperlinks: true },
        colors: Colors {
            background: BACKGROUND,
            foreground: FOREGROUND,
            cursor: None,
            palette,
        },
        cursor: Cursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            viewport: Some(CursorViewport {
                x: 2,
                y: 1,
                at_wide_tail: false,
            }),
        },
        rows,
        graphics: None,
    }))
}

/// The styled text grid of steps 0, 1, 3 and 4, with blank default-background rows below.
fn text_grid(cols: usize, rows: usize) -> Vec<Row> {
    let lines: [Vec<(String, CellStyle)>; 8] = [
        vec![
            ("$ ".into(), plain().fg(StyleColor::Palette(2))),
            ("eon rune candidate".into(), plain().bold()),
        ],
        vec![
            ("bold".into(), plain().bold()),
            (" ".into(), plain()),
            ("italic".into(), plain().italic()),
            (" ".into(), plain()),
            ("under".into(), plain().underline(Underline::Single)),
            (" ".into(), plain()),
            ("curly".into(), plain().underline(Underline::Curly)),
            (" ".into(), plain()),
            ("inverse".into(), plain().inverse()),
            (" ".into(), plain()),
            ("strike".into(), plain().strike()),
        ],
        vec![
            ("red ".into(), plain().fg(StyleColor::Palette(1))),
            ("yellow ".into(), plain().fg(StyleColor::Palette(3))),
            ("blue ".into(), plain().fg(StyleColor::Palette(4))),
            (
                " truecolor ".into(),
                plain()
                    .fg(StyleColor::Rgb(Rgb {
                        r: 0x10,
                        g: 0x10,
                        b: 0x10,
                    }))
                    .bg(StyleColor::Rgb(Rgb {
                        r: 0xff,
                        g: 0x9e,
                        b: 0x3b,
                    })),
            ),
            (" ".into(), plain()),
            ("on magenta".into(), plain().bg(StyleColor::Palette(5))),
        ],
        vec![("e\u{301} combining, emoji \u{1F680} done".into(), plain())],
        vec![(
            "\u{e0b0}\u{e0b2} \u{f115} nerd \u{e7a8} symbols".into(),
            plain().fg(StyleColor::Palette(6)),
        )],
        vec![("┌──┬──┐ ╭──╮ ▀▄█▌▐░▒▓ ⣿⡇ ═║╔╗".into(), plain())],
        vec![("selected span".into(), plain().selected())],
        vec![(
            "padding and blank cells below stay the terminal background".into(),
            plain().faint(),
        )],
    ];
    let mut grid: Vec<Row> = lines.iter().map(|line| styled_row(line, cols)).collect();
    grid.resize_with(rows, || blank_row(cols));
    grid.truncate(rows);
    // The last cell marks the grid's corner, so a resize changes what is drawn.
    grid[rows - 1].cells[cols - 1] = cell(
        "◢",
        plain()
            .fg(StyleColor::Palette(3))
            .bg(StyleColor::Palette(4)),
    );
    grid
}

/// Every cell differs from `text_grid`'s: a different glyph and colour in each.
fn full_grid(cols: usize, rows: usize) -> Vec<Row> {
    (0..rows)
        .map(|row| Row {
            wrapped: false,
            wrap_continuation: false,
            cells: (0..cols)
                .map(|col| {
                    let glyph = char::from(b'!' + ((row * 7 + col) % 90) as u8);
                    cell(
                        &glyph.to_string(),
                        plain()
                            .fg(StyleColor::Palette(1 + ((row + col) % 6) as u8))
                            .bg(StyleColor::Rgb(Rgb {
                                r: 0x20,
                                g: (row * 4) as u8,
                                b: (col * 2) as u8,
                            })),
                    )
                })
                .collect(),
        })
        .collect()
}

fn styled_row(spans: &[(String, CellStyle)], cols: usize) -> Row {
    let mut cells = Vec::with_capacity(cols);
    for (text, style) in spans {
        let mut chars = text.chars().peekable();
        while let Some(first) = chars.next() {
            let mut grapheme = first.to_string();
            while let Some(&next) = chars.peek()
                && ('\u{300}'..='\u{36f}').contains(&next)
            {
                grapheme.push(next);
                chars.next();
            }
            let wide = first == '\u{1F680}';
            cells.push(Cell {
                width: if wide {
                    CellWidth::Wide
                } else {
                    CellWidth::Narrow
                },
                ..cell(&grapheme, *style)
            });
            if wide {
                cells.push(Cell {
                    width: CellWidth::SpacerTail,
                    ..cell("", *style)
                });
            }
        }
    }
    cells.truncate(cols);
    cells.resize_with(cols, || cell("", plain()));
    Row {
        wrapped: false,
        wrap_continuation: false,
        cells,
    }
}

fn blank_row(cols: usize) -> Row {
    Row {
        wrapped: false,
        wrap_continuation: false,
        cells: (0..cols).map(|_| cell("", plain())).collect(),
    }
}

fn cell(text: &str, style: CellStyle) -> Cell {
    Cell {
        width: CellWidth::Narrow,
        style,
        text: text.into(),
        hyperlink: String::new(),
    }
}

fn plain() -> CellStyle {
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

trait Style {
    fn fg(self, color: StyleColor) -> Self;
    fn bg(self, color: StyleColor) -> Self;
    fn bold(self) -> Self;
    fn italic(self) -> Self;
    fn faint(self) -> Self;
    fn inverse(self) -> Self;
    fn strike(self) -> Self;
    fn selected(self) -> Self;
    fn underline(self, underline: Underline) -> Self;
}

impl Style for CellStyle {
    fn fg(self, foreground: StyleColor) -> Self {
        Self { foreground, ..self }
    }
    fn bg(self, background: StyleColor) -> Self {
        Self { background, ..self }
    }
    fn bold(self) -> Self {
        Self { bold: true, ..self }
    }
    fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }
    fn faint(self) -> Self {
        Self {
            faint: true,
            ..self
        }
    }
    fn inverse(self) -> Self {
        Self {
            inverse: true,
            ..self
        }
    }
    fn strike(self) -> Self {
        Self {
            strikethrough: true,
            ..self
        }
    }
    fn selected(self) -> Self {
        Self {
            selected: true,
            ..self
        }
    }
    fn underline(self, underline: Underline) -> Self {
        Self { underline, ..self }
    }
}
