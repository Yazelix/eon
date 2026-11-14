use crate::{
    Color as SceneColor, DialogScene, DrawRow, DrawStyle, GlyphRun, ImageKey, ImageResource, Scene,
    ScenePlacement, ScenePreview, SceneRect, WorkspaceFocus, WorkspaceScene,
    scene::{
        CardFill, PaneOutline, rounded_radius,
        tokens::{self, chrome_title_metrics},
    },
};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, CustomGlyph, Family, FontSystem, Metrics, Resolution,
    Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use orbit_protocol::{
    MAX_CELLS,
    session::{SurfaceSize, VerticalDirection},
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    fs::OpenOptions,
    io::{self, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use wgpu::{
    BlendState, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CompositeAlphaMode, CurrentSurfaceTexture, DeviceDescriptor,
    DeviceLostReason, Error as WgpuError, FragmentState, Instance, InstanceDescriptor, LoadOp,
    MultisampleState, Operations, PipelineCompilationOptions, PipelineLayoutDescriptor,
    PresentMode, PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, RequestAdapterOptions, ShaderModuleDescriptor, ShaderSource, StoreOp,
    Surface, SurfaceConfiguration, TextureViewDescriptor, VertexAttribute, VertexBufferLayout,
    VertexFormat, VertexState, VertexStepMode,
};
use winit::{dpi::PhysicalSize, event_loop::ActiveEventLoop, window::Window};

mod cells;
mod chrome;
mod cursor_trail;
mod fonts;
mod images;
mod palette;

use cells::{
    BRAILLE_FAMILY, CellFont, CellTextKind, block_span, build_scene_cursor, build_scene_rectangles,
    cell_ink_right, cell_text_attrs, cell_text_segments, cursor_bounds, cursor_outline,
    fitted_cell_font, set_cell_layer, shape_cell_text, symbol_spans,
};
pub use chrome::read_eon_icon;
use chrome::{HeaderInteraction, Spinners, fit_header_text, rasterize_bar_icon, spinners_turn};
use cursor_trail::{CursorAnimation, build_tail_cursor};
use images::{BandItem, DrawStep, ImageDraw, Images, draw_steps};

pub use fonts::{FontSettings, FontSetup};
pub use images::ImageResync;

const VERTEX_SIZE: u64 = 24;
#[cfg(test)]
const VERTICES_PER_QUAD: u32 = 6;
const DEVICE_LOST: &str = "the Venus GPU device was lost";
const DEFAULT_METRICS: CellMetrics = CellMetrics {
    scale: 1.0,
    width: 10.0,
    height: 18.0,
    font_size: 16.0,
    padding: 12.0,
};

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vertex(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.color = color;
    return output;
}

@fragment
fn fragment_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}

fn srgb_channel_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

@fragment
fn fragment_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    let rgb = vec3<f32>(
        srgb_channel_to_linear(input.color.r),
        srgb_channel_to_linear(input.color.g),
        srgb_channel_to_linear(input.color.b),
    );
    return vec4<f32>(rgb, input.color.a);
}
"#;

/// Runtime-resolved position of panel headings; pane headers keep their own geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelHeading {
    Notch,
    #[default]
    Connected,
}

impl std::str::FromStr for PanelHeading {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "notch" => Ok(Self::Notch),
            "connected" => Ok(Self::Connected),
            _ => Err(()),
        }
    }
}

/// Physical cell layout used by rendering, input mapping, and resize requests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub padding: f32,
}

impl CellMetrics {
    #[must_use]
    pub fn for_scale(scale_factor: f64) -> Self {
        DEFAULT_METRICS.scaled(scale_factor)
    }

    /// Admit the whole-cell grid and padding sent to Orbit for native dimensions.
    pub fn surface_size(self, screen: PhysicalSize<u32>) -> Option<SurfaceSize> {
        if screen.width == 0
            || screen.height == 0
            || screen.width > u32::from(u16::MAX)
            || screen.height > u32::from(u16::MAX)
        {
            return None;
        }
        let cell_width = self.width.round() as u32;
        let cell_height = self.height.round() as u32;
        let padding = self.padding.round() as u32;
        let horizontal_padding = padding.checked_mul(2)?;
        let vertical_padding = padding.checked_mul(2)?;
        if horizontal_padding >= screen.width || vertical_padding >= screen.height {
            return None;
        }
        let columns = (screen.width - horizontal_padding) / cell_width;
        let mut rows = (screen.height - vertical_padding) / cell_height;
        if columns == 0 || rows == 0 {
            return None;
        }
        rows = rows.min(MAX_CELLS as u32 / columns);
        Some(SurfaceSize {
            cols: u16::try_from(columns).ok()?,
            rows: u16::try_from(rows).ok()?,
            screen_width: screen.width,
            screen_height: screen.height,
            cell_width,
            cell_height,
            padding_top: padding,
            padding_bottom: screen.height - padding - rows * cell_height,
            padding_left: padding,
            padding_right: screen.width - padding - columns * cell_width,
        })
    }

    pub(crate) fn grid_rect(self, viewport: SceneRect, columns: u16, rows: u16) -> SceneRect {
        SceneRect {
            left: viewport.left + self.padding,
            top: viewport.top + self.padding,
            width: f32::from(columns) * self.width,
            height: f32::from(rows) * self.height,
        }
    }

    fn scaled(self, scale_factor: f64) -> Self {
        let scale = scale_factor as f32;
        Self {
            scale,
            width: (self.width * scale).round().max(1.0),
            height: (self.height * scale).round().max(1.0),
            font_size: (self.font_size * scale).max(1.0),
            padding: (self.padding * scale).round(),
        }
    }
}

/// Result of one bounded surface presentation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented(Option<RenderTimings>),
    Deferred,
    Occluded,
    Recovered,
}

/// Opt-in host elapsed times; these do not measure GPU completion or display latency.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderTimings {
    pub present_ns: u64,
    pub render_ns: u64,
    pub rebuild_ns: u64,
    pub prepare_ns: u64,
    pub acquire_ns: u64,
    pub submit_ns: u64,
    pub full_rebuild: bool,
}

struct FrameTimingRow {
    revision: Option<u64>,
    receive_ns: Option<u64>,
    scene_ns: u64,
    render: RenderTimings,
    dropped: u64,
}

/// A bounded diagnostic writer; the event loop never waits for disk I/O.
#[doc(hidden)]
#[derive(Default)]
pub struct FrameTimings {
    sender: Option<SyncSender<FrameTimingRow>>,
    writer: Option<JoinHandle<()>>,
    dropped: u64,
}

impl FrameTimings {
    pub fn from_environment() -> Self {
        Self::new(crate::transport::frame_timings_path())
    }

    fn new(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let path = path.to_owned();
        let (sender, receiver) = mpsc::sync_channel(256);
        match thread::Builder::new()
            .name("venus-frame-timings".into())
            .spawn(move || {
                if let Err(error) = write_frame_rows(&path, receiver) {
                    eprintln!("Venus frame timings disabled: {error}");
                }
            }) {
            Ok(writer) => Self {
                sender: Some(sender),
                writer: Some(writer),
                dropped: 0,
            },
            Err(error) => {
                eprintln!("Venus frame timings disabled: {error}");
                Self::default()
            }
        }
    }

    pub fn record(
        &mut self,
        outcome: PresentOutcome,
        revision: Option<u64>,
        receive_ns: Option<u64>,
        scene_ns: u64,
    ) {
        let (Some(sender), PresentOutcome::Presented(Some(render))) = (&self.sender, outcome)
        else {
            return;
        };
        let row = FrameTimingRow {
            revision,
            receive_ns,
            scene_ns,
            render,
            dropped: self.dropped,
        };
        match sender.try_send(row) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => self.dropped = self.dropped.saturating_add(1),
            Err(mpsc::TrySendError::Disconnected(_)) => self.sender = None,
        }
    }
}

impl Drop for FrameTimings {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(writer) = self.writer.take()
            && writer.join().is_err()
        {
            eprintln!("Venus frame timings writer failed");
        }
    }
}

fn write_frame_rows(path: &Path, receiver: mpsc::Receiver<FrameTimingRow>) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("frame timings require a regular file"));
    }
    let mut writer = BufWriter::new(file);
    for row in receiver {
        let render = row.render;
        serde_json::to_writer(
            &mut writer,
            &serde_json::json!({
                "v": 1,
                "revision": row.revision,
                "receive_ns": row.receive_ns,
                "present_ns": render.present_ns,
                "scene_ns": row.scene_ns,
                "render_ns": render.render_ns,
                "rebuild_ns": render.rebuild_ns,
                "prepare_ns": render.prepare_ns,
                "acquire_ns": render.acquire_ns,
                "submit_ns": render.submit_ns,
                "full_rebuild": render.full_rebuild,
                "dropped": row.dropped,
            }),
        )?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// GPU and text failure suitable for the client-visible bounded status path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError(String);

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for RenderError {}

enum TextBuffer {
    Owned(Box<Buffer>),
    Row { row: Arc<ShapedRow>, index: usize },
}

impl From<Buffer> for TextBuffer {
    fn from(buffer: Buffer) -> Self {
        Self::Owned(Box::new(buffer))
    }
}

impl std::ops::Deref for TextBuffer {
    type Target = Buffer;

    fn deref(&self) -> &Buffer {
        match self {
            Self::Owned(buffer) => buffer,
            Self::Row { row, index } => &row.buffers[*index].0,
        }
    }
}

impl TextBuffer {
    fn mutable(&mut self) -> &mut Buffer {
        match self {
            Self::Owned(buffer) => buffer,
            Self::Row { .. } => unreachable!("terminal row buffers are immutable"),
        }
    }
}

struct PlacedText {
    buffer: TextBuffer,
    custom_glyph: Option<CustomGlyph>,
    left: f32,
    top: f32,
    right: i32,
    bottom: i32,
    bound_left: i32,
    bound_top: i32,
    color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// shortcut: reuse only the same row slot, extend to moved rows in the scroll-handoff slice.
struct RowTextSlot {
    terminal: usize,
    preview: Option<bool>,
    row: usize,
}

#[derive(Clone, Copy, PartialEq)]
struct RowTextContext {
    metrics: CellMetrics,
    cell_font: CellFont,
    family: Option<&'static str>,
    columns: u16,
    screen: orbit_protocol::Screen,
}

struct ShapedCell {
    run: GlyphRun,
    layers: std::ops::Range<usize>,
}

struct ShapedRow {
    cells: Vec<ShapedCell>,
    buffers: Vec<(Buffer, bool)>,
}

struct CachedRowText {
    content: DrawRow,
    context: RowTextContext,
    shaped: Option<Arc<ShapedRow>>,
    used: bool,
}

#[derive(Default)]
struct RowTextCache {
    rows: HashMap<RowTextSlot, CachedRowText>,
}

impl RowTextCache {
    fn begin(&mut self) {
        for row in self.rows.values_mut() {
            row.used = false;
        }
    }

    fn finish(&mut self) {
        // Only drawn rows survive: no history margin beyond the two partial clip edges.
        self.rows.retain(|_, row| row.used);
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        slot: RowTextSlot,
        row: &DrawRow,
        context: RowTextContext,
        fonts: &mut FontSystem,
        blink_visible: bool,
        origin: SceneRect,
        top: f32,
        clip: SceneRect,
        text: &mut Vec<PlacedText>,
    ) {
        let metrics = context.metrics;
        if !row_intersects_clip(top, metrics.height, clip) {
            return;
        }
        let cached = self.rows.entry(slot).or_insert_with(|| CachedRowText {
            content: row.clone(),
            context,
            shaped: None,
            used: true,
        });
        let changed = cached.content != *row || cached.context != context;
        let bypass = changed
            && cached.context == context
            && !row.cells.is_empty()
            && cached.content.cells.len() == row.cells.len()
            && cached
                .content
                .cells
                .iter()
                .zip(&row.cells)
                .all(|(old, new)| old != new);
        if changed {
            cached.shaped = None;
            copy_row_key(&mut cached.content, row);
            cached.context = context;
        }
        cached.used = true;
        let shaped = if bypass {
            // Fully dirty rows have nothing to reuse; retain only their key, not their shapes.
            Arc::new(shape_row_text(row, context, fonts))
        } else {
            Arc::clone(
                cached
                    .shaped
                    .get_or_insert_with(|| Arc::new(shape_row_text(row, context, fonts))),
            )
        };
        let mut visible = shaped
            .cells
            .iter()
            .filter(|cell| cell.run.style.foreground_visible(blink_visible))
            .peekable();
        while let Some(cell) = visible.next() {
            let run = &cell.run;
            let left = origin.left + metrics.padding + f32::from(run.column) * metrics.width;
            let ink_width = f32::from(
                cell_ink_right(run, visible.peek().map(|cell| &cell.run), context.columns)
                    - run.column,
            ) * metrics.width;
            let Some(bounds) = (SceneRect {
                left,
                top,
                width: ink_width.max(1.0),
                height: metrics.height.max(1.0),
            })
            .intersection(clip) else {
                continue;
            };
            let bounds = text_clip_bounds(bounds);
            for index in cell.layers.clone() {
                let box_drawing = shaped.buffers[index].1;
                text.push(PlacedText {
                    buffer: TextBuffer::Row {
                        row: Arc::clone(&shaped),
                        index,
                    },
                    custom_glyph: None,
                    left,
                    top: top
                        - if box_drawing {
                            0.0
                        } else {
                            context.cell_font.top_offset
                        },
                    bound_left: bounds.left,
                    bound_top: bounds.top,
                    right: bounds.right,
                    bottom: bounds.bottom,
                    color: Color::rgba(
                        run.style.foreground.r,
                        run.style.foreground.g,
                        run.style.foreground.b,
                        run.style.foreground_alpha(),
                    ),
                });
            }
        }
    }
}

// Repeated full output must not allocate another set of cell strings for cache keys.
fn copy_row_key(target: &mut DrawRow, source: &DrawRow) {
    let DrawRow {
        wrapped,
        wrap_continuation,
        cells,
    } = source;
    if target.cells.len() != cells.len() {
        target.clone_from(source);
        return;
    }
    target.wrapped = *wrapped;
    target.wrap_continuation = *wrap_continuation;
    for (target, source) in target.cells.iter_mut().zip(cells) {
        let crate::DrawCell {
            width,
            text,
            hyperlink,
            style,
        } = source;
        target.width = *width;
        target.style = *style;
        target.text.clone_from(text);
        target.hyperlink.clone_from(hyperlink);
    }
}

fn shape_row_text(row: &DrawRow, context: RowTextContext, fonts: &mut FontSystem) -> ShapedRow {
    let mut runs = Vec::new();
    row.append_glyph_runs(0, &mut runs);
    let mut shaped = ShapedRow {
        cells: Vec::with_capacity(runs.len()),
        buffers: Vec::with_capacity(runs.len()),
    };
    for run in runs {
        let start = shaped.buffers.len();
        if block_span(&run.text, run.columns == 1).is_none() {
            for (buffer, box_drawing) in shape_cell_layers(
                fonts,
                context.metrics,
                context.cell_font,
                context.family,
                &run.text,
                f32::from(run.columns) * context.metrics.width,
                context.metrics.height,
                run.style,
                run.columns == 2,
            )
            .into_iter()
            .zip([false, true])
            {
                if let Some(buffer) = buffer {
                    shaped.buffers.push((buffer, box_drawing));
                }
            }
        }
        shaped.cells.push(ShapedCell {
            run,
            layers: start..shaped.buffers.len(),
        });
    }
    shaped
}

#[derive(Debug, PartialEq)]
struct ContentKey {
    generation: u64,
    /// Other columns' frame revisions, which repaint without touching the selected terminal.
    column_revisions: Vec<Option<u64>>,
    scroll_offset: f32,
    workspace_focus: WorkspaceFocus,
    text_blink_visible: bool,
    preedit: String,
    status: String,
    hyperlink: Option<(u16, u16)>,
    header: HeaderInteraction,
    scrollback_label: Option<String>,
    /// The whole dialog, since its rows and kind change without a new generation.
    dialog: Option<DialogScene>,
    /// CPU, RAM, disk and GPU use and the clock change without a new presentation generation. The
    /// readings key on their accessible descriptions, which hold both the use the label shows and
    /// every fact the hover lists.
    cpu: Option<String>,
    ram: Option<String>,
    disk: Option<String>,
    gpu: Option<String>,
    clock: Option<String>,
    /// Quota hovers name resets against the local day, which turns without a new generation.
    quota_tooltips: Vec<String>,
    /// Each drawn terminal's image band, the selected terminal's first, then the other columns'.
    /// An upload, a freed texture, or a marker coming due changes it, so it stands in for a
    /// version of the resident set (VEN-C31).
    images: Vec<Vec<BandItem>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CursorPoint {
    x: f32,
    y: f32,
}

fn rect_corners(rect: SceneRect) -> [CursorPoint; 4] {
    [
        CursorPoint {
            x: rect.left,
            y: rect.top,
        },
        CursorPoint {
            x: rect.right(),
            y: rect.top,
        },
        CursorPoint {
            x: rect.right(),
            y: rect.bottom(),
        },
        CursorPoint {
            x: rect.left,
            y: rect.bottom(),
        },
    ]
}

fn valid_rect(rect: SceneRect) -> bool {
    [rect.left, rect.top, rect.width, rect.height]
        .into_iter()
        .all(f32::is_finite)
        && rect.width > 0.0
        && rect.height > 0.0
}

fn shifted(mut rect: SceneRect, vertical: f32) -> SceneRect {
    rect.top += vertical;
    rect
}

/// How terminal edge backgrounds fill the space around the displayed grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaddingColor {
    #[default]
    Extend,
    ExtendAlways,
    Background,
}

impl std::str::FromStr for PaddingColor {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "extend" => Ok(Self::Extend),
            "extend-always" => Ok(Self::ExtendAlways),
            "background" => Ok(Self::Background),
            _ => Err("padding color must be extend, extend-always, or background"),
        }
    }
}

