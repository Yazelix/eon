use crate::{
    Color as SceneColor, DrawRow, DrawStyle, Scene, ScenePreview, SceneRect, ShortcutViewerScene,
    WorkspaceFocus, WorkspaceScene,
    scene::{inset_sides, pane_chrome_rect},
};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, CustomGlyph, Family, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use orbit_protocol::session::VerticalDirection;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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

use cells::{
    BRAILLE_FAMILY, CellFont, CellTextKind, block_span, build_preview_rectangles,
    build_scene_rectangles, cell_ink_right, cell_text_attrs, cell_text_segments, cursor_bounds,
    cursor_outline, fitted_cell_font, set_cell_layer,
};
use chrome::{HeaderInteraction, fit_header_text, rasterize_quota_icon};
use cursor_trail::{CursorAnimation, build_tail_cursor};

pub use fonts::{FontSettings, FontSetup};

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
const DEFAULT_BACKGROUND: SceneColor = SceneColor {
    r: 10,
    g: 13,
    b: 20,
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
    Presented,
    Deferred,
    Occluded,
    Recovered,
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

struct PlacedText {
    buffer: Buffer,
    custom_glyph: Option<CustomGlyph>,
    left: f32,
    top: f32,
    right: i32,
    bottom: i32,
    bound_left: i32,
    bound_top: i32,
    color: Color,
}

#[derive(Debug, PartialEq)]
struct ContentKey {
    generation: u64,
    /// Other columns' frame revisions, which repaint without touching the selected terminal.
    column_revisions: Vec<Option<u64>>,
    scroll_offset: f32,
    workspace_focus: WorkspaceFocus,
    blink_visible: bool,
    preedit: String,
    status: String,
    hyperlink: Option<(u16, u16)>,
    header: HeaderInteraction,
    scrollback_label: Option<String>,
    shortcut_viewer_scroll: Option<f32>,
    /// CPU and RAM use and the clock change without a new presentation generation. RAM keys on
    /// its description, whose total the hover shows and the label does not.
    cpu: Option<String>,
    ram: Option<String>,
    clock: Option<String>,
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

fn scene_grid(scene: &Scene, viewport: SceneRect, metrics: CellMetrics) -> SceneRect {
    SceneRect {
        left: viewport.left + metrics.padding,
        top: viewport.top + metrics.padding,
        width: f32::from(scene.columns) * metrics.width,
        height: f32::from(scene.rows) * metrics.height,
    }
}

/// The parts of `area` that no card covers, as rectangles in horizontal bands. Cards never
/// overlap; their rounded corners are the caller's to fill.
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

/// One wgpu surface and one glyphon text owner for the native window.
pub struct Renderer {
    instance: Instance,
    device: wgpu::Device,
    device_lost: Arc<AtomicBool>,
    queue: wgpu::Queue,
    surface: Surface<'static>,
    config: SurfaceConfiguration,
    rectangle_pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    vertex_count: u32,
    cursor_vertex_boundary: u32,
    dynamic_vertices: wgpu::Buffer,
    dynamic_vertex_capacity: u64,
    dynamic_vertex_count: u32,
    fonts: FontSetup,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_overlay: Option<(usize, TextBounds)>,
    text: Vec<PlacedText>,
    content_key: Option<ContentKey>,
    notice_lines: usize,
    terminal: TerminalLayer,
    header: HeaderInteraction,
    cursor_tail: Option<(SceneColor, SceneColor, f32)>,
    clear: wgpu::Color,
    background_opacity: f32,
    pane_frames: bool,
    metrics: CellMetrics,
    cell_font: CellFont,
    window: Arc<Window>,
}

impl Renderer {
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
                self.notice_lines as f32 * 1.25 + 0.25
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

    pub async fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        background_opacity: f32,
        cursor_tail: Option<(SceneColor, f32)>,
        pane_frames: bool,
        mut fonts: FontSetup,
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
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
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
        let srgb_target = config.format.is_srgb();
        config.present_mode = PresentMode::Fifo;
        config.alpha_mode = surface_alpha_mode(background_opacity, &capabilities.alpha_modes)?;
        ensure_device_available(&device_lost)?;
        surface.configure(&device, &config);
        ensure_device_available(&device_lost)?;

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

        let metrics = fonts.metrics(window.scale_factor());
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
            instance,
            device,
            device_lost,
            queue,
            surface,
            config,
            rectangle_pipeline,
            vertices,
            vertex_capacity,
            vertex_count: 0,
            cursor_vertex_boundary: 0,
            dynamic_vertices,
            dynamic_vertex_capacity: vertex_capacity,
            dynamic_vertex_count: 0,
            fonts,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_overlay: None,
            text: Vec::new(),
            content_key: None,
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
            cursor_tail: cursor_tail
                .map(|(color, duration)| (color, cursor_outline(color), duration)),
            clear: clear_color(DEFAULT_BACKGROUND, background_opacity, srgb_target),
            background_opacity,
            pane_frames,
            metrics,
            cell_font,
            window,
        })
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
            self.cell_font = fitted_cell_font(
                &mut self.fonts.font_system,
                metrics,
                self.fonts.family,
                self.fonts.configured,
            );
            self.metrics = metrics;
        }
        self.surface.configure(&self.device, &self.config);
        self.content_key = None;
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

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: Option<&Scene>,
        scroll_preview: Option<&ScenePreview>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        shortcut_viewer: Option<&ShortcutViewerScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> Result<PresentOutcome, RenderError> {
        ensure_device_available(&self.device_lost)?;
        let content_changed = self.rebuild_if_needed(
            scene,
            scroll_preview,
            scroll_offset,
            workspace,
            columns,
            shortcut_viewer,
            workspace_focus,
            status,
            blink_visible,
            preedit,
            generation,
        );
        if scroll_offset == 0.0 && shortcut_viewer.is_none() {
            self.rebuild_dynamic_cursor(scene, workspace, blink_visible);
        } else {
            self.reset_cursor_animation();
        }
        if content_changed {
            self.viewport.update(
                &self.queue,
                Resolution {
                    width: self.config.width,
                    height: self.config.height,
                },
            );
            if self
                .text_renderer
                .prepare_with_custom(
                    &self.device,
                    &self.queue,
                    &mut self.fonts.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    text_areas(&self.text, self.text_overlay),
                    &mut self.swash_cache,
                    rasterize_quota_icon,
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
                        text_areas(&self.text, self.text_overlay),
                        &mut self.swash_cache,
                        rasterize_quota_icon,
                    )
                    .map_err(display_error("the Venus glyph atlas is full"))?;
            }
        }

        let current_texture = self.surface.get_current_texture();
        ensure_device_available(&self.device_lost)?;
        let frame = match current_texture {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return Ok(PresentOutcome::Deferred);
            }
            CurrentSurfaceTexture::Occluded => {
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Occluded);
            }
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                ensure_device_available(&self.device_lost)?;
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Recovered);
            }
            CurrentSurfaceTexture::Suboptimal(frame) => {
                drop(frame);
                self.surface.configure(&self.device, &self.config);
                ensure_device_available(&self.device_lost)?;
                self.reset_cursor_animation();
                return Ok(PresentOutcome::Recovered);
            }
            CurrentSurfaceTexture::Lost => {
                ensure_device_available(&self.device_lost)?;
                self.surface = self
                    .instance
                    .create_surface(Arc::clone(&self.window))
                    .map_err(display_error("cannot recover the Venus GPU surface"))?;
                self.surface.configure(&self.device, &self.config);
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
            let cursor_boundary = cursor_vertex_boundary(
                self.vertex_count,
                self.cursor_vertex_boundary,
                self.dynamic_vertex_count > 0,
            );
            if self.vertex_count > 0 || self.dynamic_vertex_count > 0 {
                pass.set_pipeline(&self.rectangle_pipeline);
            }
            if cursor_boundary > 0 {
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(0..cursor_boundary, 0..1);
            }
            if self.dynamic_vertex_count > 0 {
                pass.set_vertex_buffer(0, self.dynamic_vertices.slice(..));
                pass.draw(0..self.dynamic_vertex_count, 0..1);
            }
            if cursor_boundary < self.vertex_count {
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(cursor_boundary..self.vertex_count, 0..1);
            }
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(display_error("cannot render the Venus glyph atlas"))?;
        }
        self.queue.submit(Some(encoder.finish()));
        ensure_device_available(&self.device_lost)?;
        self.window.pre_present_notify();
        self.queue.present(frame);
        ensure_device_available(&self.device_lost)?;
        self.atlas.trim();
        self.terminal.presented_scrollback_rect = self.terminal.scrollback_rect;
        Ok(PresentOutcome::Presented)
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_if_needed(
        &mut self,
        scene: Option<&Scene>,
        scroll_preview: Option<&ScenePreview>,
        scroll_offset: f32,
        workspace: Option<&WorkspaceScene>,
        columns: &[Option<&Scene>],
        shortcut_viewer: Option<&ShortcutViewerScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> bool {
        let key = ContentKey {
            generation,
            column_revisions: columns
                .iter()
                .map(|scene| scene.map(|scene| scene.revision))
                .collect(),
            scroll_offset,
            workspace_focus,
            blink_visible,
            preedit: preedit.to_owned(),
            status: status.to_owned(),
            hyperlink: self.terminal.hyperlink,
            header: self.header.clone(),
            scrollback_label: self.terminal.scrollback_label.clone(),
            shortcut_viewer_scroll: shortcut_viewer.map(|viewer| viewer.scroll),
            cpu: workspace
                .and_then(|workspace| workspace.cpu.as_ref())
                .map(|cpu| cpu.label.clone()),
            ram: workspace
                .and_then(|workspace| workspace.ram.as_ref())
                .map(|ram| ram.description.clone()),
            clock: workspace
                .and_then(|workspace| workspace.clock.as_ref())
                .map(|clock| clock.label.clone()),
        };
        if self.content_key.as_ref() == Some(&key) {
            return false;
        }

        self.text.clear();
        self.text_overlay = None;
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
            self.clear = wgpu::Color::TRANSPARENT;
            if self.pane_frames {
                // Below the tab bar, everything no card covers is opaque bar color, so the bar,
                // the gaps and the hints read as one frame; each card takes its own terminal's
                // background.
                let cards = std::iter::once((
                    workspace.chrome,
                    !workspace.panes.is_empty() || workspace.popup_label().is_some(),
                    scene,
                ))
                .chain(
                    workspace
                        .other_columns
                        .iter()
                        .zip(columns)
                        .map(|(column, scene)| (column.chrome, !column.panes.is_empty(), *scene)),
                )
                .filter_map(|(card, shown, scene)| shown.then_some((card, scene)))
                .collect::<Vec<_>>();
                let frame = SceneRect {
                    left: 0.0,
                    top: workspace.tab_viewport.bottom(),
                    width: self.config.width as f32,
                    height: self.config.height as f32 - workspace.tab_viewport.bottom(),
                };
                let rects = cards.iter().map(|(card, _)| *card).collect::<Vec<_>>();
                for rect in uncovered(frame, &rects) {
                    rectangles.push(
                        rect.left,
                        rect.top,
                        rect.width,
                        rect.height,
                        DEFAULT_BACKGROUND,
                        1.0,
                    );
                }
                let radius = workspace.card_radius();
                for (card, scene) in cards {
                    rectangles.push_framed_rounded(
                        card,
                        radius,
                        scene.map_or(DEFAULT_BACKGROUND, |scene| scene.background),
                        self.background_opacity,
                        DEFAULT_BACKGROUND,
                    );
                }
            } else {
                // Each column's strip takes its own terminal's background; strips meet in the
                // middle of the gaps between columns and reach the window's sides.
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
                        scene.map_or(DEFAULT_BACKGROUND, |scene| scene.background),
                        self.background_opacity,
                    );
                }
            }
            self.build_workspace(workspace, workspace_focus, &mut rectangles);
            for (column, scene) in workspace.other_columns.iter().zip(columns) {
                let Some(scene) = scene else {
                    continue;
                };
                let Some(terminal) = column.terminal.intersection(workspace.pane_viewport) else {
                    continue;
                };
                let clip = scene_grid(scene, column.terminal, self.metrics)
                    .intersection(terminal)
                    .unwrap_or(terminal);
                rectangles.clip = Some(clip);
                self.build_scene_text(scene, blink_visible, column.terminal, clip);
                // The cursor trail follows the selected column; other columns draw a static cursor.
                build_scene_rectangles(
                    &mut rectangles,
                    scene,
                    blink_visible,
                    self.metrics,
                    column.terminal,
                    true,
                );
                rectangles.clip = None;
            }
            if let Some(terminal) = workspace.visible_terminal()
                && let Some(scene) = scene
            {
                let clip = scene_grid(scene, workspace.terminal, self.metrics)
                    .intersection(terminal)
                    .unwrap_or(terminal);
                rectangles.clip = Some(clip);
                let origin = shifted(workspace.terminal, scroll_offset);
                self.build_scene_text(scene, blink_visible, origin, clip);
                self.build_preview_text(scene, scroll_preview, blink_visible, origin, clip);
                cursor_vertex_boundary = build_scene_rectangles(
                    &mut rectangles,
                    scene,
                    blink_visible,
                    self.metrics,
                    origin,
                    self.cursor_tail.is_none(),
                );
                build_preview_rectangles(
                    &mut rectangles,
                    scene,
                    scroll_preview,
                    blink_visible,
                    self.metrics,
                    origin,
                );
                self.build_preedit(scene, preedit, &mut rectangles, origin, clip);
                self.build_hyperlink(scene, &mut rectangles, origin);
                rectangles.clip = None;
            }
            // Each column is its own rounded card; separators divide its visible pane stack.
            let radius = workspace.card_radius();
            let columns = std::iter::once((workspace.chrome, workspace.panes.as_slice()))
                .chain(
                    workspace
                        .other_columns
                        .iter()
                        .map(|column| (column.chrome, column.panes.as_slice())),
                )
                .filter(|(_, panes)| self.pane_frames && !panes.is_empty());
            for (card, panes) in columns {
                for pane in panes.iter().skip(1) {
                    let Some(row) = (SceneRect {
                        top: pane.rect.top,
                        height: 1.0,
                        ..card
                    })
                    .intersection(workspace.pane_viewport) else {
                        continue;
                    };
                    rectangles.clip = Some(row);
                    rectangles.push_rounded(
                        card,
                        radius,
                        SceneColor {
                            r: 43,
                            g: 53,
                            b: 67,
                        },
                    );
                }
                rectangles.clip = None;
                rectangles.push_rounded_outline(
                    card,
                    radius,
                    SceneColor {
                        r: 58,
                        g: 75,
                        b: 91,
                    },
                );
            }
            if workspace_focus == WorkspaceFocus::Panes
                && let Some(pane) = workspace.panes.iter().find(|pane| pane.selected)
            {
                let accent = SceneColor {
                    r: 126,
                    g: 231,
                    b: 185,
                };
                if !self.pane_frames {
                    rectangles.clip = Some(workspace.pane_viewport);
                    rectangles.push_rounded_outline(
                        inset_sides(pane_chrome_rect(pane.rect, self.metrics), self.metrics),
                        self.metrics.padding,
                        accent,
                    );
                } else if let Some(clip) = pane.rect.intersection(workspace.pane_viewport) {
                    // The header's outline follows its card: the card's sides and corner curves
                    // within the header, plus the card's shape cut to the header's top and bottom
                    // rows where those lie inside the card.
                    let card = workspace.chrome;
                    rectangles.clip = Some(clip);
                    rectangles.push_rounded_outline(card, radius, accent);
                    for (needed, top) in [
                        (pane.rect.top > card.top, pane.rect.top),
                        (pane.rect.bottom() < card.bottom(), pane.rect.bottom() - 1.0),
                    ] {
                        if needed
                            && let Some(row) = (SceneRect {
                                top,
                                height: 1.0,
                                ..pane.rect
                            })
                            .intersection(clip)
                        {
                            rectangles.clip = Some(row);
                            rectangles.push_rounded(card, radius, accent);
                        }
                    }
                }
            }
            rectangles.clip = None;
            if let Some(label) = workspace.popup_label()
                && let Some(terminal) = workspace.visible_terminal()
            {
                rectangles.push_rounded_outline(
                    workspace.chrome,
                    radius,
                    SceneColor {
                        r: 82,
                        g: 104,
                        b: 124,
                    },
                );
                let label_height = 12.0 * self.metrics.scale;
                let label_top = workspace.chrome.top;
                let label_width = terminal.width - self.metrics.padding * 2.0;
                if terminal.top + self.metrics.padding
                    >= label_top + label_height + self.metrics.scale * 2.0
                    && terminal.height >= self.metrics.padding * 2.0 + self.metrics.height
                    && terminal.width >= self.metrics.padding * 2.0 + self.metrics.width * 4.0
                {
                    self.push_text_clipped(
                        label,
                        terminal.left + self.metrics.padding,
                        label_top,
                        label_width,
                        label_width,
                        label_height,
                        SceneColor {
                            r: 162,
                            g: 174,
                            b: 190,
                        },
                        DrawStyleKind::PopupLabel,
                        workspace.chrome,
                    );
                }
            }
            if !status.is_empty() {
                self.build_notice(status, &mut rectangles, Some(workspace));
            }
            self.build_header_tooltip(workspace, &mut rectangles);
            if workspace.popup_label().is_some()
                && status.is_empty()
                && self.text_overlay.is_none()
                && let Some(terminal) = workspace.visible_terminal()
            {
                self.build_scrollback(terminal, terminal, false, cursor, &mut rectangles);
            }
        } else if let Some(scene) = scene {
            self.clear = clear_color(
                scene.background,
                self.background_opacity,
                self.config.format.is_srgb(),
            );
            let clip = scene_grid(scene, viewport, self.metrics);
            rectangles.clip = Some(clip);
            let origin = shifted(viewport, scroll_offset);
            self.build_scene_text(scene, blink_visible, origin, clip);
            self.build_preview_text(scene, scroll_preview, blink_visible, origin, clip);
            cursor_vertex_boundary = build_scene_rectangles(
                &mut rectangles,
                scene,
                blink_visible,
                self.metrics,
                origin,
                self.cursor_tail.is_none(),
            );
            build_preview_rectangles(
                &mut rectangles,
                scene,
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
                DEFAULT_BACKGROUND,
                self.background_opacity,
                self.config.format.is_srgb(),
            );
            self.push_text(
                "VENUS",
                self.metrics.padding * 2.0,
                self.config.height as f32 * 0.42,
                self.config.width as f32 - self.metrics.padding * 4.0,
                self.metrics.height * 1.5,
                SceneColor {
                    r: 126,
                    g: 231,
                    b: 185,
                },
                DrawStyleKind::Heading,
            );
            self.push_text(
                status,
                self.metrics.padding * 2.0,
                self.config.height as f32 * 0.50,
                self.config.width as f32 - self.metrics.padding * 4.0,
                self.metrics.height * 3.0,
                SceneColor {
                    r: 196,
                    g: 202,
                    b: 216,
                },
                DrawStyleKind::Status(Wrap::WordOrGlyph),
            );
        }
        if let Some(shortcut_viewer) = shortcut_viewer {
            self.build_shortcut_viewer(shortcut_viewer, &mut rectangles);
        }
        self.cursor_vertex_boundary = cursor_vertex_boundary;
        self.upload_vertices(&rectangles.bytes);
        self.content_key = Some(key);
        true
    }

    fn rebuild_dynamic_cursor(
        &mut self,
        scene: Option<&Scene>,
        workspace: Option<&WorkspaceScene>,
        blink_visible: bool,
    ) {
        let Some((trail_color, outline_color, duration_scale)) = self.cursor_tail else {
            self.reset_cursor_animation();
            return;
        };
        let (viewport, clip) = if let Some(workspace) = workspace {
            let Some(clip) = workspace.visible_terminal() else {
                self.reset_cursor_animation();
                return;
            };
            (workspace.terminal, Some(clip))
        } else {
            (
                SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: self.config.width as f32,
                    height: self.config.height as f32,
                },
                None,
            )
        };
        if scene.is_none() && self.terminal.cursor_handoff {
            return;
        }
        self.terminal.cursor_handoff = false;
        let now = Instant::now();
        let delta = self
            .terminal
            .last_cursor_frame
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        let mut rectangles = RectangleBatch::new(self.config.width, self.config.height);
        rectangles.clip = clip;
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
        self.upload_dynamic_vertices(&rectangles.bytes);
    }

    fn build_scrollback(
        &mut self,
        area: SceneRect,
        clip: SceneRect,
        in_header: bool,
        cursor: Option<SceneRect>,
        rectangles: &mut RectangleBatch,
    ) -> f32 {
        let Some(label) = self.terminal.scrollback_label.clone() else {
            return 0.0;
        };
        let padding = self.metrics.padding;
        let width = fit_header_text(
            &mut self.fonts.font_system,
            self.metrics,
            &label,
            f32::INFINITY,
        )
        .1
        .ceil()
            + padding * 2.0;
        let inset = if in_header { 0.0 } else { padding / 2.0 };
        let height = self.metrics.height + inset * 2.0;
        if width > area.width - padding || height > area.height {
            return 0.0;
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
            return 0.0;
        }
        self.terminal.scrollback_rect = Some(rect);
        let first = self.text.len();
        self.push_text_clipped(
            &label,
            rect.left + padding,
            rect.top + inset,
            width - padding * 2.0,
            width - padding * 2.0,
            self.metrics.height,
            SceneColor {
                r: 162,
                g: 204,
                b: 230,
            },
            DrawStyleKind::Status(Wrap::None),
            clip,
        );
        rectangles.push_rounded(
            rect,
            height / 2.0,
            if in_header {
                SceneColor {
                    r: 37,
                    g: 58,
                    b: 74,
                }
            } else {
                SceneColor {
                    r: 28,
                    g: 43,
                    b: 58,
                }
            },
        );
        if !in_header {
            self.text_overlay = Some((
                first,
                TextBounds {
                    left: rect.left.floor() as i32,
                    top: rect.top.floor() as i32,
                    right: rect.right().ceil() as i32,
                    bottom: rect.bottom().ceil() as i32,
                },
            ));
        }
        width
    }

    fn build_hyperlink(&self, scene: &Scene, rectangles: &mut RectangleBatch, origin: SceneRect) {
        if let Some((row, column)) = self.terminal.hyperlink
            && let Some(link) = scene.hyperlink_at(row, column)
        {
            let rect = link.rect(origin, self.metrics);
            rectangles.push_hollow(
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                1.0,
                SceneColor {
                    r: 137,
                    g: 180,
                    b: 250,
                },
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
                    1.25 * MAX_NOTICE_LINES as f32
                },
            SceneColor {
                r: 245,
                g: 192,
                b: 94,
            },
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
            SceneColor {
                r: 35,
                g: 29,
                b: 18,
            },
            0.96,
        );
    }

    fn build_scene_text(
        &mut self,
        scene: &Scene,
        blink_visible: bool,
        origin: SceneRect,
        clip: SceneRect,
    ) {
        let mut runs = scene.glyph_runs();
        runs.retain(|run| run.style.foreground_visible(blink_visible));
        for (index, run) in runs.iter().enumerate() {
            if block_span(&run.text, run.columns == 1).is_some() {
                continue;
            }
            let left =
                origin.left + self.metrics.padding + f32::from(run.column) * self.metrics.width;
            let top = origin.top + self.metrics.padding + f32::from(run.row) * self.metrics.height;
            let width = f32::from(run.columns) * self.metrics.width;
            let next = runs.get(index + 1).filter(|next| next.row == run.row);
            let ink_width = f32::from(cell_ink_right(run, next, scene.columns) - run.column)
                * self.metrics.width;
            self.push_text_clipped(
                &run.text,
                left,
                top,
                width,
                ink_width,
                self.metrics.height,
                run.style.foreground,
                DrawStyleKind::Cell(run.style),
                clip,
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
        let mut runs = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            let top = preview_row_top(scene, direction, row_index, self.metrics, origin);
            if !row_intersects_clip(top, self.metrics.height, clip) {
                continue;
            }
            runs.clear();
            row.append_glyph_runs(0, &mut runs);
            runs.retain(|run| run.style.foreground_visible(blink_visible));
            for (index, run) in runs.iter().enumerate() {
                if block_span(&run.text, run.columns == 1).is_some() {
                    continue;
                }
                let left =
                    origin.left + self.metrics.padding + f32::from(run.column) * self.metrics.width;
                let width = f32::from(run.columns) * self.metrics.width;
                let next = runs.get(index + 1);
                let ink_width = f32::from(cell_ink_right(run, next, scene.columns) - run.column)
                    * self.metrics.width;
                self.push_text_clipped(
                    &run.text,
                    left,
                    top,
                    width,
                    ink_width,
                    self.metrics.height,
                    run.style.foreground,
                    DrawStyleKind::Cell(run.style),
                    clip,
                );
            }
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
            for text in &mut self.text[previous_len..] {
                text.bound_left = bounds.left.floor() as i32;
                text.bound_top = bounds.top.floor() as i32;
                text.right = bounds.right().ceil() as i32;
                text.bottom = bounds.bottom().ceil() as i32;
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
        let (font_size, line_height, attrs, monospace_width, wrap, alpha) = match kind {
            DrawStyleKind::Cell(style) => {
                self.push_cell_text(
                    text,
                    left,
                    top,
                    layout_width,
                    layout_height,
                    foreground,
                    style,
                );
                return 0.0;
            }
            DrawStyleKind::PopupLabel => (
                11.0 * self.metrics.scale,
                12.0 * self.metrics.scale,
                Attrs::new().family(Family::SansSerif),
                None,
                Wrap::None,
                255,
            ),
            DrawStyleKind::Heading => (
                self.metrics.font_size * 1.15,
                self.metrics.height * 1.4,
                Attrs::new().family(Family::SansSerif).weight(Weight::BOLD),
                None,
                Wrap::None,
                255,
            ),
            DrawStyleKind::Preedit | DrawStyleKind::Monospace => (
                self.cell_font.size,
                self.metrics.height,
                self.cell_font.attrs(),
                Some(self.metrics.width),
                Wrap::None,
                255,
            ),
            DrawStyleKind::Status(wrap) => (
                self.metrics.font_size,
                if wrap == Wrap::None {
                    self.metrics.height
                } else {
                    self.metrics.height * 1.25
                },
                Attrs::new().family(Family::SansSerif),
                None,
                wrap,
                255,
            ),
        };
        let layout_width = layout_width.max(1.0);
        let layout_height = layout_height.max(1.0);
        let mut buffer = Buffer::new(
            &mut self.fonts.font_system,
            Metrics::new(font_size, line_height),
        );
        buffer.set_size(Some(layout_width), Some(layout_height));
        buffer.set_wrap(wrap);
        buffer.set_monospace_width(monospace_width);
        buffer.set_text(text, &attrs, shaping(text), None);
        buffer.shape_until_scroll(&mut self.fonts.font_system, false);
        let (left_offset, measured_width) = if matches!(kind, DrawStyleKind::Preedit) {
            shaped_preedit_placement(&buffer)
        } else {
            (0.0, 0.0)
        };
        self.text.push(PlacedText {
            buffer,
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
            color: Color::rgba(foreground.r, foreground.g, foreground.b, alpha),
        });
        measured_width
    }

    #[allow(clippy::too_many_arguments)]
    fn push_cell_text(
        &mut self,
        text: &str,
        left: f32,
        top: f32,
        layout_width: f32,
        layout_height: f32,
        foreground: SceneColor,
        style: DrawStyle,
    ) {
        let segments = cell_text_segments(text);
        let attrs = cell_text_attrs(self.cell_font, text, style);
        let box_attrs = attrs
            .clone()
            .metrics(Metrics::new(self.cell_font.box_size, self.metrics.height))
            .letter_spacing(self.cell_font.box_letter_spacing);
        let braille_attrs = if self.fonts.family.is_some() {
            attrs.clone()
        } else {
            attrs.clone().family(Family::Name(BRAILLE_FAMILY))
        };
        for box_drawing in [false, true] {
            if !segments
                .iter()
                .any(|(_, kind)| (*kind == CellTextKind::Box) == box_drawing)
            {
                continue;
            }
            // A cell needs one line, not Vec's four-line initial allocation.
            let mut buffer =
                Buffer::new_empty(Metrics::new(self.cell_font.size, self.metrics.height));
            buffer.lines.reserve_exact(1);
            buffer.set_size(Some(layout_width.max(1.0)), Some(layout_height.max(1.0)));
            buffer.set_wrap(Wrap::None);
            buffer.set_monospace_width(Some(self.metrics.width));
            set_cell_layer(
                &mut buffer,
                text,
                &segments,
                &attrs,
                &braille_attrs,
                &box_attrs,
                box_drawing,
            );
            buffer.shape_until_scroll(&mut self.fonts.font_system, false);
            self.text.push(PlacedText {
                buffer,
                custom_glyph: None,
                left,
                top: top
                    - if box_drawing {
                        0.0
                    } else {
                        self.cell_font.top_offset
                    },
                right: (left + layout_width).ceil() as i32,
                bottom: (top + layout_height).ceil() as i32,
                bound_left: left.floor() as i32,
                bound_top: top.floor() as i32,
                color: Color::rgba(
                    foreground.r,
                    foreground.g,
                    foreground.b,
                    style.foreground_alpha(),
                ),
            });
        }
    }

    fn upload_vertices(&mut self, bytes: &[u8]) {
        self.vertex_count = upload_vertices(
            &self.device,
            &self.queue,
            &mut self.vertices,
            &mut self.vertex_capacity,
            bytes,
            "venus rectangle vertices",
        );
    }

    fn upload_dynamic_vertices(&mut self, bytes: &[u8]) {
        self.dynamic_vertex_count = upload_vertices(
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

fn cursor_vertex_boundary(total: u32, boundary: u32, dynamic: bool) -> u32 {
    (if dynamic { boundary } else { total }).min(total)
}

fn upload_vertices(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &mut wgpu::Buffer,
    capacity: &mut u64,
    bytes: &[u8],
    label: &'static str,
) -> u32 {
    let count = vertex_count(bytes);
    if bytes.is_empty() {
        return count;
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
    count
}

#[derive(Clone, Copy)]
enum DrawStyleKind {
    Cell(DrawStyle),
    Heading,
    Preedit,
    Monospace,
    Status(Wrap),
    PopupLabel,
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

fn shaping(text: &str) -> Shaping {
    if text.is_ascii() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

fn text_areas(
    text: &[PlacedText],
    overlay: Option<(usize, TextBounds)>,
) -> impl Iterator<Item = TextArea<'_>> {
    text.iter().enumerate().flat_map(move |(index, text)| {
        let bounds = TextBounds {
            left: text.bound_left,
            top: text.bound_top,
            right: text.right,
            bottom: text.bottom,
        };
        let mut clips = [bounds; 4];
        let count = if let Some((first_overlay_text, occlusion)) = overlay
            && index < first_overlay_text
            && bounds.left < occlusion.right
            && bounds.right > occlusion.left
            && bounds.top < occlusion.bottom
            && bounds.bottom > occlusion.top
        {
            // Rectangles precede glyphs. Keep the text around this floating
            // preview without cloning its buffers or adding a rendering pass.
            clips[0].bottom = bounds.bottom.min(occlusion.top);
            clips[1].top = bounds.top.max(occlusion.bottom);
            clips[2].top = bounds.top.max(occlusion.top);
            clips[2].bottom = bounds.bottom.min(occlusion.bottom);
            clips[2].right = bounds.right.min(occlusion.left);
            clips[3] = TextBounds {
                left: bounds.left.max(occlusion.right),
                right: bounds.right,
                ..clips[2]
            };
            4
        } else {
            1
        };
        clips
            .into_iter()
            .take(count)
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

/// A rounded rectangle's corner radius in whole pixels, at most half its shorter side.
fn rounded_radius(rect: SceneRect, radius: f32) -> f32 {
    radius
        .min(rect.width / 2.0)
        .min(rect.height / 2.0)
        .max(0.0)
        .floor()
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

struct RectangleBatch {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    clip: Option<SceneRect>,
}

impl RectangleBatch {
    /// A one-physical-pixel stroke, leaving the translucent interior untouched.
    fn push_rounded_outline(&mut self, rect: SceneRect, radius: f32, color: SceneColor) {
        if self
            .clip
            .is_some_and(|clip| rect.intersection(clip).is_none())
        {
            return;
        }
        if rect.width <= 2.0 || rect.height <= 2.0 {
            self.push_rounded(rect, radius, color);
            return;
        }
        let radius = radius
            .min(rect.width / 2.0)
            .min(rect.height / 2.0)
            .max(0.0)
            .floor();
        if radius < 1.0 {
            self.push_hollow(rect.left, rect.top, rect.width, rect.height, 1.0, color);
            return;
        }
        for left in [rect.left, rect.right() - 1.0] {
            self.push(
                left,
                rect.top + radius,
                1.0,
                rect.height - radius * 2.0,
                color,
                1.0,
            );
        }
        for row in 0..radius as u32 {
            let dy = radius - row as f32 - 0.5;
            let outer = radius - (radius * radius - dy * dy).sqrt();
            for top in [rect.top + row as f32, rect.bottom() - row as f32 - 1.0] {
                if row == 0 {
                    self.push_span(rect.left + outer, rect.right() - outer, top, color);
                } else {
                    let inner = radius - ((radius - 1.0).powi(2) - dy * dy).sqrt();
                    self.push_span(rect.left + outer, rect.left + inner, top, color);
                    self.push_span(rect.right() - inner, rect.right() - outer, top, color);
                }
            }
        }
    }

    fn push_span(&mut self, left: f32, right: f32, top: f32, color: SceneColor) {
        if left.floor() == right.floor() {
            self.push(left.floor(), top, 1.0, 1.0, color, right - left);
        } else {
            self.push(left.floor(), top, 1.0, 1.0, color, left.ceil() - left);
            self.push(
                left.ceil(),
                top,
                right.floor() - left.ceil(),
                1.0,
                color,
                1.0,
            );
            self.push(right.floor(), top, 1.0, 1.0, color, right - right.floor());
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

    /// Fills a pixel-aligned rounded `rect` with `fill` at `alpha` and its corners outside the
    /// curve with opaque `frame`. Each curve pixel is opaque frame under `fill` at its coverage, so
    /// the GPU mixes them in the target's color space and the edge never turns translucent.
    fn push_framed_rounded(
        &mut self,
        rect: SceneRect,
        radius: f32,
        fill: SceneColor,
        alpha: f32,
        frame: SceneColor,
    ) {
        let radius = rounded_radius(rect, radius);
        self.push(
            rect.left,
            rect.top + radius,
            rect.width,
            rect.height - radius * 2.0,
            fill,
            alpha,
        );
        for (top, inset) in corner_rows(rect, radius) {
            let left = rect.left + inset;
            let right = rect.right() - inset;
            let (first, last) = (left.floor(), right.floor());
            self.push(rect.left, top, first - rect.left, 1.0, frame, 1.0);
            self.push(first + 1.0, top, last - first - 1.0, 1.0, fill, alpha);
            self.push(last + 1.0, top, rect.right() - last - 1.0, 1.0, frame, 1.0);
            for (pixel, coverage) in [(first, first + 1.0 - left), (last, right - last)] {
                self.push(pixel, top, 1.0, 1.0, frame, 1.0);
                self.push(pixel, top, 1.0, 1.0, fill, coverage * alpha);
            }
        }
    }

    fn new(width: u32, height: u32) -> Self {
        Self {
            bytes: Vec::new(),
            width,
            height,
            clip: None,
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

fn surface_alpha_mode(
    background_opacity: f32,
    supported: &[CompositeAlphaMode],
) -> Result<CompositeAlphaMode, RenderError> {
    if background_opacity == 1.0 {
        return Ok(CompositeAlphaMode::Auto);
    }
    supported
        .contains(&CompositeAlphaMode::PreMultiplied)
        .then_some(CompositeAlphaMode::PreMultiplied)
        .ok_or_else(|| {
            RenderError("the GPU surface does not support premultiplied transparency".into())
        })
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
mod tests {
    use super::cells::tests::plain_style;
    use super::*;
    use crate::{DrawCell, DrawCursor, DrawRow};
    use orbit_protocol::{CellWidth, CursorShape, Screen};

    #[test]
    fn device_loss_blocks_surface_recovery_but_destroy_does_not() {
        let lost = AtomicBool::new(false);

        assert!(!record_device_loss(&lost, DeviceLostReason::Destroyed));
        assert_eq!(ensure_device_available(&lost), Ok(()));

        assert!(record_device_loss(&lost, DeviceLostReason::Unknown));
        assert_eq!(
            ensure_device_available(&lost),
            Err(RenderError(DEVICE_LOST.into()))
        );
    }

    #[test]
    fn uncaptured_errors_remain_fatal_until_device_loss() {
        let lost = AtomicBool::new(false);
        let out_of_memory = || wgpu::Error::OutOfMemory {
            source: Box::new(std::io::Error::other("test GPU allocation failure")),
        };

        assert!(
            std::panic::catch_unwind(|| handle_uncaptured_gpu_error(&lost, out_of_memory()))
                .is_err()
        );

        assert!(record_device_loss(&lost, DeviceLostReason::Unknown));
        handle_uncaptured_gpu_error(&lost, out_of_memory());
    }

    #[test]
    fn content_cache_separates_generations_and_viewer_state() {
        let key = |generation, shortcut_viewer_scroll| ContentKey {
            generation,
            column_revisions: vec![Some(4), None],
            scroll_offset: 0.0,
            workspace_focus: WorkspaceFocus::Terminal,
            blink_visible: true,
            preedit: String::new(),
            status: String::new(),
            hyperlink: None,
            header: HeaderInteraction::default(),
            scrollback_label: None,
            shortcut_viewer_scroll,
            cpu: None,
            ram: None,
            clock: None,
        };

        assert_ne!(key(1, None), key(2, None));
        assert_ne!(key(1, None), key(1, Some(0.0)));
        assert_ne!(key(1, Some(0.0)), key(1, Some(1.0)));
        // Another column's frame repaints without a new selected-terminal generation.
        assert_ne!(
            key(1, None),
            ContentKey {
                column_revisions: vec![Some(5), None],
                ..key(1, None)
            }
        );
    }

    #[test]
    fn bounded_preview_rows_cover_multi_row_fractional_offsets() {
        let metrics = CellMetrics {
            scale: 1.0,
            width: 10.0,
            height: 20.0,
            font_size: 16.0,
            padding: 5.0,
        };
        let scene = Scene {
            revision: 7,
            columns: 1,
            rows: 2,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground: SceneColor::default(),
            cursor: None,
            content: Vec::new(),
        };
        let row = DrawRow {
            wrapped: false,
            wrap_continuation: false,
            kitty_virtual_placeholder: false,
            cells: Vec::new(),
        };
        let viewport = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 20.0,
            height: 50.0,
        };
        let grid = scene_grid(&scene, viewport, metrics);
        assert!(row_intersects_clip(
            grid.top - metrics.height / 2.0,
            metrics.height,
            grid
        ));
        assert!(!row_intersects_clip(grid.bottom(), metrics.height, grid));

        for (direction, offset) in [
            (VerticalDirection::Up, 3.0 * metrics.height - 0.25),
            (VerticalDirection::Down, -3.0 * metrics.height + 0.25),
        ] {
            let preview = ScenePreview::Viewport {
                frame_revision: scene.revision,
                direction,
                edge_reached: false,
                rows: vec![row.clone(); 3],
            };
            let (actual_direction, rows) = preview_rows(Some(&preview)).unwrap();
            assert_eq!(rows.len(), 3);
            let origin = shifted(viewport, offset);
            let first = preview_row_top(&scene, actual_direction, 0, metrics, origin);
            let top = preview_row_top(&scene, actual_direction, 2, metrics, origin);
            assert_eq!((top - first).abs(), 2.0 * metrics.height);
            assert!(
                (SceneRect {
                    left: grid.left,
                    top,
                    width: grid.width,
                    height: metrics.height,
                })
                .intersection(grid)
                .is_some()
            );
        }
    }

    #[test]
    fn tail_cursor_outlines_one_color_and_resets_when_hidden_or_blinking_off() {
        let style = plain_style();
        let mut scene = Scene {
            revision: 1,
            columns: 4,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground: SceneColor::default(),
            cursor: Some(DrawCursor {
                visible: true,
                blinking: false,
                password_input: false,
                shape: CursorShape::Block,
                column: 0,
                row: 0,
                at_wide_tail: false,
                color: SceneColor { r: 1, g: 2, b: 3 },
                explicit_color: false,
            }),
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: [
                    CellWidth::Narrow,
                    CellWidth::Wide,
                    CellWidth::SpacerTail,
                    CellWidth::Narrow,
                ]
                .into_iter()
                .map(|width| DrawCell {
                    width,
                    text: String::new(),
                    hyperlink: String::new(),
                    style,
                })
                .collect(),
            }],
        };
        let viewport = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let metrics = CellMetrics::for_scale(1.0);
        let trail = SceneColor {
            r: 0x12,
            g: 0xab,
            b: 0xcf,
        };
        let outline = cursor_outline(trail);
        let mut animation = CursorAnimation::default();
        let mut first = RectangleBatch::new(100, 100);
        assert!(!build_tail_cursor(
            &mut first,
            &mut animation,
            &scene,
            true,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.0,
        ));
        assert_eq!(
            first.bytes.len(),
            VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize * 5
        );

        scene.cursor.as_mut().unwrap().column = 2;
        scene.cursor.as_mut().unwrap().at_wide_tail = true;
        let mut moving = RectangleBatch::new(100, 100);
        assert!(build_tail_cursor(
            &mut moving,
            &mut animation,
            &scene,
            true,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.0,
        ));
        assert_eq!(
            moving.bytes.len(),
            VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize * 7
        );
        let quad = VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize;
        let vertex_color = |vertex: &[u8]| {
            [8, 12, 16]
                .map(|offset| f32::from_ne_bytes(vertex[offset..offset + 4].try_into().unwrap()))
        };
        for (bytes, color) in moving
            .bytes
            .chunks_exact(quad)
            .zip([outline, trail, trail, outline, outline, outline, outline])
        {
            let expected = [color.r, color.g, color.b].map(|value| f32::from(value) / 255.0);
            assert!(
                bytes
                    .chunks_exact(VERTEX_SIZE as usize)
                    .all(|vertex| vertex_color(vertex) == expected)
            );
        }
        scene.cursor.as_mut().unwrap().explicit_color = true;
        let mut explicit = RectangleBatch::new(100, 100);
        assert!(build_tail_cursor(
            &mut explicit,
            &mut animation,
            &scene,
            true,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.0,
        ));
        let expected = [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0];
        assert_eq!(explicit.bytes.len(), 3 * quad);
        assert!(
            explicit.bytes[explicit.bytes.len() - quad..]
                .chunks_exact(VERTEX_SIZE as usize)
                .all(|vertex| vertex_color(vertex) == expected)
        );
        scene.cursor.as_mut().unwrap().explicit_color = false;
        for (shape, colors) in [
            (CursorShape::Bar, &[outline, trail][..]),
            (CursorShape::Underline, &[outline, trail][..]),
            (
                CursorShape::Block,
                &[trail, outline, outline, outline, outline][..],
            ),
            (
                CursorShape::BlockHollow,
                &[
                    outline, outline, outline, outline, trail, trail, trail, trail,
                ][..],
            ),
        ] {
            scene.cursor.as_mut().unwrap().shape = shape;
            let mut body = RectangleBatch::new(100, 100);
            assert!(!build_tail_cursor(
                &mut body,
                &mut CursorAnimation::default(),
                &scene,
                true,
                metrics,
                viewport,
                trail,
                outline,
                1.0,
                0.0,
            ));
            assert_eq!(body.bytes.len(), colors.len() * quad, "{shape:?}");
            for (bytes, color) in body.bytes.chunks_exact(quad).zip(colors) {
                let expected = [color.r, color.g, color.b].map(|value| f32::from(value) / 255.0);
                assert!(
                    bytes
                        .chunks_exact(VERTEX_SIZE as usize)
                        .all(|vertex| vertex_color(vertex) == expected)
                );
            }
        }
        scene.cursor.as_mut().unwrap().shape = CursorShape::Block;
        let mut tick = RectangleBatch::new(100, 100);
        assert!(build_tail_cursor(
            &mut tick,
            &mut animation,
            &scene,
            true,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.016,
        ));

        scene.cursor.as_mut().unwrap().blinking = true;
        let mut hidden = RectangleBatch::new(100, 100);
        assert!(!build_tail_cursor(
            &mut hidden,
            &mut animation,
            &scene,
            false,
            metrics,
            viewport,
            trail,
            outline,
            1.0,
            0.01,
        ));
        assert!(hidden.bytes.is_empty());
        assert!(!animation.is_active());
    }

    #[test]
    fn dynamic_cursor_keeps_later_static_overlays_on_top() {
        assert_eq!(cursor_vertex_boundary(18, 12, true), 12);
        assert_eq!(cursor_vertex_boundary(18, 12, false), 18);
        assert_eq!(cursor_vertex_boundary(18, 24, true), 18);
    }

    #[test]
    fn trail_quad_is_two_clipped_finite_triangles_in_stable_order() {
        let mut rectangles = RectangleBatch::new(100, 100);
        rectangles.clip = Some(SceneRect {
            left: 10.0,
            top: 20.0,
            width: 50.0,
            height: 40.0,
        });
        let points = [
            CursorPoint { x: -5.0, y: 10.0 },
            CursorPoint { x: 70.0, y: 15.0 },
            CursorPoint { x: 65.0, y: 70.0 },
            CursorPoint { x: 5.0, y: 65.0 },
        ];
        rectangles.push_quad(points, SceneColor { r: 1, g: 2, b: 3 }, 1.0);

        assert_eq!(
            rectangles.bytes.len(),
            VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
        );
        let positions = rectangles
            .bytes
            .chunks_exact(VERTEX_SIZE as usize)
            .map(|vertex| {
                (
                    f32::from_ne_bytes(vertex[..4].try_into().unwrap()),
                    f32::from_ne_bytes(vertex[4..8].try_into().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            [
                points[0], points[1], points[2], points[0], points[2], points[3]
            ]
            .map(|point| {
                let x = point.x.clamp(10.0, 60.0) / 100.0 * 2.0 - 1.0;
                let y = 1.0 - point.y.clamp(20.0, 60.0) / 100.0 * 2.0;
                (x, y)
            })
        );
        assert!(
            positions
                .iter()
                .all(|(x, y)| x.is_finite() && y.is_finite())
        );

        rectangles.push_quad(
            [CursorPoint {
                x: f32::NAN,
                y: 0.0,
            }; 4],
            SceneColor::default(),
            1.0,
        );
        assert_eq!(
            rectangles.bytes.len(),
            VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
        );
    }

    #[test]
    fn rounded_outline_keeps_corners_and_interior_clear_when_clipped() {
        let bounds = SceneRect {
            left: 10.0,
            top: 20.0,
            width: 80.0,
            height: 60.0,
        };
        for radius in [0.0, 12.0, 100.0] {
            for clip in [
                bounds,
                SceneRect {
                    top: 30.0,
                    height: 40.0,
                    ..bounds
                },
            ] {
                let mut batch = RectangleBatch::new(100, 100);
                batch.clip = Some(clip);
                batch.push_rounded_outline(bounds, radius, SceneColor::default());
                let quads: Vec<_> = batch
                    .bytes
                    .chunks_exact(VERTEX_SIZE as usize * 6)
                    .map(|quad| {
                        let value = |vertex: usize, component: usize| {
                            let offset = vertex * VERTEX_SIZE as usize + component * 4;
                            f32::from_ne_bytes(quad[offset..offset + 4].try_into().unwrap())
                        };
                        let rect = SceneRect {
                            left: (value(0, 0) + 1.0) * 50.0,
                            top: (1.0 - value(0, 1)) * 50.0,
                            width: (value(2, 0) - value(0, 0)) * 50.0,
                            height: (value(0, 1) - value(2, 1)) * 50.0,
                        };
                        let alpha = value(0, 5);
                        assert!((0.0..=1.0).contains(&alpha));
                        assert!(
                            rect.left >= clip.left - 0.001 && rect.right() <= clip.right() + 0.001
                        );
                        assert!(
                            rect.top >= clip.top - 0.001 && rect.bottom() <= clip.bottom() + 0.001
                        );
                        (rect, alpha)
                    })
                    .collect();
                let painted = |x, y| {
                    quads
                        .iter()
                        .any(|(rect, alpha)| *alpha > 0.0 && rect.contains(x, y))
                };
                assert!(!painted(50.0, 50.0), "outline must not fill the terminal");
                assert!(painted(10.5, 50.0), "side stroke is missing");
                if clip == bounds {
                    assert!(painted(50.0, 20.5), "top stroke is missing");
                    if radius > 1.0 {
                        assert!(!painted(10.5, 20.5), "corner is square");
                    }
                } else {
                    assert!(
                        !painted(50.0, 30.5),
                        "clipping must not create a new top border"
                    );
                }
            }
        }
    }

    #[test]
    fn rectangle_encoding_is_row_major_and_bounded() {
        let mut rectangles = RectangleBatch::new(100, 100);
        rectangles.push(0.0, 0.0, 10.0, 20.0, SceneColor { r: 1, g: 2, b: 3 }, 1.0);
        assert_eq!(
            rectangles.bytes.len(),
            VERTEX_SIZE as usize * VERTICES_PER_QUAD as usize
        );
        assert_eq!(&rectangles.bytes[..4], &(-1.0_f32).to_ne_bytes());
        assert_eq!(&rectangles.bytes[4..8], &1.0_f32.to_ne_bytes());
    }

    #[test]
    fn column_backgrounds_cover_the_gaps_between_cards() {
        let column = |left: f32| SceneRect {
            left,
            top: 40.0,
            width: 289.0,
            height: 500.0,
        };
        let terminals = [column(8.0), column(305.0), column(602.0)];
        let spans = background_spans(&terminals, 901.0);
        // The strips tile the window's width, so no gap stays unpainted.
        assert_eq!(spans.first().map(|span| span.0), Some(0.0));
        assert_eq!(spans.last().map(|span| span.1), Some(901.0));
        for (pair, terminal) in spans.windows(2).zip(&terminals) {
            assert_eq!(pair[0].1, pair[1].0);
            assert!(pair[0].0 <= terminal.left && terminal.right() <= pair[0].1);
        }
        assert_eq!(background_spans(&terminals[..1], 901.0), [(0.0, 901.0)]);
    }

    #[test]
    fn frame_fills_everything_outside_the_cards() {
        let frame = SceneRect {
            left: 0.0,
            top: 32.0,
            width: 901.0,
            height: 568.0,
        };
        let card = |left: f32, top: f32| SceneRect {
            left,
            top,
            width: 289.0,
            height: 500.0,
        };
        // A popup card can sit at another height than the columns beside it.
        let cards = [card(8.0, 40.0), card(305.0, 48.0), card(602.0, 40.0)];
        let free = uncovered(frame, &cards);
        assert!(
            free.iter()
                .all(|rect| cards.iter().all(|card| rect.intersection(*card).is_none()))
        );
        let area = |rect: &SceneRect| rect.width * rect.height;
        let covered = free.iter().chain(&cards).map(area).sum::<f32>();
        assert_eq!(covered, area(&frame));
        assert_eq!(uncovered(frame, &[]), [frame]);
    }

    #[test]
    fn framed_card_corners_stay_opaque() {
        let (width, height) = (40_u32, 40_u32);
        let mut rectangles = RectangleBatch::new(width, height);
        let card = SceneRect {
            left: 4.0,
            top: 4.0,
            width: 32.0,
            height: 32.0,
        };
        let color = SceneColor { r: 0, g: 0, b: 0 };
        rectangles.push_framed_rounded(card, 8.0, color, 1.0, DEFAULT_BACKGROUND);
        // Composite every quad's straight alpha over a transparent surface, pixel by pixel.
        let mut alpha = vec![0.0_f32; (width * height) as usize];
        let values = rectangles
            .bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        for quad in values.chunks_exact(36) {
            let xs = quad
                .chunks_exact(6)
                .map(|vertex| (vertex[0] + 1.0) / 2.0 * width as f32);
            let ys = quad
                .chunks_exact(6)
                .map(|vertex| (1.0 - vertex[1]) / 2.0 * height as f32);
            let (left, right) = xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            let (top, bottom) = ys.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
            let source = quad[5];
            for y in top.round() as u32..bottom.round() as u32 {
                for x in left.round() as u32..right.round() as u32 {
                    let pixel = &mut alpha[(y * width + x) as usize];
                    *pixel = source + *pixel * (1.0 - source);
                }
            }
        }
        for y in 4..36 {
            for x in 4..36 {
                let pixel = alpha[(y * width + x) as usize];
                assert!(
                    (pixel - 1.0).abs() < 1e-4,
                    "pixel {x},{y} has alpha {pixel}"
                );
            }
        }
    }

    #[test]
    fn background_opacity_selects_only_proved_surface_modes() {
        assert_eq!(
            surface_alpha_mode(1.0, &[CompositeAlphaMode::Inherit]).unwrap(),
            CompositeAlphaMode::Auto
        );
        assert_eq!(
            surface_alpha_mode(
                0.88,
                &[
                    CompositeAlphaMode::Opaque,
                    CompositeAlphaMode::PreMultiplied,
                ],
            )
            .unwrap(),
            CompositeAlphaMode::PreMultiplied
        );
        assert!(surface_alpha_mode(0.0, &[CompositeAlphaMode::Opaque]).is_err());
        assert!(surface_alpha_mode(0.5, &[CompositeAlphaMode::PostMultiplied]).is_err());
    }

    #[test]
    fn background_opacity_premultiplies_clear_colors() {
        let source = SceneColor {
            r: 128,
            g: 0,
            b: 255,
        };
        let opaque = clear_color(source, 1.0, true);
        let translucent = clear_color(source, 0.5, true);
        let encoded = clear_color(source, 1.0, false);

        assert!((opaque.r - 0.215_860_5).abs() < 0.000_001);
        assert_eq!(opaque.a, 1.0);
        assert!((translucent.r - 0.107_930_25).abs() < 0.000_001);
        assert_eq!(translucent.g, 0.0);
        assert_eq!(translucent.b, 0.5);
        assert_eq!(translucent.a, 0.5);
        assert_eq!(encoded.r, 128.0 / 255.0);
    }

    #[test]
    fn shaped_preedit_follows_emitted_glyphs_without_charging_combining_marks() {
        let mut fonts = FontSetup::new(&FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            size: 20.0,
            line_height: 1.5,
            ..Default::default()
        })
        .unwrap();
        let metrics = fonts.metrics(1.25);
        let cell_font = fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, true);
        let mut font_system = fonts.font_system;
        let mut text_width = |text: &str| {
            let mut buffer = Buffer::new(
                &mut font_system,
                Metrics::new(cell_font.size, metrics.height),
            );
            buffer.set_size(Some(100.0), Some(metrics.height));
            buffer.set_wrap(Wrap::None);
            buffer.set_monospace_width(Some(metrics.width));
            buffer.set_text(text, &cell_font.attrs(), shaping(text), None);
            buffer.shape_until_scroll(&mut font_system, false);
            let glyphs = buffer
                .layout_runs()
                .next()
                .expect("the preedit should shape")
                .glyphs;
            let (left, right) = glyphs.iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(left, right), glyph| (left.min(glyph.x), right.max(glyph.x + glyph.w)),
            );
            let (left_offset, width) = shaped_preedit_placement(&buffer);
            (width, right - left, left + left_offset)
        };

        let widths = ["e", "eee", "e\u{301}", "אבג"].map(|text| {
            let (width, emitted_width, left) = text_width(text);
            assert!(
                (width - emitted_width).abs() < 0.01,
                "{text:?} measured {width} but emitted {emitted_width}"
            );
            assert!(left.abs() < 0.01, "{text:?} starts at {left}");
            width
        });
        assert!((widths[0] - widths[2]).abs() < 0.01);
    }

    #[test]
    fn ascii_uses_basic_shaping_without_weakening_unicode() {
        assert_eq!(shaping("plain ASCII"), Shaping::Basic);
        assert_eq!(shaping("e\u{301}"), Shaping::Advanced);
    }
}