fn build_padding_rectangles(
    rectangles: &mut RectangleBatch,
    scene: &Scene,
    metrics: CellMetrics,
    viewport: SceneRect,
    bounds: SceneRect,
    padding_color: PaddingColor,
    (preview, offset): (Option<&ScenePreview>, f32),
) {
    if padding_color == PaddingColor::Background || scene.columns == 0 || scene.rows == 0 {
        return;
    }
    let grid = scene_grid(scene, viewport, metrics);
    let columns = usize::from(scene.columns);
    let push = |rectangles: &mut RectangleBatch, rect: SceneRect, style: DrawStyle| {
        // Default cells already have the backdrop's configured opacity.
        if !style.background_is_default {
            rectangles.push(
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                style.background,
                1.0,
            );
        }
    };
    let origin = shifted(viewport, offset);
    let rows = scene
        .content
        .iter()
        .take(usize::from(scene.rows))
        .enumerate()
        .map(|(index, row)| (grid.top + offset + index as f32 * metrics.height, row))
        .chain(
            preview_rows(preview)
                .into_iter()
                .flat_map(move |(direction, rows)| {
                    rows.iter().enumerate().map(move |(index, row)| {
                        (
                            preview_row_top(scene, direction, index, metrics, origin),
                            row,
                        )
                    })
                }),
        )
        .filter(|(top, _)| row_intersects_clip(*top, metrics.height, grid));
    for (top, row) in rows.clone() {
        for (cell, left, right) in [
            (row.cells.first(), bounds.left, grid.left),
            (row.cells.get(columns - 1), grid.right(), bounds.right()),
        ] {
            if let Some(cell) = cell {
                push(
                    rectangles,
                    SceneRect {
                        left,
                        top: top.max(grid.top),
                        width: right - left,
                        height: (top + metrics.height).min(grid.bottom()) - top.max(grid.top),
                    },
                    cell.style,
                );
            }
        }
    }
    for (edge, top, bottom) in [
        (grid.top, bounds.top, grid.top),
        (grid.bottom(), grid.bottom(), bounds.bottom()),
    ] {
        let Some((_, row)) = rows.clone().min_by(|(a, _), (b, _)| {
            (a + metrics.height / 2.0 - edge)
                .abs()
                .total_cmp(&(b + metrics.height / 2.0 - edge).abs())
        }) else {
            continue;
        };
        if padding_color == PaddingColor::Extend
            && scene.screen == orbit_protocol::Screen::Primary
            && row.cells.iter().any(|cell| {
                cell.style.background_is_default
                    || cell.style.background == scene.background
                    || cell.text.chars().any(|ch| {
                        matches!(ch,
                        '\u{e0b0}'..='\u{e0c8}' | '\u{e0ca}' | '\u{e0cc}'..='\u{e0d2}' | '\u{e0d4}')
                    })
            })
        {
            continue;
        }
        for (column, cell) in row.cells.iter().take(columns).enumerate() {
            let left = if column == 0 {
                bounds.left
            } else {
                grid.left + column as f32 * metrics.width
            };
            let right = if column + 1 == columns {
                bounds.right()
            } else {
                grid.left + (column + 1) as f32 * metrics.width
            };
            push(
                rectangles,
                SceneRect {
                    left,
                    top,
                    width: right - left,
                    height: bottom - top,
                },
                cell.style,
            );
        }
    }
}

fn scene_grid(scene: &Scene, viewport: SceneRect, metrics: CellMetrics) -> SceneRect {
    metrics.grid_rect(viewport, scene.columns, scene.rows)
}

fn workspace_paint<'a>(
    workspace: &'a WorkspaceScene,
    scene: Option<&Scene>,
    columns: &[Option<&Scene>],
    metrics: CellMetrics,
) -> Cow<'a, WorkspaceScene> {
    // This grid also clips shifted cells and scroll previews in rebuild_if_needed.
    let selected = scene.map(|scene| {
        (
            workspace.terminal,
            scene_grid(scene, workspace.terminal, metrics),
        )
    });
    let peers = workspace
        .other_columns
        .iter()
        .zip(columns)
        .filter_map(|(column, scene)| {
            Some((
                column.terminal,
                scene_grid((*scene)?, column.terminal, metrics),
            ))
        });
    workspace.with_displayed_grids(metrics, selected.into_iter().chain(peers))
}

/// Each card behind the workspace's terminals with its background: the selected column's, the
/// other columns', then the agent sidebar's last.
fn card_backdrops(
    workspace: &WorkspaceScene,
    scene: Option<&Scene>,
    columns: &[Option<&Scene>],
    metrics: CellMetrics,
    stroke_width: f32,
) -> Vec<(Option<CardFill>, SceneColor)> {
    let mut cards = std::iter::once((
        workspace.card_fill(workspace.chrome, metrics, stroke_width),
        !workspace.panes.is_empty() || workspace.popup_label().is_some(),
        scene,
    ))
    .chain(
        workspace
            .other_columns
            .iter()
            .zip(columns)
            .map(|(column, scene)| {
                (
                    workspace.card_fill(column.chrome, metrics, stroke_width),
                    !column.panes.is_empty(),
                    *scene,
                )
            }),
    )
    .map(|(card, shown, scene)| {
        (
            shown.then_some(card),
            scene.map_or(palette::TERMINAL_FALLBACK, |scene| scene.background),
        )
    })
    .collect::<Vec<_>>();
    // The agent sidebar's card takes the background of the card beside it (VEN-C30),
    // so it reads like the panes and blur shows through it as through them.
    if workspace.sidebar().is_some() {
        let beside = cards
            .iter()
            .filter_map(|(card, background)| card.map(|card| (card, background)))
            .min_by(|left, right| left.0.bounds.left.total_cmp(&right.0.bounds.left))
            .map_or(palette::TERMINAL_FALLBACK, |(_, background)| *background);
        cards.push((
            Some(CardFill {
                bounds: workspace.card_rect(
                    workspace.sidebar_area().unwrap(),
                    metrics,
                    stroke_width,
                ),
                radius: workspace.card_radius(),
            }),
            beside,
        ));
    }
    cards
}

/// Paints `frame` opaque bar color where no card covers it, and each card in its background at
/// the terminal-background `opacity`, so the frame reads as one piece around translucent cards.
fn push_card_backdrop(
    rectangles: &mut RectangleBatch,
    frame: SceneRect,
    cards: &[(Option<CardFill>, SceneColor)],
    opacity: f32,
) {
    let rects = cards
        .iter()
        .filter_map(|(card, _)| card.map(|card| card.bounds))
        .collect::<Vec<_>>();
    for rect in uncovered(frame, &rects) {
        rectangles.push(
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            palette::BAR_BACKGROUND,
            1.0,
        );
    }
    for (fill, background) in cards {
        let Some(fill) = fill else { continue };
        for band in fill.bands() {
            rectangles.push(
                band.left,
                band.top,
                band.width,
                band.height,
                *background,
                opacity,
            );
        }
        let card = fill.bounds;
        let radius = rounded_radius(card, fill.radius);
        // Partition each corner row: an opaque underpaint would apply alpha twice.
        for (top, inset) in corner_rows(card, radius) {
            for (left, width, color, alpha) in [
                (card.left, inset, palette::BAR_BACKGROUND, 1.0),
                (card.right() - inset, inset, palette::BAR_BACKGROUND, 1.0),
            ] {
                rectangles.push(left, top, width, 1.0, color, alpha);
            }
        }
    }
}

#[derive(Clone)]
struct FrameTitle {
    label: String,
    width: f32,
}

#[derive(Clone)]
struct FrameTitleTab {
    heading: PanelHeading,
    top: f32,
    start: f32,
    left_radius: f32,
    row: SceneRect,
    return_run: f32,
    title: FrameTitle,
    /// `start` of the mirrored action notch at the card's right, on headings that hold a button.
    action: Option<f32>,
}

#[derive(Clone)]
struct PaneTitleTab {
    card: CardFill,
    tab: FrameTitleTab,
}

struct PaneTitleCache {
    workspace: WorkspaceScene,
    metrics: CellMetrics,
    widths: (f32, f32),
    scrollback: Option<String>,
    tabs: Vec<Vec<Option<PaneTitleTab>>>,
}

impl PaneTitleCache {
    fn matches(
        &self,
        workspace: &WorkspaceScene,
        metrics: CellMetrics,
        widths: (f32, f32),
        scrollback: &Option<String>,
    ) -> bool {
        self.workspace == *workspace
            && self.metrics == metrics
            && self.widths == widths
            && self.scrollback == *scrollback
    }
}

impl PaneTitleTab {
    fn bands(&self, inset: f32) -> Vec<SceneRect> {
        let cut = self.card.bounds.top + rounded_radius(self.card.bounds, self.card.radius);
        title_tab_bands(self.card, &self.tab, inset)
            .into_iter()
            .map(|mut band| {
                if band.top < cut {
                    band.width = (self.card.bounds.left + self.card.bounds.right()
                        - 2.0 * band.left)
                        .max(0.0);
                }
                band
            })
            .collect()
    }

    fn outline_rects(&self, width: f32) -> Vec<(SceneRect, f32)> {
        let inner = self.bands(width);
        let replacement = self.replacement();
        self.bands(0.0)
            .into_iter()
            .filter_map(|band| band.intersection(replacement))
            .flat_map(|band| uncovered(band, &inner))
            .flat_map(|piece| span_rects(piece.left, piece.right(), piece.top))
            .collect()
    }

    fn replacement(&self) -> SceneRect {
        SceneRect {
            top: self.tab.top,
            height: self.card.bounds.top + rounded_radius(self.card.bounds, self.card.radius)
                - self.tab.top,
            ..self.card.bounds
        }
    }
}

// Fill and stroke share the contour, including the tangent return into the top rule.
fn title_tab_bands(card: CardFill, tab: &FrameTitleTab, inset: f32) -> Vec<SceneRect> {
    if tab.heading == PanelHeading::Connected {
        let cut = card.bounds.top + rounded_radius(card.bounds, card.radius);
        return CardFill {
            bounds: SceneRect {
                left: card.bounds.left + inset,
                top: tab.top + inset,
                width: (card.bounds.width - inset * 2.0).max(0.0),
                height: (card.bounds.bottom() - tab.top - inset * 2.0).max(0.0),
            },
            radius: (rounded_radius(card.bounds, card.radius) - inset).max(0.0),
        }
        .bands()
        // Outline sampling emits one raster row per band above the existing join.
        .flat_map(|band| {
            let rows = (cut - band.top).clamp(0.0, band.height) as u32;
            (0..rows)
                .map(move |row| SceneRect {
                    top: band.top + row as f32,
                    height: 1.0,
                    ..band
                })
                .chain((band.height > rows as f32).then_some(SceneRect {
                    top: band.top + rows as f32,
                    height: band.height - rows as f32,
                    ..band
                }))
        })
        .collect();
    }
    let (mut bands, body) = notch_bands(card, tab, tab.start, inset);
    if let Some(start) = tab.action {
        // The action notch is the title notch mirrored about the card's centre. Each row keeps
        // the title's band and adds only what the mirrored band reaches beyond it, starting at
        // that band's own right edge, so no pixel blends twice.
        let mirror = card.bounds.left + card.bounds.right();
        let (action, _) = notch_bands(card, tab, start, inset);
        let pieces = bands
            .iter()
            .zip(action)
            .filter_map(|(band, action)| {
                let left = (mirror - action.right()).max(band.right());
                let right = mirror - action.left;
                (right > left).then_some(SceneRect {
                    left,
                    width: right - left,
                    ..*band
                })
            })
            .collect::<Vec<_>>();
        bands.extend(pieces);
    }
    bands.extend(body);
    bands
}

/// The rows of one notch of `tab` whose flat top ends at `start`, down to where the card's
/// own corner ends, and the card's bands below them.
fn notch_bands(
    card: CardFill,
    tab: &FrameTitleTab,
    start: f32,
    inset: f32,
) -> (Vec<SceneRect>, Vec<SceneRect>) {
    let bounds = SceneRect {
        left: card.bounds.left + inset,
        top: card.bounds.top + inset,
        width: (card.bounds.width - inset * 2.0).max(0.0),
        height: (card.bounds.height - inset * 2.0).max(0.0),
    };
    let radius = (rounded_radius(card.bounds, card.radius) - inset).max(0.0);
    let left_radius = (tab.left_radius - inset).max(0.0);
    let top = tab.top + inset;
    let return_start = start;
    let cut = bounds.top + radius;
    let circle =
        |radius: f32, distance: f32| (radius * radius - distance * distance).max(0.0).sqrt();
    let mut bands = Vec::new();
    for row in top.floor() as i32..cut.ceil() as i32 {
        let y = row as f32 + 0.5;
        let left = if y < top + left_radius {
            bounds.left + left_radius - circle(left_radius, top + left_radius - y)
        } else {
            bounds.left
        };
        let end = if y >= bounds.top {
            bounds.right() - radius + circle(radius, bounds.top + radius - y)
        } else {
            let rise = card.bounds.top - tab.top;
            let radius = (tab.return_run.powi(2) + rise.powi(2)) / (4.0 * rise);
            let join_y = tab.top + rise / 2.0 + inset * (1.0 - rise / (2.0 * radius));
            if y < join_y {
                return_start + circle((radius - inset).max(0.0), tab.top + radius - y)
            } else {
                return_start + tab.return_run
                    - circle(radius + inset, y - (card.bounds.top - radius))
            }
        };
        bands.push(SceneRect {
            left,
            top: row as f32,
            width: (end - left).max(0.0),
            height: 1.0,
        });
    }
    let body = CardFill { bounds, radius }
        .bands()
        .filter(|band| band.top >= cut)
        .collect();
    (bands, body)
}

fn title_tab_outline_rects(
    card: CardFill,
    tab: &FrameTitleTab,
    width: f32,
) -> Vec<(SceneRect, f32)> {
    let inner = title_tab_bands(card, tab, width);
    let cut = card.bounds.top + rounded_radius(card.bounds, card.radius);
    title_tab_bands(card, tab, 0.0)
        .into_iter()
        .filter(|band| band.top < cut)
        .flat_map(|band| uncovered(band, &inner))
        .flat_map(|piece| span_rects(piece.left, piece.right(), piece.top))
        .collect()
}

// Partition once so the terminal background under the title never double-blends.
fn push_title_tab_backdrop(
    rectangles: &mut RectangleBatch,
    frame: SceneRect,
    cards: &[(Option<CardFill>, SceneColor)],
    opacity: f32,
    tabs: &[Option<FrameTitleTab>],
) {
    let bands = cards
        .iter()
        .enumerate()
        .flat_map(|(index, (card, background))| {
            let Some(card) = card else { return Vec::new() };
            let fill = if let Some(Some(tab)) = tabs.get(index) {
                title_tab_bands(*card, tab, 0.0)
            } else {
                card.bands().collect()
            };
            fill.into_iter()
                .map(|band| (band, *background, opacity))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let covered = bands.iter().map(|(rect, _, _)| *rect).collect::<Vec<_>>();
    for rect in uncovered(frame, &covered) {
        rectangles.push(
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            palette::BAR_BACKGROUND,
            1.0,
        );
    }
    for (rect, color, alpha) in bands {
        rectangles.push(rect.left, rect.top, rect.width, rect.height, color, alpha);
    }
}

fn terminal_paint_clip(
    workspace: &WorkspaceScene,
    allocated: SceneRect,
    terminal: SceneRect,
    metrics: CellMetrics,
    pane_frames: bool,
    stroke_width: f32,
) -> Option<(SceneRect, Option<CardFill>)> {
    // The popup card is outlined in both frame modes, so it always bounds its paint.
    let card = (pane_frames || workspace.popup_label().is_some())
        .then(|| workspace.card_fill(allocated, metrics, stroke_width));
    let viewport = card.map_or(workspace.header_viewport, |card| card.bounds);
    // Terminals scroll below the columns' Stack band.
    Some((
        terminal
            .intersection(workspace.header_viewport)?
            .intersection(viewport)?,
        card,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_workspace_padding(
    rectangles: &mut RectangleBatch,
    workspace: &WorkspaceScene,
    scene: Option<&Scene>,
    columns: &[Option<&Scene>],
    metrics: CellMetrics,
    padding_color: PaddingColor,
    pane_frames: bool,
    stroke_width: f32,
    scroll: (Option<&ScenePreview>, f32),
) {
    if padding_color == PaddingColor::Background {
        return;
    }
    let mut surfaces = std::iter::once((
        workspace.chrome,
        workspace.terminal,
        scene,
        workspace.panes.as_slice(),
    ))
    .chain(
        workspace
            .other_columns
            .iter()
            .zip(columns)
            .map(|(column, scene)| {
                (
                    column.chrome,
                    column.terminal,
                    *scene,
                    column.panes.as_slice(),
                )
            }),
    )
    .collect::<Vec<_>>();
    surfaces.sort_by(|a, b| a.1.left.total_cmp(&b.1.left));
    let spans = background_spans(
        &surfaces
            .iter()
            .map(|(_, terminal, _, _)| *terminal)
            .collect::<Vec<_>>(),
        rectangles.width as f32,
    );
    let popup = workspace.popup_label().is_some();
    // Padding stays below the columns' Stack band, where their headers scroll.
    let viewport = if pane_frames || popup {
        workspace.header_viewport
    } else {
        let left = workspace.sidebar().map_or(0.0, SceneRect::right);
        SceneRect {
            left,
            width: rectangles.width as f32 - left,
            ..workspace.header_viewport
        }
    };
    for ((allocated, terminal, scene, panes), (left, right)) in surfaces.into_iter().zip(spans) {
        let Some(scene) = scene else { continue };
        let outer =
            (pane_frames || popup).then(|| workspace.card_fill(allocated, metrics, stroke_width));
        let pane = outer.map(|outer| {
            workspace
                .pane_outlines(panes, terminal, outer, stroke_width, metrics, stroke_width)
                .into_iter()
                .find(|outline| outline.expanded)
                .map_or(outer, |outline| CardFill {
                    bounds: outline.bounds,
                    radius: outline.radius,
                })
        });
        let bounds = if popup {
            let card = outer.unwrap();
            // Popup margins are padding too; the title band retains its own background.
            SceneRect {
                left: card.bounds.left,
                width: card.bounds.width,
                height: (card.bounds.bottom() - terminal.top).max(0.0),
                ..terminal
            }
        } else if pane_frames {
            terminal
        } else {
            let left = left.max(viewport.left);
            SceneRect {
                left,
                width: right - left,
                ..terminal
            }
        };
        // Intersect the expanded pane's rounded shape with its column's outer shape.
        let bands = outer
            .into_iter()
            .flat_map(CardFill::bands)
            .chain(outer.is_none().then_some(bounds));
        rectangles.card_clip = pane;
        for band in bands {
            let Some(clip) = bounds
                .intersection(viewport)
                .and_then(|clip| clip.intersection(band))
            else {
                continue;
            };
            rectangles.clip = Some(clip);
            build_padding_rectangles(
                rectangles,
                scene,
                metrics,
                terminal,
                bounds,
                padding_color,
                if terminal == workspace.terminal {
                    scroll
                } else {
                    (None, 0.0)
                },
            );
        }
    }
    rectangles.clip = None;
    rectangles.card_clip = None;
}

fn terminal_text_clip(
    terminal: SceneRect,
    clip: SceneRect,
    metrics: CellMetrics,
    pane_frames: bool,
) -> SceneRect {
    if !pane_frames {
        return clip;
    }
    // A retained frame can be wider than the current terminal after a resize.
    let left = clip.left.max(terminal.left + metrics.padding);
    let right = clip.right().min(terminal.right() - metrics.padding);
    SceneRect {
        left,
        width: (right - left).max(0.0),
        ..clip
    }
}

fn text_clip_bounds(bounds: SceneRect) -> TextBounds {
    // A pixel belongs to the half-open fill only when its center is inside it.
    TextBounds {
        left: (bounds.left - 0.5).ceil() as i32,
        top: (bounds.top - 0.5).ceil() as i32,
        right: (bounds.right() - 0.5).ceil() as i32,
        bottom: (bounds.bottom() - 0.5).ceil() as i32,
    }
}

/// The parts of `area` that no rectangle covers, as horizontal bands; covers may overlap.
fn uncovered(area: SceneRect, cards: &[SceneRect]) -> Vec<SceneRect> {
    let cards = cards
        .iter()
        .filter_map(|card| card.intersection(area))
        .collect::<Vec<_>>();
    let mut edges = cards
        .iter()
        .flat_map(|card| [card.top, card.bottom()])
        .chain([area.top, area.bottom()])
        .collect::<Vec<_>>();
    edges.sort_by(f32::total_cmp);
    edges.dedup();
    let mut free = Vec::new();
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let mut spans = cards
            .iter()
            .filter(|card| card.top <= top && bottom <= card.bottom())
            .map(|card| (card.left, card.right()))
            .collect::<Vec<_>>();
        spans.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut left = area.left;
        for (start, end) in spans.into_iter().chain([(area.right(), area.right())]) {
            if start > left {
                free.push(SceneRect {
                    left,
                    top,
                    width: start - left,
                    height: bottom - top,
                });
            }
            left = left.max(end);
        }
    }
    free
}

/// Left and right edges of each column's background strip, for terminals ordered left to right:
/// neighbouring strips meet in the middle of the gap between them, and the outer strips reach the
/// window's sides.
fn background_spans(terminals: &[SceneRect], width: f32) -> Vec<(f32, f32)> {
    terminals
        .iter()
        .enumerate()
        .map(|(index, terminal)| {
            let left = if index == 0 {
                0.0
            } else {
                (terminals[index - 1].right() + terminal.left) / 2.0
            };
            let right = terminals
                .get(index + 1)
                .map_or(width, |next| (terminal.right() + next.left) / 2.0);
            (left, right)
        })
        .collect()
}

fn preview_rows(preview: Option<&ScenePreview>) -> Option<(VerticalDirection, &[DrawRow])> {
    let ScenePreview::Viewport {
        direction, rows, ..
    } = preview?
    else {
        return None;
    };
    Some((*direction, rows))
}

fn preview_row_top(
    scene: &Scene,
    direction: VerticalDirection,
    index: usize,
    metrics: CellMetrics,
    origin: SceneRect,
) -> f32 {
    match direction {
        VerticalDirection::Up => {
            origin.top + metrics.padding - (index as f32 + 1.0) * metrics.height
        }
        VerticalDirection::Down => {
            origin.top + metrics.padding + (f32::from(scene.rows) + index as f32) * metrics.height
        }
    }
}

fn row_intersects_clip(top: f32, height: f32, clip: SceneRect) -> bool {
    top < clip.bottom() && top + height > clip.top
}

/// Draw state for the terminal this renderer presents: hovered link, scrollback pill and cursor trail.
struct TerminalLayer {
    hyperlink: Option<(u16, u16)>,
    scrollback_label: Option<String>,
    scrollback_rect: Option<SceneRect>,
    presented_scrollback_rect: Option<SceneRect>,
    cursor_animation: CursorAnimation,
    cursor_handoff: bool,
    last_cursor_frame: Option<Instant>,
}

struct WindowedTarget {
    instance: Instance,
    surface: Surface<'static>,
    window: Arc<Window>,
}

/// One shared GPU/text owner, optionally presenting to its own native window.
pub struct Renderer {
    #[cfg(test)]
    grid_rebuilds: u64,
    #[cfg(test)]
    glyphon_prepares: u64,
    windowed: Option<WindowedTarget>,
    device: wgpu::Device,
    device_lost: Arc<AtomicBool>,
    queue: wgpu::Queue,
    config: SurfaceConfiguration,
    rectangle_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    vertex_count: u32,
    cursor_vertex_boundary: u32,
    dynamic_vertices: wgpu::Buffer,
    dynamic_vertex_capacity: u64,
    dynamic_vertex_count: u32,
    images: Images,
    fonts: FontSetup,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_overlay: Option<(usize, Vec<TextBounds>)>,
    text: Vec<PlacedText>,
    row_text: RowTextCache,
    /// The keycap popup this frame drew, for the pointer.
    hint_popup_hit: Option<chrome::HintPopupHit>,
    content_key: Option<ContentKey>,
    pane_titles: Option<PaneTitleCache>,
    notice_lines: usize,
    terminal: TerminalLayer,
    header: HeaderInteraction,
    spinners: Spinners,
    /// The Eon icon's premultiplied RGBA image, when Eon supplied a readable one.
    eon_icon: Option<Arc<[u8]>>,
    cursor_tail: Option<(SceneColor, SceneColor, f32)>,
    clear: wgpu::Color,
    background_opacity: f32,
    padding_color: PaddingColor,
    pane_frames: bool,
    panel_heading: PanelHeading,
    detect_local_hosts: bool,
    focus_color: SceneColor,
    border_width: u8,
    focus_border_width: u8,
    metrics: CellMetrics,
    /// Widest Status percentage for these fonts and metrics, shared by the bar readings.
    reading_width: f32,
    cell_font: CellFont,
}

impl Renderer {
    fn card_stroke_width(&self) -> f32 {
        physical_outline_width(
            self.focus_border_width.max(self.border_width),
            self.metrics.scale,
        )
    }

    pub fn set_scrollback_label(&mut self, label: Option<String>) {
        self.terminal.scrollback_label = label;
    }

    pub fn presented_scrollback_rect(&self) -> Option<SceneRect> {
        self.terminal.presented_scrollback_rect
    }

    pub fn set_hyperlink(&mut self, hyperlink: Option<(u16, u16)>) {
        self.terminal.hyperlink = hyperlink;
    }

    /// Notice placement shares the rendered link's geometry, keeping its target visible.
    #[must_use]
    pub fn notice_rect(&self, workspace: Option<&WorkspaceScene>) -> SceneRect {
        let height = self.metrics.height
            * if self.terminal.hyperlink.is_some() {
                3.0
            } else {
                // Wrapped status lines are 1.25 rows tall, with an eighth of a row above and below.
                self.notice_lines as f32 * tokens::WRAPPED_LINE_ROWS + 0.25
            };
        let bottom = self.config.height as f32 - height - self.metrics.height * 0.5;
        let link_bottom = self.terminal.hyperlink.map(|(row, _)| {
            workspace.map_or(0.0, |workspace| workspace.terminal.top)
                + self.metrics.padding
                + f32::from(row + 1) * self.metrics.height
        });
        SceneRect {
            left: self.metrics.padding,
            top: if link_bottom.is_some_and(|link| link > bottom) {
                self.metrics.padding
            } else {
                bottom
            },
            width: self.config.width as f32 - self.metrics.padding * 2.0,
            height,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        background_opacity: f32,
        padding_color: PaddingColor,
        cursor_tail: Option<(SceneColor, f32)>,
        pane_frames: bool,
        panel_heading: PanelHeading,
        detect_local_hosts: bool,
        focus_color: SceneColor,
        border_width: u8,
        focus_border_width: u8,
        animate_working_badge: bool,
        eon_icon: Option<Arc<[u8]>>,
        fonts: FontSetup,
    ) -> Result<Self, RenderError> {
        let size = nonzero(window.inner_size());
        let instance = Instance::new(InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(display_error("cannot create the Venus GPU surface"))?;
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(display_error("no compatible Venus GPU adapter"))?;
        let adapter_info = adapter.get_info();
        crate::transport::bench_trace(
            "gpu",
            format_args!(
                "software={} {adapter_info:?}",
                adapter_info.device_type == wgpu::DeviceType::Cpu
            ),
        );
        let spinners_turn = spinners_turn(adapter_info.device_type, animate_working_badge);
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                memory_hints: device_memory_hints(adapter_info.device_type),
                ..Default::default()
            })
            .await
            .map_err(display_error("cannot create the Venus GPU device"))?;
        let device_lost = Arc::new(AtomicBool::new(false));
        let loss_state = Arc::clone(&device_lost);
        let redraw_window = Arc::clone(&window);
        device.set_device_lost_callback(move |reason, _| {
            if record_device_loss(&loss_state, reason) {
                redraw_window.request_redraw();
            }
        });
        let loss_state = Arc::clone(&device_lost);
        device.on_uncaptured_error(Arc::new(move |error| {
            handle_uncaptured_gpu_error(&loss_state, error);
        }));
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or_else(|| RenderError("the GPU surface has no supported format".into()))?;
        let capabilities = surface.get_capabilities(&adapter);
        if let Some(format) = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = format;
        }
        config.present_mode = PresentMode::Fifo;
        config.alpha_mode = surface_alpha_mode(
            background_opacity,
            adapter_info.backend,
            &capabilities.alpha_modes,
        )?;
        ensure_device_available(&device_lost)?;
        surface.configure(&device, &config);
        ensure_device_available(&device_lost)?;

        Self::assemble(
            Some(WindowedTarget {
                instance,
                surface,
                window: Arc::clone(&window),
            }),
            device,
            device_lost,
            queue,
            config,
            window.scale_factor(),
            spinners_turn,
            background_opacity,
            padding_color,
            cursor_tail,
            pane_frames,
            panel_heading,
            detect_local_hosts,
            focus_color,
            border_width,
            focus_border_width,
            eon_icon,
            fonts,
        )
    }

    /// Uses a host's device, queue and format without creating a window or surface.
    /// The host sets the pass viewport to the slot, submits and presents, and owns
    /// device-loss handling and callbacks; this renderer does not replace them.
    pub fn hosted(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        scale_factor: f64,
        fonts: FontSetup,
    ) -> Result<Self, RenderError> {
        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: Default::default(),
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
        };
        Self::assemble(
            None,
            device,
            Arc::new(AtomicBool::new(false)),
            queue,
            config,
            scale_factor,
            false,
            1.0,
            PaddingColor::default(),
            None,
            false,
            PanelHeading::default(),
            false,
            SceneColor::default(),
            1,
            1,
            None,
            fonts,
        )
    }

    /// Prepares one terminal scene before the host opens its render pass.
    pub fn prepare_hosted(
        &mut self,
        scene: &Scene,
        blink_visible: bool,
        generation: u64,
    ) -> Result<(), RenderError> {
        self.prepare_frame(
            Some(scene),
            None,
            0.0,
            None,
            &[],
            None,
            WorkspaceFocus::Terminal,
            "",
            blink_visible,
            "",
            generation,
        )
        .map(|_| ())
    }

    /// Draws the prepared terminal into the host's current viewport.
    pub fn draw_hosted(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), RenderError> {
        self.draw_into(pass)
    }

    /// Releases frame-local atlas entries after the host has submitted the frame.
    pub fn end_frame_hosted(&mut self) {
        self.atlas.trim();
        self.terminal.presented_scrollback_rect = self.terminal.scrollback_rect;
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        windowed: Option<WindowedTarget>,
        device: wgpu::Device,
        device_lost: Arc<AtomicBool>,
        queue: wgpu::Queue,
        config: SurfaceConfiguration,
        scale_factor: f64,
        spinners_turn: bool,
        background_opacity: f32,
        padding_color: PaddingColor,
        cursor_tail: Option<(SceneColor, f32)>,
        pane_frames: bool,
        panel_heading: PanelHeading,
        detect_local_hosts: bool,
        focus_color: SceneColor,
        border_width: u8,
        focus_border_width: u8,
        eon_icon: Option<Arc<[u8]>>,
        mut fonts: FontSetup,
    ) -> Result<Self, RenderError> {
        let srgb_target = config.format.is_srgb();
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("venus rectangles"),
            source: ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("venus rectangle pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let rectangle_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("venus rectangle pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: VERTEX_SIZE,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x4,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some(if srgb_target {
                    "fragment_srgb"
                } else {
                    "fragment_linear"
                }),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = VERTEX_SIZE;
        let vertices = device.create_buffer(&BufferDescriptor {
            label: Some("venus rectangle vertices"),
            size: vertex_capacity,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dynamic_vertices = device.create_buffer(&BufferDescriptor {
            label: Some("venus dynamic cursor vertices"),
            size: vertex_capacity,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let images = Images::new(&device, config.format);

        let metrics = fonts.metrics(scale_factor);
        let reading_width = chrome::measure_reading_width(&mut fonts.font_system, metrics);
        let cell_font = fitted_cell_font(
            &mut fonts.font_system,
            metrics,
            fonts.family,
            fonts.configured,
        );
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let color_mode = if srgb_target {
            ColorMode::Accurate
        } else {
            ColorMode::Web
        };
        let mut atlas =
            TextAtlas::with_color_mode(&device, &queue, &cache, config.format, color_mode);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        ensure_device_available(&device_lost)?;
        Ok(Self {
            #[cfg(test)]
            grid_rebuilds: 0,
            #[cfg(test)]
            glyphon_prepares: 0,
            windowed,
            device,
            device_lost,
            queue,
            config,
            rectangle_pipeline,
            vertices,
            vertex_capacity,
            vertex_count: 0,
            cursor_vertex_boundary: 0,
            dynamic_vertices,
            dynamic_vertex_capacity: vertex_capacity,
            dynamic_vertex_count: 0,
            images,
            fonts,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_overlay: None,
            text: Vec::new(),
            row_text: RowTextCache::default(),
            hint_popup_hit: None,
            content_key: None,
            pane_titles: None,
            notice_lines: 1,
            terminal: TerminalLayer {
                hyperlink: None,
                scrollback_label: None,
                scrollback_rect: None,
                presented_scrollback_rect: None,
                cursor_animation: CursorAnimation::default(),
                cursor_handoff: false,
                last_cursor_frame: None,
            },
            header: HeaderInteraction::default(),
            spinners: Spinners::new(spinners_turn, focus_color),
            eon_icon,
            cursor_tail: cursor_tail
                .map(|(color, duration)| (color, cursor_outline(color), duration)),
            clear: clear_color(palette::TERMINAL_FALLBACK, background_opacity, srgb_target),
            background_opacity,
            padding_color,
            pane_frames,
            panel_heading,
            detect_local_hosts,
            focus_color,
            border_width,
            focus_border_width,
            metrics,
            reading_width,
            cell_font,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_frame(
        &mut self,
        scene: Option<&Scene>,
        scroll_preview: Option<&ScenePreview>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        dialog: Option<&DialogScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> Result<(bool, u64, u64), RenderError> {
        let rebuild_started = crate::transport::frame_time();
        crate::transport::bench_trace("rebuild_begin", "");
        let content_changed = self.rebuild_if_needed(
            scene,
            scroll_preview,
            scroll_offset,
            workspace,
            columns,
            dialog,
            workspace_focus,
            status,
            blink_visible,
            preedit,
            generation,
        );
        crate::transport::bench_trace("rebuild_end", content_changed);
        let rebuild_ns = crate::transport::frame_elapsed(rebuild_started);
        let prepare_started = crate::transport::frame_time();
        let cursors = self.build_dynamic_cursors(
            scene,
            scroll_offset,
            workspace,
            columns,
            dialog,
            blink_visible,
        );
        self.upload_dynamic_vertices(&cursors.bytes);
        if content_changed {
            #[cfg(test)]
            {
                self.glyphon_prepares += 1;
            }
            self.viewport.update(
                &self.queue,
                Resolution {
                    width: self.config.width,
                    height: self.config.height,
                },
            );
            let eon_icon = self.eon_icon.as_deref();
            if self
                .text_renderer
                .prepare_with_custom(
                    &self.device,
                    &self.queue,
                    &mut self.fonts.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    text_areas(&self.text, self.text_overlay.as_ref()),
                    &mut self.swash_cache,
                    |request| rasterize_bar_icon(request, eon_icon),
                )
                .is_err()
            {
                self.atlas.trim();
                self.text_renderer
                    .prepare_with_custom(
                        &self.device,
                        &self.queue,
                        &mut self.fonts.font_system,
                        &mut self.atlas,
                        &self.viewport,
                        text_areas(&self.text, self.text_overlay.as_ref()),
                        &mut self.swash_cache,
                        |request| rasterize_bar_icon(request, eon_icon),
                    )
                    .map_err(display_error("the Venus glyph atlas is full"))?;
            }
        }

        let prepare_ns = crate::transport::frame_elapsed(prepare_started);
        Ok((content_changed, rebuild_ns, prepare_ns))
    }

    fn draw_into(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), RenderError> {
        for step in draw_steps(
            self.vertex_count,
            (self.dynamic_vertex_count > 0).then_some(self.cursor_vertex_boundary),
            &self.images.draws,
        ) {
            match step {
                DrawStep::Rectangles(vertices) => {
                    pass.set_pipeline(&self.rectangle_pipeline);
                    pass.set_vertex_buffer(0, self.vertices.slice(..));
                    pass.draw(vertices, 0..1);
                }
                DrawStep::Cursor => {
                    pass.set_pipeline(&self.rectangle_pipeline);
                    pass.set_vertex_buffer(0, self.dynamic_vertices.slice(..));
                    pass.draw(0..self.dynamic_vertex_count, 0..1);
                }
                DrawStep::Image(index) => self.images.draw(pass, index),
            }
        }
        self.text_renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(display_error("cannot render the Venus glyph atlas"))?;
        Ok(())
    }

    #[must_use]
    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    #[must_use]
    pub fn metrics_at_scale(&self, scale: f64) -> CellMetrics {
        self.fonts.metrics(scale)
    }

    #[must_use]
    pub fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.config.width, self.config.height)
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        let size = nonzero(size);
        self.config.width = size.width;
        self.config.height = size.height;
        let metrics = self.fonts.metrics(scale_factor);
        if metrics != self.metrics {
            self.reading_width =
                chrome::measure_reading_width(&mut self.fonts.font_system, metrics);
            self.cell_font = fitted_cell_font(
                &mut self.fonts.font_system,
                metrics,
                self.fonts.family,
                self.fonts.configured,
            );
            self.metrics = metrics;
        }
        if let Some(target) = &self.windowed {
            target.surface.configure(&self.device, &self.config);
        }
        self.content_key = None;
        self.row_text.rows.clear();
        self.pane_titles = None;
        self.reset_cursor_animation();
    }

    pub fn reset_cursor_animation(&mut self) {
        self.terminal.cursor_animation.reset();
        self.terminal.cursor_handoff = false;
        self.terminal.last_cursor_frame = None;
        self.dynamic_vertex_count = 0;
    }

    pub fn suspend_cursor_animation(&mut self) {
        // Keep cosmetic position without drawing or ticking the old attachment.
        self.terminal.cursor_handoff = self.terminal.cursor_animation.target.is_some();
        self.terminal.last_cursor_frame = None;
        self.dynamic_vertex_count = 0;
    }

    pub fn attach_cursor_animation(&mut self) {
        if !self.terminal.cursor_handoff {
            self.reset_cursor_animation();
        }
    }

    #[must_use]
    pub fn cursor_animation_active(&self) -> bool {
        !self.terminal.cursor_handoff && self.terminal.cursor_animation.is_active()
    }

    /// Uploads a completely received image resource, or refuses it so its entries show the
    /// marker; either way the renderer keeps working (VEN-C31). `referenced` names every resource
    /// that a current frame of an attached terminal references, which the budget never frees.
    pub fn upload_image(
        &mut self,
        resource: ImageResource,
        referenced: &HashSet<ImageKey>,
    ) -> Result<(), RenderError> {
        self.images
            .upload(&self.device, &self.queue, resource, referenced)
    }

    /// Orbit retired a resource; its texture goes once no current frame references it.
    pub fn retire_image(&mut self, key: ImageKey) {
        self.images.store.retire(key);
    }

    /// Frees the resources of every attachment outside `live`, the attachments of attached
    /// terminals, and retired textures outside `referenced`. Frames of other attachments show
    /// the marker over every entry, and pending entries of `at_once` attachments show it without
    /// waiting (VEN-C31).
    pub fn sweep_images(
        &mut self,
        live: HashSet<u64>,
        at_once: HashSet<u64>,
        referenced: &HashSet<ImageKey>,
    ) {
        self.images.store.sweep(live, at_once, referenced);
    }

    /// Whether an attached terminal's current frame wants a reattach to recover its images.
    #[must_use]
    pub fn image_resync(
        &self,
        attachment: u64,
        placements: &[ScenePlacement],
        referenced: &HashSet<ImageKey>,
    ) -> Option<ImageResync> {
        self.images.store.resync(attachment, placements, referenced)
    }

    /// Bytes of image textures held, counted at 4 bytes per pixel.
    #[must_use]
    pub fn image_resident(&self) -> u64 {
        self.images.store.resident()
    }

    /// When a pending image entry of the last drawn frame shows its marker, which needs a redraw.
    #[must_use]
    pub fn next_marker_deadline(&self) -> Option<Instant> {
        self.images.store.next_marker()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: Option<&Scene>,
        scroll_preview: Option<&ScenePreview>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        dialog: Option<&DialogScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> Result<PresentOutcome, RenderError> {
        ensure_device_available(&self.device_lost)?;
        let started = crate::transport::frame_time();
        let paint = workspace
            .filter(|workspace| self.pane_frames || workspace.popup_label().is_some())
            .map(|workspace| workspace_paint(workspace, scene, columns, self.metrics));
        let workspace = paint.as_deref().or(workspace);
        let (content_changed, rebuild_ns, prepare_ns) = self.prepare_frame(
            scene,
            scroll_preview,
            scroll_offset,
            workspace,
            columns,
            dialog,
            workspace_focus,
            status,
            blink_visible,
            preedit,
            generation,
        )?;
        let target = self.windowed.as_mut().ok_or_else(|| {
            RenderError("a hosted Venus renderer has no presentation surface".into())
        })?;
        crate::transport::bench_trace("texture_begin", "");
        let acquire_started = crate::transport::frame_time();
        let current_texture = target.surface.get_current_texture();
        let acquire_ns = crate::transport::frame_elapsed(acquire_started);
        crate::transport::bench_trace("texture_end", "");
        ensure_device_available(&self.device_lost)?;
        let frame = match current_texture {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Timeout => {
                target.window.request_redraw();
                return Ok(PresentOutcome::Deferred);
            }
            CurrentSurfaceTexture::Occluded => {
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Occluded);
            }
            CurrentSurfaceTexture::Outdated => {
                target.surface.configure(&self.device, &self.config);
                ensure_device_available(&self.device_lost)?;
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Recovered);
            }
            CurrentSurfaceTexture::Suboptimal(frame) => {
                drop(frame);
                target.surface.configure(&self.device, &self.config);
                ensure_device_available(&self.device_lost)?;
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Recovered);
            }
            CurrentSurfaceTexture::Lost => {
                ensure_device_available(&self.device_lost)?;
                target.surface = target
                    .instance
                    .create_surface(Arc::clone(&target.window))
                    .map_err(display_error("cannot recover the Venus GPU surface"))?;
                target.surface.configure(&self.device, &self.config);
                ensure_device_available(&self.device_lost)?;
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Recovered);
            }
            CurrentSurfaceTexture::Validation => {
                return Err(RenderError("the GPU rejected the Venus surface".into()));
            }
        };
        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("venus frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("venus frame"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(self.clear),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_into(&mut pass)?;
        }
        crate::transport::bench_trace("submit", "");
        let render_ns = crate::transport::frame_elapsed(started).saturating_sub(acquire_ns);
        let submit_started = crate::transport::frame_time();
        self.queue.submit(Some(encoder.finish()));
        ensure_device_available(&self.device_lost)?;
        if let Some(target) = &self.windowed {
            target.window.pre_present_notify();
        }
        self.queue.present(frame);
        ensure_device_available(&self.device_lost)?;
        let present_ns = crate::transport::frame_time();
        let submit_ns = present_ns
            .zip(submit_started)
            .map_or(0, |(end, start)| end.saturating_sub(start));
        self.atlas.trim();
        self.terminal.presented_scrollback_rect = self.terminal.scrollback_rect;
        if crate::transport::bench_tracing() {
            let target = |name: &str, rect: SceneRect| {
                crate::transport::bench_trace(
                    "target",
                    format_args!(
                        "{name} {} {} {} {}",
                        rect.left, rect.top, rect.width, rect.height
                    ),
                );
            };
            if let Some(workspace) = workspace {
                target("terminal", workspace.terminal);
                crate::transport::bench_trace(
                    "grid",
                    format_args!(
                        "{} {} {}",
                        self.metrics.width, self.metrics.height, self.metrics.padding
                    ),
                );
                for tab in &workspace.tabs {
                    target(&format!("tab-{}", tab.id), tab.rect);
                }
                for pane in &workspace.panes {
                    target(&format!("pane-{}", pane.id), pane.rect);
                }
                for column in &workspace.other_columns {
                    target(&format!("column-{}", column.index), column.terminal);
                }
                for control in &workspace.controls {
                    target(&format!("control-{:?}", control.kind), control.rect);
                }
            }
            if let Some(dialog) = dialog {
                target("dialog", dialog.bounds);
                for chip in &dialog.filters {
                    target(&format!("filter-{:?}", chip.filter), chip.rect);
                }
            }
        }
        Ok(PresentOutcome::Presented(present_ns.map(|present_ns| {
            RenderTimings {
                present_ns,
                render_ns,
                rebuild_ns,
                prepare_ns,
                acquire_ns,
                submit_ns,
                full_rebuild: content_changed,
            }
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_if_needed(
        &mut self,
        scene: Option<&Scene>,
        scroll_preview: Option<&ScenePreview>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        dialog: Option<&DialogScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> bool {
        let preview = scene.and_then(|_| preview_rows(scroll_preview));
        let text_blink_visible = blink_visible
            || !std::iter::once(scene)
                .chain(columns.iter().copied())
                .flatten()
                .flat_map(|scene| &scene.content)
                .chain(preview.into_iter().flat_map(|(_, rows)| rows))
                .flat_map(|row| &row.cells)
                .any(|cell| cell.style.blink && cell.style.foreground_visible(true));
        let graphics = std::iter::once(scene)
            .chain(columns.iter().copied())
            .map(|scene| scene.map(|scene| &scene.graphics))
            .collect::<Vec<_>>();
        let key = ContentKey {
            images: self.images.store.derive(&graphics, Instant::now()),
            generation,
            column_revisions: columns
                .iter()
                .map(|scene| scene.map(|scene| scene.revision))
                .collect(),
            scroll_offset,
            workspace_focus,
            text_blink_visible,
            preedit: preedit.to_owned(),
            status: status.to_owned(),
            hyperlink: self.terminal.hyperlink,
            header: self.header.clone(),
            scrollback_label: self.terminal.scrollback_label.clone(),
            dialog: dialog.cloned(),
            cpu: workspace
                .and_then(|workspace| workspace.cpu.as_ref())
                .map(|cpu| cpu.description.clone()),
            ram: workspace
                .and_then(|workspace| workspace.ram.as_ref())
                .map(|ram| ram.description.clone()),
            disk: workspace
                .and_then(|workspace| workspace.disk.as_ref())
                .map(|disk| disk.description.clone()),
            gpu: workspace
                .and_then(|workspace| workspace.gpu.as_ref())
                .map(|gpu| gpu.description.clone()),
            clock: workspace
                .and_then(|workspace| workspace.clock.as_ref())
                .map(|clock| clock.label.clone()),
            quota_tooltips: workspace
                .map(|workspace| {
                    workspace
                        .quotas
                        .iter()
                        .map(|quota| quota.tooltip.clone())
                        .collect()
                })
                .unwrap_or_default(),
        };
        if self.content_key.as_ref() == Some(&key) {
            return false;
        }

        #[cfg(test)]
        {
            self.grid_rebuilds += 1;
        }
        self.text.clear();
        self.row_text.begin();
        self.text_overlay = None;
        self.hint_popup_hit = None;
        self.spinners.clear();
        self.terminal.scrollback_rect = None;
        let viewport = workspace.map_or(
            SceneRect {
                left: 0.0,
                top: 0.0,
                width: self.config.width as f32,
                height: self.config.height as f32,
            },
            |workspace| workspace.terminal,
        );
        let cursor = scene.and_then(|scene| {
            scene
                .cursor
                .filter(|cursor| cursor.visible)
                .and_then(|cursor| {
                    cursor_bounds(
                        scene,
                        cursor,
                        self.metrics,
                        shifted(viewport, scroll_offset),
                    )
                })
        });
        let mut rectangles = RectangleBatch::new(self.config.width, self.config.height);
        let mut cursor_vertex_boundary = 0;
        if let Some(workspace) = workspace {
            let title_tabs = self.workspace_title_tabs(workspace);
            let pane_tabs = self.workspace_pane_tabs(workspace);
            self.clear = wgpu::Color::TRANSPARENT;
            if self.pane_frames {
                // Below the tab bar, everything no card covers is opaque bar color, so the bar,
                // the gaps and the hints read as one frame; each card takes its own terminal's
                // background.
                let cards = card_backdrops(
                    workspace,
                    scene,
                    columns,
                    self.metrics,
                    self.card_stroke_width(),
                );
                let frame = SceneRect {
                    left: 0.0,
                    top: workspace.tab_viewport.bottom(),
                    width: self.config.width as f32,
                    height: self.config.height as f32 - workspace.tab_viewport.bottom(),
                };
                if title_tabs.iter().any(Option::is_some) {
                    push_title_tab_backdrop(
                        &mut rectangles,
                        frame,
                        &cards,
                        self.background_opacity,
                        &title_tabs,
                    );
                } else {
                    push_card_backdrop(&mut rectangles, frame, &cards, self.background_opacity);
                }
            } else {
                // Each column's strip takes its own terminal's background; strips meet in the
                // middle of the gaps between columns and reach the window's sides, so the first
                // also lies under a visible agent sidebar (VEN-C30).
                let mut strips = std::iter::once((workspace.terminal, scene))
                    .chain(
                        workspace
                            .other_columns
                            .iter()
                            .zip(columns)
                            .map(|(column, scene)| (column.terminal, *scene)),
                    )
                    .collect::<Vec<_>>();
                strips.sort_by(|left, right| left.0.left.total_cmp(&right.0.left));
                let spans = background_spans(
                    &strips
                        .iter()
                        .map(|(terminal, _)| *terminal)
                        .collect::<Vec<_>>(),
                    self.config.width as f32,
                );
                for ((left, right), (_, scene)) in spans.into_iter().zip(&strips) {
                    rectangles.push(
                        left,
                        workspace.tab_viewport.bottom(),
                        right - left,
                        self.config.height as f32 - workspace.tab_viewport.bottom(),
                        scene.map_or(palette::TERMINAL_FALLBACK, |scene| scene.background),
                        self.background_opacity,
                    );
                }
            }
            build_workspace_padding(
                &mut rectangles,
                workspace,
                scene,
                columns,
                self.metrics,
                self.padding_color,
                self.pane_frames,
                self.card_stroke_width(),
                (scroll_preview, scroll_offset),
            );
            let backgrounds = card_backdrops(
                workspace,
                scene,
                columns,
                self.metrics,
                self.card_stroke_width(),
            )
            .into_iter()
            .map(|(_, color)| color)
            .collect::<Vec<_>>();
            self.build_workspace(
                workspace,
                workspace_focus,
                &mut rectangles,
                &backgrounds,
                &title_tabs,
                &pane_tabs,
            );
            for (text_slot, ((column, scene), band)) in workspace
                .other_columns
                .iter()
                .zip(columns)
                .zip(&key.images[1..])
                .enumerate()
            {
                let Some(scene) = scene else {
                    continue;
                };
                let Some((terminal, card_clip)) = terminal_paint_clip(
                    workspace,
                    column.chrome,
                    column.terminal,
                    self.metrics,
                    self.pane_frames,
                    self.card_stroke_width(),
                ) else {
                    continue;
                };
                let clip = scene_grid(scene, column.terminal, self.metrics)
                    .intersection(terminal)
                    .unwrap_or(terminal);
                rectangles.clip = Some(clip);
                rectangles.card_clip = card_clip;
                let text_clip =
                    terminal_text_clip(column.terminal, clip, self.metrics, self.pane_frames);
                self.build_scene_text(
                    scene,
                    blink_visible,
                    column.terminal,
                    text_clip,
                    text_slot + 1,
                );
                build_scene_rectangles(
                    &mut rectangles,
                    scene,
                    band,
                    None,
                    blink_visible,
                    self.metrics,
                    column.terminal,
                );
                rectangles.clip = None;
                rectangles.card_clip = None;
            }
            // Other columns still have cursors while the selected attachment is absent.
            cursor_vertex_boundary = vertex_count(&rectangles.bytes);
            if let Some((terminal, card_clip)) = terminal_paint_clip(
                workspace,
                workspace.chrome,
                workspace.terminal,
                self.metrics,
                self.pane_frames,
                self.card_stroke_width(),
            ) && let Some(scene) = scene
            {
                let clip = scene_grid(scene, workspace.terminal, self.metrics)
                    .intersection(terminal)
                    .unwrap_or(terminal);
                rectangles.clip = Some(clip);
                rectangles.card_clip = card_clip;
                let origin = shifted(workspace.terminal, scroll_offset);
                let text_clip =
                    terminal_text_clip(workspace.terminal, clip, self.metrics, self.pane_frames);
                self.build_scene_text(scene, blink_visible, origin, text_clip, 0);
                self.build_preview_text(scene, scroll_preview, blink_visible, origin, text_clip);
                cursor_vertex_boundary = build_scene_rectangles(
                    &mut rectangles,
                    scene,
                    &key.images[0],
                    scroll_preview,
                    blink_visible,
                    self.metrics,
                    origin,
                );
                self.build_preedit(scene, preedit, &mut rectangles, origin, text_clip);
                self.build_hyperlink(scene, &mut rectangles, origin);
                rectangles.clip = None;
                rectangles.card_clip = None;
            }
            if !status.is_empty() {
                self.build_notice(status, &mut rectangles, Some(workspace));
            }
            self.build_header_tooltip(workspace, &mut rectangles);
            if workspace.popup_label().is_some()
                && status.is_empty()
                && self.text_overlay.is_none()
                && let Some((terminal, card_clip)) = terminal_paint_clip(
                    workspace,
                    workspace.chrome,
                    workspace.terminal,
                    self.metrics,
                    self.pane_frames,
                    self.card_stroke_width(),
                )
            {
                rectangles.clip = Some(terminal);
                rectangles.card_clip = card_clip;
                self.build_scrollback(terminal, terminal, false, cursor, &mut rectangles);
                rectangles.clip = None;
                rectangles.card_clip = None;
            }
        } else if let Some(scene) = scene {
            self.clear = clear_color(
                scene.background,
                self.background_opacity,
                self.config.format.is_srgb(),
            );
            build_padding_rectangles(
                &mut rectangles,
                scene,
                self.metrics,
                viewport,
                viewport,
                self.padding_color,
                (scroll_preview, scroll_offset),
            );
            let clip = scene_grid(scene, viewport, self.metrics);
            rectangles.clip = Some(clip);
            let origin = shifted(viewport, scroll_offset);
            self.build_scene_text(scene, blink_visible, origin, clip, 0);
            self.build_preview_text(scene, scroll_preview, blink_visible, origin, clip);
            cursor_vertex_boundary = build_scene_rectangles(
                &mut rectangles,
                scene,
                &key.images[0],
                scroll_preview,
                blink_visible,
                self.metrics,
                origin,
            );
            self.build_preedit(scene, preedit, &mut rectangles, origin, clip);
            self.build_hyperlink(scene, &mut rectangles, origin);
            rectangles.clip = None;
            if !status.is_empty() {
                self.build_notice(status, &mut rectangles, None);
            } else {
                self.build_scrollback(viewport, viewport, false, cursor, &mut rectangles);
            }
        } else {
            self.clear = clear_color(
                palette::TERMINAL_FALLBACK,
                self.background_opacity,
                self.config.format.is_srgb(),
            );
            self.push_text(
                "VENUS",
                self.metrics.padding * 2.0,
                self.config.height as f32 * 0.42,
                self.config.width as f32 - self.metrics.padding * 4.0,
                self.metrics.height * 1.5,
                palette::SPLASH_TITLE,
                DrawStyleKind::Heading,
            );
            self.push_text(
                status,
                self.metrics.padding * 2.0,
                self.config.height as f32 * 0.50,
                self.config.width as f32 - self.metrics.padding * 4.0,
                self.metrics.height * 3.0,
                palette::SPLASH_TEXT,
                DrawStyleKind::Status(Wrap::WordOrGlyph),
            );
        }
        if let Some(dialog) = dialog {
            self.build_dialog(dialog, &mut rectangles);
        }
        self.row_text.finish();
        self.cursor_vertex_boundary = cursor_vertex_boundary;
        self.upload_vertices(&rectangles.bytes);
        self.images.prepare(
            &self.device,
            &self.queue,
            &rectangles.image_bytes,
            rectangles.images,
        );
        self.content_key = Some(key);
        true
    }

    fn build_dynamic_cursors(
        &mut self,
        scene: Option<&Scene>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        dialog: Option<&DialogScene>,
        blink_visible: bool,
    ) -> RectangleBatch {
        let mut rectangles = RectangleBatch::new(self.config.width, self.config.height);
        if let Some(workspace) = workspace {
            for (column, scene) in workspace.other_columns.iter().zip(columns) {
                let Some(scene) = scene else {
                    continue;
                };
                let Some((terminal, card_clip)) = terminal_paint_clip(
                    workspace,
                    column.chrome,
                    column.terminal,
                    self.metrics,
                    self.pane_frames,
                    self.card_stroke_width(),
                ) else {
                    continue;
                };
                rectangles.clip = Some(
                    scene_grid(scene, column.terminal, self.metrics)
                        .intersection(terminal)
                        .unwrap_or(terminal),
                );
                rectangles.card_clip = card_clip;
                build_scene_cursor(
                    &mut rectangles,
                    scene,
                    blink_visible,
                    self.metrics,
                    column.terminal,
                );
            }
        }
        let target = if let Some(workspace) = workspace {
            terminal_paint_clip(
                workspace,
                workspace.chrome,
                workspace.terminal,
                self.metrics,
                self.pane_frames,
                self.card_stroke_width(),
            )
            .map(|(clip, card_clip)| (workspace.terminal, Some(clip), card_clip))
        } else {
            Some((
                SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: self.config.width as f32,
                    height: self.config.height as f32,
                },
                None,
                None,
            ))
        };
        if let Some((viewport, clip, card_clip)) = target {
            rectangles.card_clip = card_clip;
            if scroll_offset == 0.0
                && dialog.is_none()
                && let Some((trail_color, outline_color, duration_scale)) = self.cursor_tail
            {
                rectangles.clip = clip;
                if !(scene.is_none() && self.terminal.cursor_handoff) {
                    self.terminal.cursor_handoff = false;
                    let now = Instant::now();
                    let delta = self
                        .terminal
                        .last_cursor_frame
                        .replace(now)
                        .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
                    if let Some(scene) = scene {
                        build_tail_cursor(
                            &mut rectangles,
                            &mut self.terminal.cursor_animation,
                            scene,
                            blink_visible,
                            self.metrics,
                            viewport,
                            trail_color,
                            outline_color,
                            duration_scale,
                            delta,
                        );
                    } else {
                        self.terminal.cursor_animation.reset();
                    }
                }
            } else {
                self.reset_cursor_animation();
                if self.cursor_tail.is_none()
                    && let Some(scene) = scene
                {
                    let grid = scene_grid(scene, viewport, self.metrics);
                    rectangles.clip =
                        Some(clip.map_or(grid, |clip| grid.intersection(clip).unwrap_or(clip)));
                    build_scene_cursor(
                        &mut rectangles,
                        scene,
                        blink_visible,
                        self.metrics,
                        shifted(viewport, scroll_offset),
                    );
                }
            }
        } else {
            self.reset_cursor_animation();
        }
        rectangles
    }

    fn scrollback_geometry(
        &mut self,
        area: SceneRect,
        clip: SceneRect,
        in_header: bool,
        cursor: Option<SceneRect>,
    ) -> Option<(SceneRect, f32)> {
        let label = self.terminal.scrollback_label.as_ref()?;
        let padding = self.metrics.padding;
        let width = fit_header_text(
            &mut self.fonts.font_system,
            self.metrics,
            label,
            f32::INFINITY,
        )
        .1
        .ceil()
            + padding * 2.0;
        let inset = if in_header {
            0.0
        } else {
            tokens::inner_inset(self.metrics)
        };
        let height = self.metrics.height + inset * 2.0;
        if width > area.width - padding || height > area.height {
            return None;
        }
        let rect = SceneRect {
            left: area.right() - width - padding,
            top: if in_header {
                area.top + (area.height - height) / 2.0
            } else {
                area.top + padding
            },
            width,
            height,
        };
        if rect.intersection(clip) != Some(rect)
            || cursor.is_some_and(|cursor| cursor.intersection(rect).is_some())
        {
            return None;
        }
        Some((rect, inset))
    }

    fn build_scrollback(
        &mut self,
        area: SceneRect,
        clip: SceneRect,
        in_header: bool,
        cursor: Option<SceneRect>,
        rectangles: &mut RectangleBatch,
    ) -> f32 {
        let Some((rect, inset)) = self.scrollback_geometry(area, clip, in_header, cursor) else {
            return 0.0;
        };
        let label = self.terminal.scrollback_label.clone().unwrap();
        let padding = self.metrics.padding;
        let width = rect.width;
        let height = rect.height;
        self.terminal.scrollback_rect = Some(rect);
        let first = self.text.len();
        self.push_text_clipped(
            &label,
            rect.left + padding,
            rect.top + inset,
            width - padding * 2.0,
            width - padding * 2.0,
            self.metrics.height,
            palette::SCROLLBACK_TEXT,
            DrawStyleKind::Status(Wrap::None),
            clip,
        );
        rectangles.push_rounded(
            rect,
            tokens::capsule_radius(height),
            if in_header {
                palette::SCROLLBACK_PILL_HEADER
            } else {
                palette::SCROLLBACK_PILL_OVERLAY
            },
        );
        if !in_header {
            self.text_overlay = Some((
                first,
                vec![TextBounds {
                    left: rect.left.floor() as i32,
                    top: rect.top.floor() as i32,
                    right: rect.right().ceil() as i32,
                    bottom: rect.bottom().ceil() as i32,
                }],
            ));
        }
        width
    }

    fn build_hyperlink(&self, scene: &Scene, rectangles: &mut RectangleBatch, origin: SceneRect) {
        if let Some((row, column)) = self.terminal.hyperlink
            && let Some(link) = scene.hyperlink_at(row, column, self.detect_local_hosts)
        {
            let rect = link.rect(origin, self.metrics);
            rectangles.push_hollow(
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                1.0,
                palette::HYPERLINK_OUTLINE,
            );
        }
    }

    fn build_notice(
        &mut self,
        status: &str,
        rectangles: &mut RectangleBatch,
        workspace: Option<&WorkspaceScene>,
    ) {
        const MAX_NOTICE_LINES: usize = 3;
        let hyperlink = self.terminal.hyperlink.is_some();
        // Lay the text out first: its wrapped line count sizes the notice.
        let notice = self.text.len();
        self.push_text(
            status,
            self.metrics.padding * 2.0,
            0.0,
            self.config.width as f32 - self.metrics.padding * 4.0,
            self.metrics.height
                * if hyperlink {
                    2.7
                } else {
                    tokens::WRAPPED_LINE_ROWS * MAX_NOTICE_LINES as f32
                },
            palette::NOTICE_TEXT,
            if hyperlink {
                DrawStyleKind::Monospace
            } else {
                DrawStyleKind::Status(Wrap::WordOrGlyph)
            },
        );
        self.notice_lines = if hyperlink {
            1
        } else {
            self.text[notice]
                .buffer
                .layout_runs()
                .count()
                .clamp(1, MAX_NOTICE_LINES)
        };
        let rect = self.notice_rect(workspace);
        let text = &mut self.text[notice];
        text.top += rect.top + self.metrics.height * if hyperlink { 0.3 } else { 0.125 };
        text.bound_top = rect.top.floor() as i32;
        text.bottom = rect.bottom().ceil() as i32;
        if self
            .terminal
            .scrollback_rect
            .is_some_and(|pill| pill.intersection(rect).is_some())
        {
            self.terminal.scrollback_rect = None;
        }
        // Text is drawn after rectangles; clip underlying rows out of this overlay.
        for text in &mut self.text[..notice] {
            if text.top < rect.top {
                text.bottom = text.bottom.min(rect.top as i32);
            } else {
                text.bound_top = text.bound_top.max(rect.bottom() as i32);
            }
        }
        rectangles.push(
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            palette::NOTICE_BACKGROUND,
            0.96,
        );
    }

    fn row_text_context(&self, scene: &Scene) -> RowTextContext {
        RowTextContext {
            metrics: self.metrics,
            cell_font: self.cell_font,
            family: self.fonts.family,
            columns: scene.columns,
            screen: scene.screen,
        }
    }

    fn build_scene_text(
        &mut self,
        scene: &Scene,
        blink_visible: bool,
        origin: SceneRect,
        clip: SceneRect,
        terminal: usize,
    ) {
        let context = self.row_text_context(scene);
        for (row_index, row) in scene.content.iter().enumerate() {
            let top = origin.top + self.metrics.padding + row_index as f32 * self.metrics.height;
            self.row_text.place(
                RowTextSlot {
                    terminal,
                    preview: None,
                    row: row_index,
                },
                row,
                context,
                &mut self.fonts.font_system,
                blink_visible,
                origin,
                top,
                clip,
                &mut self.text,
            );
        }
    }

    fn build_preview_text(
        &mut self,
        scene: &Scene,
        preview: Option<&ScenePreview>,
        blink_visible: bool,
        origin: SceneRect,
        clip: SceneRect,
    ) {
        let Some((direction, rows)) = preview_rows(preview) else {
            return;
        };
        let context = self.row_text_context(scene);
        for (row_index, row) in rows.iter().enumerate() {
            let top = preview_row_top(scene, direction, row_index, self.metrics, origin);
            self.row_text.place(
                RowTextSlot {
                    terminal: 0,
                    preview: Some(direction == VerticalDirection::Up),
                    row: row_index,
                },
                row,
                context,
                &mut self.fonts.font_system,
                blink_visible,
                origin,
                top,
                clip,
                &mut self.text,
            );
        }
    }

    fn build_preedit(
        &mut self,
        scene: &Scene,
        preedit: &str,
        rectangles: &mut RectangleBatch,
        origin: SceneRect,
        clip: SceneRect,
    ) {
        let Some(cursor) = scene.cursor.filter(|_| !preedit.is_empty()) else {
            return;
        };
        let left = origin.left
            + self.metrics.padding
            + f32::from(cursor.leading_column()) * self.metrics.width;
        let top = origin.top + self.metrics.padding + f32::from(cursor.row) * self.metrics.height;
        let width = (origin.right() - left - self.metrics.padding).max(1.0);
        let preedit_width = self.push_text_clipped(
            preedit,
            left,
            top,
            width,
            width,
            self.metrics.height,
            scene.foreground,
            DrawStyleKind::Preedit,
            clip,
        );
        rectangles.push(
            left,
            top + self.metrics.height - 2.0,
            preedit_width.max(self.metrics.width).min(width),
            2.0,
            scene.foreground,
            1.0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_text_clipped(
        &mut self,
        text: &str,
        left: f32,
        top: f32,
        layout_width: f32,
        ink_width: f32,
        layout_height: f32,
        foreground: SceneColor,
        kind: DrawStyleKind,
        clip: SceneRect,
    ) -> f32 {
        let Some(bounds) = (SceneRect {
            left,
            top,
            width: ink_width.max(1.0),
            height: layout_height.max(1.0),
        })
        .intersection(clip) else {
            return 0.0;
        };
        let previous_len = self.text.len();
        let width = self.push_text(
            text,
            left,
            top,
            layout_width,
            layout_height,
            foreground,
            kind,
        );
        if self.text.len() > previous_len {
            let bounds = text_clip_bounds(bounds);
            for text in &mut self.text[previous_len..] {
                text.bound_left = bounds.left;
                text.bound_top = bounds.top;
                text.right = bounds.right;
                text.bottom = bounds.bottom;
            }
        }
        width
    }

    #[allow(clippy::too_many_arguments)]
    fn push_text(
        &mut self,
        text: &str,
        left: f32,
        top: f32,
        layout_width: f32,
        layout_height: f32,
        foreground: SceneColor,
        kind: DrawStyleKind,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let layout_width = layout_width.max(1.0);
        let layout_height = layout_height.max(1.0);
        let buffer = shape_styled_text(
            &mut self.fonts.font_system,
            self.metrics,
            self.cell_font,
            text,
            layout_width,
            layout_height,
            kind,
        );
        let (left_offset, measured_width) = if matches!(kind, DrawStyleKind::Preedit) {
            shaped_preedit_placement(&buffer)
        } else {
            (0.0, 0.0)
        };
        self.text.push(PlacedText {
            buffer: buffer.into(),
            custom_glyph: None,
            left: left + left_offset,
            top: top
                - if matches!(kind, DrawStyleKind::Preedit | DrawStyleKind::Monospace) {
                    self.cell_font.top_offset
                } else {
                    0.0
                },
            right: (left + layout_width).ceil() as i32,
            bottom: (top + layout_height).ceil() as i32,
            bound_left: left.floor() as i32,
            bound_top: top.floor() as i32,
            color: Color::rgba(foreground.r, foreground.g, foreground.b, 255),
        });
        measured_width
    }

    fn upload_vertices(&mut self, bytes: &[u8]) {
        self.vertex_count = vertex_count(bytes);
        upload_vertices(
            &self.device,
            &self.queue,
            &mut self.vertices,
            &mut self.vertex_capacity,
            bytes,
            "venus rectangle vertices",
        );
    }

    fn upload_dynamic_vertices(&mut self, bytes: &[u8]) {
        self.dynamic_vertex_count = vertex_count(bytes);
        upload_vertices(
            &self.device,
            &self.queue,
            &mut self.dynamic_vertices,
            &mut self.dynamic_vertex_capacity,
            bytes,
            "venus dynamic cursor vertices",
        );
    }
}

fn vertex_count(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len() / VERTEX_SIZE as usize)
        .expect("bounded draw inputs fit the vertex count")
}

fn upload_vertices(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &mut wgpu::Buffer,
    capacity: &mut u64,
    bytes: &[u8],
    label: &'static str,
) {
    if bytes.is_empty() {
        return;
    }
    let needed = bytes.len() as u64;
    if needed > *capacity {
        *capacity = needed.next_power_of_two();
        *buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: *capacity,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
    queue.write_buffer(buffer, 0, bytes);
}

#[derive(Clone, Copy)]
enum DrawStyleKind {
    Heading,
    Preedit,
    Monospace,
    Status(Wrap),
    PopupLabel,
}

/// `text` shaped in the typography of `kind` for a box `layout_width` by `layout_height`, as
/// `push_text` draws it. Terminal cell layers use `shape_cell_layers` instead.
fn shape_styled_text(
    font_system: &mut FontSystem,
    metrics: CellMetrics,
    cell_font: CellFont,
    text: &str,
    layout_width: f32,
    layout_height: f32,
    kind: DrawStyleKind,
) -> Buffer {
    let (font_size, line_height, attrs, monospace_width, wrap) = match kind {
        DrawStyleKind::PopupLabel => (
            tokens::popup_label_font_size(metrics),
            12.0 * metrics.scale,
            Attrs::new().family(Family::SansSerif),
            None,
            Wrap::None,
        ),
        DrawStyleKind::Heading => (
            metrics.font_size * 1.15,
            metrics.height * 1.4,
            Attrs::new().family(Family::SansSerif).weight(Weight::BOLD),
            None,
            Wrap::None,
        ),
        DrawStyleKind::Preedit | DrawStyleKind::Monospace => (
            cell_font.size,
            metrics.height,
            cell_font.attrs(),
            Some(metrics.width),
            Wrap::None,
        ),
        DrawStyleKind::Status(wrap) => (
            chrome_title_metrics(metrics).font_size,
            if wrap == Wrap::None {
                chrome_title_metrics(metrics).line_height
            } else {
                metrics.height * tokens::WRAPPED_LINE_ROWS
            },
            Attrs::new().family(Family::SansSerif),
            None,
            wrap,
        ),
    };
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(Some(layout_width), Some(layout_height));
    buffer.set_wrap(wrap);
    buffer.set_monospace_width(monospace_width);
    if matches!(kind, DrawStyleKind::Monospace) {
        // Icons in monospace chrome, such as the Eon Bar readings', draw from the bundled face.
        buffer.set_rich_text(symbol_spans(text, &attrs), &attrs, shaping(text), None);
    } else {
        buffer.set_text(text, &attrs, shaping(text), None);
    }
    buffer.shape_until_scroll(font_system, false);
    buffer
}

fn shaped_preedit_placement(buffer: &Buffer) -> (f32, f32) {
    let mut glyphs = buffer.layout_runs().flat_map(|run| run.glyphs);
    let Some(first) = glyphs.next() else {
        return (0.0, 0.0);
    };
    let (left, right) = glyphs.fold((first.x, first.x + first.w), |(left, right), glyph| {
        (left.min(glyph.x), right.max(glyph.x + glyph.w))
    });
    (-left, right - left)
}

#[allow(clippy::too_many_arguments)]
fn shape_cell_layers(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    cell_font: CellFont,
    family: Option<&'static str>,
    text: &str,
    layout_width: f32,
    layout_height: f32,
    style: DrawStyle,
    wide: bool,
) -> [Option<Buffer>; 2] {
    let segments = cell_text_segments(text);
    let attrs = cell_text_attrs(cell_font, text, wide, style);
    let box_attrs = attrs
        .clone()
        .metrics(Metrics::new(cell_font.box_size, metrics.height))
        .letter_spacing(cell_font.box_letter_spacing);
    let braille_attrs = if family.is_some() {
        attrs.clone()
    } else {
        attrs.clone().family(Family::Name(BRAILLE_FAMILY))
    };
    let mut layers = [None, None];
    for (layer, box_drawing) in layers.iter_mut().zip([false, true]) {
        if !segments
            .iter()
            .any(|(_, kind)| (*kind == CellTextKind::Box) == box_drawing)
        {
            continue;
        }
        // A cell needs one line, not Vec's four-line initial allocation.
        let mut buffer = Buffer::new_empty(Metrics::new(cell_font.size, metrics.height));
        buffer.lines.reserve_exact(1);
        buffer.set_size(Some(layout_width.max(1.0)), Some(layout_height.max(1.0)));
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        set_cell_layer(
            &mut buffer,
            text,
            &segments,
            &attrs,
            &braille_attrs,
            &box_attrs,
            box_drawing,
        );
        if box_drawing {
            buffer.shape_until_scroll(fonts, false);
        } else {
            shape_cell_text(&mut buffer, fonts, cell_font, text, style, &attrs);
        }
        *layer = Some(buffer);
    }
    layers
}

fn shaping(text: &str) -> Shaping {
    if text.is_ascii() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

fn text_areas<'a>(
    text: &'a [PlacedText],
    overlay: Option<&(usize, Vec<TextBounds>)>,
) -> impl Iterator<Item = TextArea<'a>> {
    let rect = |bounds: &TextBounds| SceneRect {
        left: bounds.left as f32,
        top: bounds.top as f32,
        width: bounds.right as f32 - bounds.left as f32,
        height: bounds.bottom as f32 - bounds.top as f32,
    };
    let overlay =
        overlay.map(|(first, masks)| (*first, masks.iter().map(rect).collect::<Vec<_>>()));
    text.iter().enumerate().flat_map(move |(index, text)| {
        let bounds = TextBounds {
            left: text.bound_left,
            top: text.bound_top,
            right: text.right,
            bottom: text.bottom,
        };
        let occlusion = overlay.as_ref().filter(|(first, _)| index < *first);
        // Rectangles precede glyphs; use the painted contour without another rendering pass.
        occlusion
            .map(|(_, masks)| uncovered(rect(&bounds), masks))
            .into_iter()
            .flatten()
            .map(text_clip_bounds)
            .chain(occlusion.is_none().then_some(bounds))
            .filter(|clip| clip.left < clip.right && clip.top < clip.bottom)
            .map(move |bounds| TextArea {
                buffer: &text.buffer,
                left: text.left,
                top: text.top,
                scale: 1.0,
                bounds,
                default_color: text.color,
                custom_glyphs: text.custom_glyph.as_slice(),
            })
    })
}

pub(crate) fn physical_outline_width(logical_width: u8, scale: f32) -> f32 {
    (f32::from(logical_width) * scale).round().max(1.0)
}

/// Each pixel row of a rounded rectangle's top and bottom corner bands, with how far the curve
/// sits in from the sides at that row.
fn corner_rows(rect: SceneRect, radius: f32) -> impl Iterator<Item = (f32, f32)> {
    (0..radius as u32).flat_map(move |row| {
        let dy = radius - row as f32 - 0.5;
        let inset = radius - (radius * radius - dy * dy).sqrt();
        [
            (rect.top + row as f32, inset),
            (rect.bottom() - row as f32 - 1.0, inset),
        ]
    })
}

impl CardFill {
    fn middle(self) -> SceneRect {
        let radius = rounded_radius(self.bounds, self.radius);
        SceneRect {
            top: self.bounds.top + radius,
            height: self.bounds.height - radius * 2.0,
            ..self.bounds
        }
    }

    fn bands(self) -> impl Iterator<Item = SceneRect> {
        let rect = self.bounds;
        std::iter::once(self.middle()).chain(
            corner_rows(rect, rounded_radius(rect, self.radius)).map(move |(top, inset)| {
                SceneRect {
                    left: rect.left + inset,
                    top,
                    width: rect.width - inset * 2.0,
                    height: 1.0,
                }
            }),
        )
    }
}

fn span_rects(left: f32, right: f32, top: f32) -> impl Iterator<Item = (SceneRect, f32)> {
    let parts = if left.floor() == right.floor() {
        [
            (left.floor(), 1.0, right - left),
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 0.0),
        ]
    } else {
        [
            (left.floor(), 1.0, left.ceil() - left),
            (left.ceil(), right.floor() - left.ceil(), 1.0),
            (right.floor(), 1.0, right - right.floor()),
        ]
    };
    parts
        .into_iter()
        .filter(|(_, width, alpha)| *width > 0.0 && *alpha > 0.0)
        .map(move |(left, width, alpha)| {
            (
                SceneRect {
                    left,
                    top,
                    width,
                    height: 1.0,
                },
                alpha,
            )
        })
}

fn rounded_outline_rects(rect: SceneRect, radius: f32, width: f32) -> Vec<(SceneRect, f32)> {
    let radius = rounded_radius(rect, radius);
    if rect.width <= width * 2.0 || rect.height <= width * 2.0 {
        let fill = CardFill {
            bounds: rect,
            radius,
        };
        let mut rects = vec![(fill.middle(), 1.0)];
        for (top, inset) in corner_rows(rect, radius) {
            rects.extend(span_rects(rect.left + inset, rect.right() - inset, top));
        }
        return rects;
    }
    let mut rects = Vec::new();
    for left in [rect.left, rect.right() - width] {
        rects.push((
            SceneRect {
                left,
                top: rect.top + radius,
                width,
                height: rect.height - radius * 2.0,
            },
            1.0,
        ));
    }
    if width > radius {
        for top in [rect.top + radius, rect.bottom() - width] {
            rects.push((
                SceneRect {
                    left: rect.left + width,
                    top,
                    width: rect.width - width * 2.0,
                    height: width - radius,
                },
                1.0,
            ));
        }
    }
    for row in 0..radius as u32 {
        let dy = radius - row as f32 - 0.5;
        let outer = radius - (radius * radius - dy * dy).sqrt();
        for top in [rect.top + row as f32, rect.bottom() - row as f32 - 1.0] {
            if (row as f32) < width {
                rects.extend(span_rects(rect.left + outer, rect.right() - outer, top));
            } else {
                let inner = radius - ((radius - width).powi(2) - dy * dy).sqrt();
                rects.extend(span_rects(rect.left + outer, rect.left + inner, top));
                rects.extend(span_rects(rect.right() - inner, rect.right() - outer, top));
            }
        }
    }
    rects
}

/// Clip triangles to a fill band without moving vertices that are already inside it.
fn clip_polygon(points: &[CursorPoint], clip: SceneRect) -> Vec<CursorPoint> {
    let mut polygon = points.to_vec();
    for (axis, edge, minimum) in [
        (0, clip.left, true),
        (0, clip.right(), false),
        (1, clip.top, true),
        (1, clip.bottom(), false),
    ] {
        let coordinate = |p: CursorPoint| if axis == 0 { p.x } else { p.y };
        let inside = |p: CursorPoint| {
            if minimum {
                coordinate(p) >= edge
            } else {
                coordinate(p) <= edge
            }
        };
        let Some(mut previous) = polygon.last().copied() else {
            break;
        };
        let mut output = Vec::new();
        for current in polygon {
            if inside(previous) != inside(current) {
                let t =
                    (edge - coordinate(previous)) / (coordinate(current) - coordinate(previous));
                output.push(CursorPoint {
                    x: previous.x + t * (current.x - previous.x),
                    y: previous.y + t * (current.y - previous.y),
                });
            }
            if inside(current) {
                output.push(current);
            }
            previous = current;
        }
        polygon = output;
    }
    polygon
}

struct RectangleBatch {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    clip: Option<SceneRect>,
    card_clip: Option<CardFill>,
    /// Image draws in stream order and their own vertices.
    images: Vec<ImageDraw>,
    image_bytes: Vec<u8>,
}

impl RectangleBatch {
    fn push_title_tab_outline(
        &mut self,
        card: CardFill,
        tab: &FrameTitleTab,
        width: f32,
        color: SceneColor,
    ) {
        for (rect, alpha) in title_tab_outline_rects(card, tab, width) {
            self.push(rect.left, rect.top, rect.width, rect.height, color, alpha);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_pane_outlines(
        &mut self,
        fill: CardFill,
        viewport: SceneRect,
        frames: &[(PaneOutline, [SceneRect; 2])],
        tabs: &[Option<PaneTitleTab>],
        focused_title: Option<usize>,
        focus_color: Option<SceneColor>,
        border_width: f32,
        focus_width: f32,
    ) {
        let Some(focus_color) = focus_color else {
            return;
        };
        if !valid_rect(fill.bounds) {
            return;
        }
        let mut focus = Vec::new();
        let mut neutral = Vec::new();
        let mut accent = Vec::new();
        // Header scrolling keeps its viewport; only the fixed bottom curve bounds joins.
        let radius = rounded_radius(fill.bounds, fill.radius);
        let bottom = fill.bounds.bottom() - radius;
        let outline_clip = std::iter::once(SceneRect {
            top: viewport.top,
            height: (bottom - viewport.top).max(0.0),
            ..fill.bounds
        })
        .chain(fill.bands().filter(|band| band.top >= bottom))
        .collect::<Vec<_>>();
        for (index, (outline, gaps)) in frames.iter().enumerate() {
            let Some(clip) = outline
                .clip
                .intersection(viewport)
                .and_then(|clip| self.clip.map_or(Some(clip), |old| clip.intersection(old)))
            else {
                continue;
            };
            let width = if outline.expanded {
                focus_width
            } else {
                border_width
            };
            let gaps = gaps.map(|gap| {
                let left = gap.left.max(outline.bounds.left + outline.radius);
                SceneRect {
                    left,
                    top: outline.title_y.unwrap_or(0.0),
                    width: (gap.right() - left).max(0.0),
                    height: if outline.title_y.is_some() {
                        width
                    } else {
                        0.0
                    },
                }
            });
            let target = if outline.expanded {
                &mut focus
            } else {
                &mut neutral
            };
            let tab = tabs.get(index).and_then(Option::as_ref);
            let replacement = tab.map(|tab| tab.replacement());
            for (rect, alpha) in rounded_outline_rects(outline.bounds, outline.radius, width) {
                let Some(rect) = rect.intersection(clip) else {
                    continue;
                };
                for band in &outline_clip {
                    if let Some(rect) = rect.intersection(*band) {
                        for piece in uncovered(rect, &gaps) {
                            target.extend(
                                uncovered(piece, replacement.as_slice())
                                    .into_iter()
                                    .map(|rect| (rect, alpha)),
                            );
                        }
                    }
                }
            }
            if let Some(tab) = tab {
                let title_span = (focused_title == Some(index)).then(|| SceneRect {
                    width: tab.tab.start + tab.tab.return_run - tab.card.bounds.left,
                    ..tab.replacement()
                });
                // Caps may borrow an adjacent header's margin, but never its input bounds.
                for (rect, alpha) in tab.outline_rects(width) {
                    if let Some(rect) = rect
                        .intersection(viewport)
                        .and_then(|rect| rect.intersection(fill.bounds))
                    {
                        for rect in uncovered(rect, &gaps) {
                            if let Some(title) = title_span.and_then(|span| rect.intersection(span))
                            {
                                accent.push((title, alpha));
                            }
                            target.push((rect, alpha));
                        }
                    }
                }
            }
        }
        let mut painted = Vec::new();
        neutral.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (pieces, color) in [
            (accent, palette::ACCENT),
            (focus, focus_color),
            (neutral, palette::CARD_OUTLINE),
        ] {
            for (rect, alpha) in pieces {
                let pieces = uncovered(rect, &painted);
                for piece in &pieces {
                    self.push(
                        piece.left,
                        piece.top,
                        piece.width,
                        piece.height,
                        color,
                        alpha,
                    );
                }
                painted.extend(pieces);
            }
        }
    }

    /// Paint one frame, interrupting only its top stroke around shaped title/scrollback gaps.
    fn push_card_outline(
        &mut self,
        card: SceneRect,
        radius: f32,
        gaps: &[SceneRect],
        width: f32,
        color: SceneColor,
    ) {
        let clip = self.clip;
        let gaps = gaps
            .iter()
            .enumerate()
            .map(|(index, gap)| {
                let left = if index == 0 {
                    gap.left.max(card.left + rounded_radius(card, radius))
                } else {
                    gap.left
                };
                SceneRect {
                    left,
                    top: card.top,
                    width: (gap.right() - left).max(0.0),
                    height: width,
                }
            })
            .collect::<Vec<_>>();
        for segment in uncovered(card, &gaps) {
            self.clip = match clip {
                Some(clip) => segment.intersection(clip),
                None => Some(segment),
            };
            if self.clip.is_some() {
                self.push_rounded_outline_width(card, radius, width, color);
            }
        }
        self.clip = clip;
    }

    /// A one-physical-pixel stroke, leaving the translucent interior untouched.
    fn push_rounded_outline(&mut self, rect: SceneRect, radius: f32, color: SceneColor) {
        self.push_rounded_outline_width(rect, radius, 1.0, color);
    }

    fn push_rounded_outline_width(
        &mut self,
        rect: SceneRect,
        radius: f32,
        width: f32,
        color: SceneColor,
    ) {
        for (rect, alpha) in rounded_outline_rects(rect, radius, width) {
            self.push(rect.left, rect.top, rect.width, rect.height, color, alpha);
        }
    }

    fn push_span(&mut self, left: f32, right: f32, top: f32, color: SceneColor) {
        for (rect, alpha) in span_rects(left, right, top) {
            self.push(rect.left, rect.top, rect.width, rect.height, color, alpha);
        }
    }

    fn push_rounded(&mut self, rect: SceneRect, radius: f32, color: SceneColor) {
        let radius = rounded_radius(rect, radius);
        self.push(
            rect.left,
            rect.top + radius,
            rect.width,
            rect.height - radius * 2.0,
            color,
            1.0,
        );
        // Pixel-height bands reuse the existing clipped pipeline; fractional edge
        // coverage softens the curve without widening every terminal vertex.
        for (top, inset) in corner_rows(rect, radius) {
            self.push_span(rect.left + inset, rect.right() - inset, top, color);
        }
    }

    fn new(width: u32, height: u32) -> Self {
        Self {
            bytes: Vec::new(),
            width,
            height,
            clip: None,
            card_clip: None,
            images: Vec::new(),
            image_bytes: Vec::new(),
        }
    }

    fn push(&mut self, x: f32, y: f32, width: f32, height: f32, color: SceneColor, alpha: f32) {
        let mut rect = SceneRect {
            left: x,
            top: y,
            width,
            height,
        };
        if let Some(clip) = self.clip {
            let Some(clipped) = rect.intersection(clip) else {
                return;
            };
            rect = clipped;
        }
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let [top_left, top_right, bottom_right, bottom_left] = rect_corners(rect);
        self.push_points(
            [
                top_left,
                bottom_left,
                bottom_right,
                top_left,
                bottom_right,
                top_right,
            ],
            color,
            alpha,
        );
    }

    /// A straight stroke `thickness` wide from `start` to `end`.
    fn push_stroke(
        &mut self,
        start: CursorPoint,
        end: CursorPoint,
        thickness: f32,
        color: SceneColor,
        alpha: f32,
    ) {
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let length = dx.hypot(dy).max(f32::EPSILON);
        let (nx, ny) = (
            -dy / length * thickness / 2.0,
            dx / length * thickness / 2.0,
        );
        let offset = |point: CursorPoint, sign: f32| CursorPoint {
            x: point.x + nx * sign,
            y: point.y + ny * sign,
        };
        self.push_quad(
            [
                offset(start, 1.0),
                offset(end, 1.0),
                offset(end, -1.0),
                offset(start, -1.0),
            ],
            color,
            alpha,
        );
    }

    fn push_quad(&mut self, points: [CursorPoint; 4], color: SceneColor, alpha: f32) {
        if !alpha.is_finite()
            || !points
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
        {
            return;
        }
        let points = points.map(|mut point| {
            if let Some(clip) = self.clip {
                point.x = point.x.clamp(clip.left, clip.right());
                point.y = point.y.clamp(clip.top, clip.bottom());
            }
            point
        });
        self.push_points(
            [
                points[0], points[1], points[2], points[0], points[2], points[3],
            ],
            color,
            alpha,
        );
    }

    fn push_points(&mut self, points: [CursorPoint; 6], color: SceneColor, alpha: f32) {
        if let Some(card) = self.card_clip {
            let left = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let top = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let bounds = SceneRect {
                left,
                top,
                width: points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max) - left,
                height: points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max) - top,
            };
            if bounds.intersection(card.middle()) != Some(bounds) {
                for band in card.bands().filter_map(|band| band.intersection(bounds)) {
                    for triangle in points.chunks_exact(3) {
                        let polygon = clip_polygon(triangle, band);
                        for index in 1..polygon.len().saturating_sub(1) {
                            self.write_points(
                                &[polygon[0], polygon[index], polygon[index + 1]],
                                color,
                                alpha,
                            );
                        }
                    }
                }
                return;
            }
        }
        self.write_points(&points, color, alpha);
    }

    fn write_points(&mut self, points: &[CursorPoint], color: SceneColor, alpha: f32) {
        let to_x = |value: f32| value / self.width as f32 * 2.0 - 1.0;
        let to_y = |value: f32| 1.0 - value / self.height as f32 * 2.0;
        let rgba = [
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
            alpha,
        ];
        for point in points {
            let x = to_x(point.x);
            let y = to_y(point.y);
            for value in [x, y, rgba[0], rgba[1], rgba[2], rgba[3]] {
                self.bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
    }

    fn push_hollow(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        thickness: f32,
        color: SceneColor,
    ) {
        for (x, y, width, height) in [
            (x, y, width, thickness),
            (x, y + height - thickness, width, thickness),
            (x, y, thickness, height),
            (x + width - thickness, y, thickness, height),
        ] {
            self.push(x, y, width, height, color, 1.0);
        }
    }
}

fn nonzero(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

fn device_memory_hints(device_type: wgpu::DeviceType) -> wgpu::MemoryHints {
    // Intel saved 180 MiB; lavapipe input timing did not pass the comparison gate.
    if device_type == wgpu::DeviceType::Cpu {
        wgpu::MemoryHints::Performance
    } else {
        wgpu::MemoryHints::MemoryUsage
    }
}

fn surface_alpha_mode(
    background_opacity: f32,
    backend: wgpu::Backend,
    supported: &[CompositeAlphaMode],
) -> Result<CompositeAlphaMode, RenderError> {
    if background_opacity == 1.0 {
        Ok(CompositeAlphaMode::Auto)
    } else if supported.contains(&CompositeAlphaMode::PreMultiplied) {
        Ok(CompositeAlphaMode::PreMultiplied)
    } else if backend == wgpu::Backend::Metal
        && supported.contains(&CompositeAlphaMode::PostMultiplied)
    {
        // Core Animation composites non-opaque CAMetalLayer contents as premultiplied, as Venus renders.
        Ok(CompositeAlphaMode::PostMultiplied)
    } else {
        Err(RenderError(
            "the GPU surface does not support premultiplied transparency".into(),
        ))
    }
}

fn clear_color(color: SceneColor, background_opacity: f32, srgb_target: bool) -> wgpu::Color {
    let channel = |value| {
        let value = f64::from(value) / 255.0;
        let linear = if !srgb_target {
            value
        } else if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        };
        linear * f64::from(background_opacity)
    };
    wgpu::Color {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: f64::from(background_opacity),
    }
}

fn display_error<E>(context: &'static str) -> impl FnOnce(E) -> RenderError
where
    E: fmt::Display,
{
    move |error| RenderError(format!("{context}: {error}"))
}

fn record_device_loss(device_lost: &AtomicBool, reason: DeviceLostReason) -> bool {
    if reason == DeviceLostReason::Destroyed {
        return false;
    }
    device_lost.store(true, Ordering::Release);
    true
}

fn ensure_device_available(device_lost: &AtomicBool) -> Result<(), RenderError> {
    if device_lost.load(Ordering::Acquire) {
        Err(RenderError(DEVICE_LOST.into()))
    } else {
        Ok(())
    }
}

fn handle_uncaptured_gpu_error(device_lost: &AtomicBool, error: WgpuError) {
    assert!(device_lost.load(Ordering::Acquire), "wgpu error: {error}\n");
}

#[cfg(test)]
mod tests;
