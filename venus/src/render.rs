use crate::{
    Color as SceneColor, DrawCursor, DrawRow, DrawStyle, GlyphRun, Scene, ScenePreview, SceneRect,
    ShortcutViewerScene, WorkspaceFocus, WorkspaceHeaderControl, WorkspaceScene,
    scene::{QuotaProvider, pane_chrome_rect},
};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, Style, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use orbit_protocol::{CellWidth, CursorShape, Underline, session::VerticalDirection};
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

mod cursor_trail;
mod fonts;

use cursor_trail::{CursorAnimation, build_tail_cursor};

pub use fonts::{FontSettings, FontSetup};

const VERTEX_SIZE: u64 = 24;
#[cfg(test)]
const VERTICES_PER_QUAD: u32 = 6;
const BRAILLE_FAMILY: &str = "DejaVu Sans";
const NERD_FONT_FAMILY: &str = "Symbols Nerd Font Mono";
const ACCELERANDO_FAMILY: &str = "Accelerando Terminal";
const OPENAI_BLOSSOM_ID: u16 = 1;
const CLAUDE_SPLAT_ID: u16 = 2;
const QUOTA_ICON_SIZE: usize = 104;
const OPENAI_BLOSSOM_ALPHA: &[u8; QUOTA_ICON_SIZE * QUOTA_ICON_SIZE] =
    include_bytes!("../assets/openai-blossom-104.alpha");
const CLAUDE_SPLAT_ALPHA: &[u8; QUOTA_ICON_SIZE * QUOTA_ICON_SIZE] =
    include_bytes!("../assets/claude-splat-104.alpha");
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

#[derive(Clone, Copy)]
struct CellFont {
    family: Option<&'static str>,
    size: f32,
    top_offset: f32,
    letter_spacing: f32,
    box_size: f32,
    box_letter_spacing: f32,
}

impl CellFont {
    fn attrs(self) -> Attrs<'static> {
        Attrs::new()
            .family(self.family.map_or(Family::Monospace, Family::Name))
            .letter_spacing(self.letter_spacing)
    }
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

fn rasterize_quota_icon(request: RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph> {
    let source = match request.id {
        OPENAI_BLOSSOM_ID => OPENAI_BLOSSOM_ALPHA,
        CLAUDE_SPLAT_ID => CLAUDE_SPLAT_ALPHA,
        _ => return None,
    };
    let width = usize::from(request.width);
    let height = usize::from(request.height);
    if width == 0 || height == 0 {
        return None;
    }
    let mut data = vec![0; width * height];
    for y in 0..height {
        let top = y * QUOTA_ICON_SIZE / height;
        let bottom = ((y + 1) * QUOTA_ICON_SIZE)
            .div_ceil(height)
            .max(top + 1)
            .min(QUOTA_ICON_SIZE);
        for x in 0..width {
            let left = x * QUOTA_ICON_SIZE / width;
            let right = ((x + 1) * QUOTA_ICON_SIZE)
                .div_ceil(width)
                .max(left + 1)
                .min(QUOTA_ICON_SIZE);
            let mut alpha = 0_u64;
            let count = ((right - left) * (bottom - top)) as u64;
            for source_y in top..bottom {
                for source_x in left..right {
                    alpha += u64::from(source[source_y * QUOTA_ICON_SIZE + source_x]);
                }
            }
            data[y * width + x] = (alpha / count) as u8;
        }
    }
    Some(RasterizedCustomGlyph {
        data,
        content_type: ContentType::Mask,
    })
}

#[derive(Debug, PartialEq)]
struct ContentKey {
    generation: u64,
    scroll_offset: f32,
    workspace_focus: WorkspaceFocus,
    blink_visible: bool,
    preedit: String,
    status: String,
    hyperlink: Option<(u16, u16)>,
    hovered_header: Option<(WorkspaceFocus, String)>,
    quota_hovered: Option<usize>,
    version_hovered: bool,
    pressed_header: Option<WorkspaceHeaderControl>,
    scrollback_label: Option<String>,
    shortcut_viewer_scroll: Option<f32>,
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

fn header_item_visual(rect: SceneRect, padding: f32) -> SceneRect {
    let inset = (padding / 3.0).min(rect.width / 4.0).min(rect.height / 4.0);
    SceneRect {
        left: rect.left + inset,
        top: rect.top + inset,
        width: (rect.width - inset * 2.0).max(0.0),
        height: (rect.height - inset * 2.0).max(0.0),
    }
}

fn scene_grid(scene: &Scene, viewport: SceneRect, metrics: CellMetrics) -> SceneRect {
    SceneRect {
        left: viewport.left + metrics.padding,
        top: viewport.top + metrics.padding,
        width: f32::from(scene.columns) * metrics.width,
        height: f32::from(scene.rows) * metrics.height,
    }
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
    terminal: TerminalLayer,
    hovered_header: Option<(WorkspaceFocus, String)>,
    quota_hovered: Option<usize>,
    version_hovered: bool,
    pressed_header: Option<WorkspaceHeaderControl>,
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

    pub fn set_hovered_header(&mut self, header: Option<(WorkspaceFocus, String)>) {
        self.hovered_header = header;
    }

    pub fn set_version_hovered(&mut self, hovered: bool) {
        self.version_hovered = hovered;
    }

    pub fn set_quota_hovered(&mut self, hovered: Option<usize>) {
        self.quota_hovered = hovered;
    }

    pub fn set_pressed_header(&mut self, control: Option<WorkspaceHeaderControl>) {
        self.pressed_header = control;
    }

    fn header_hovered(&self, focus: WorkspaceFocus, id: &str) -> bool {
        self.hovered_header
            .as_ref()
            .is_some_and(|(region, target)| *region == focus && target == id)
    }
    /// Fit the scene's canonical label using the same typography as its header.
    pub fn fit_tab_text(&mut self, label: &str) -> (String, f32) {
        let width = WorkspaceScene::tab_text_width(self.size(), self.metrics);
        fit_header_text(&mut self.fonts.font_system, self.metrics, label, width)
    }

    /// Measure an exact, non-truncated Eon Bar fact label with header typography.
    pub fn measure_header_text(&mut self, label: &str) -> (String, f32) {
        fit_header_text(
            &mut self.fonts.font_system,
            self.metrics,
            label,
            f32::INFINITY,
        )
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
                1.5
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
            terminal: TerminalLayer {
                hyperlink: None,
                scrollback_label: None,
                scrollback_rect: None,
                presented_scrollback_rect: None,
                cursor_animation: CursorAnimation::default(),
                cursor_handoff: false,
                last_cursor_frame: None,
            },
            hovered_header: None,
            quota_hovered: None,
            version_hovered: false,
            pressed_header: None,
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
        shortcut_viewer: Option<&ShortcutViewerScene>,
        workspace_focus: WorkspaceFocus,
        status: &str,
        blink_visible: bool,
        preedit: &str,
        generation: u64,
    ) -> bool {
        let key = ContentKey {
            generation,
            scroll_offset,
            workspace_focus,
            blink_visible,
            preedit: preedit.to_owned(),
            status: status.to_owned(),
            hyperlink: self.terminal.hyperlink,
            hovered_header: self.hovered_header.clone(),
            quota_hovered: self.quota_hovered,
            version_hovered: self.version_hovered,
            pressed_header: self.pressed_header,
            scrollback_label: self.terminal.scrollback_label.clone(),
            shortcut_viewer_scroll: shortcut_viewer.map(|viewer| viewer.scroll),
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
            let background = scene.map_or(DEFAULT_BACKGROUND, |scene| scene.background);
            rectangles.push(
                0.0,
                workspace.tab_viewport.bottom(),
                self.config.width as f32,
                self.config.height as f32 - workspace.tab_viewport.bottom(),
                background,
                self.background_opacity,
            );
            self.build_workspace(workspace, workspace_focus, &mut rectangles);
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
            let chrome = SceneRect {
                height: workspace
                    .hints
                    .as_ref()
                    .map_or(workspace.chrome.height, |hints| {
                        (hints.rect.bottom() - workspace.chrome.top).max(0.0)
                    }),
                ..workspace.chrome
            };
            rectangles.clip = Some(workspace.pane_viewport);
            let frames = std::iter::once((chrome, workspace.panes.as_slice())).chain(
                workspace.other_columns.iter().map(|column| {
                    (
                        SceneRect {
                            height: chrome.height,
                            ..column.chrome
                        },
                        column.panes.as_slice(),
                    )
                }),
            );
            for (frame, panes) in frames.filter(|(_, panes)| self.pane_frames && !panes.is_empty())
            {
                for pane in panes.iter().skip(1) {
                    rectangles.clip = Some(SceneRect {
                        top: pane.rect.top,
                        height: 1.0,
                        ..frame
                    });
                    rectangles.push_rounded(
                        frame,
                        self.metrics.padding,
                        SceneColor {
                            r: 43,
                            g: 53,
                            b: 67,
                        },
                    );
                }
                rectangles.clip = Some(SceneRect {
                    height: workspace
                        .hints
                        .as_ref()
                        .map_or(workspace.pane_viewport.height, |hints| {
                            hints.rect.top - workspace.pane_viewport.top
                        })
                        + 1.0,
                    ..workspace.pane_viewport
                });
                rectangles.push_rounded_outline(
                    frame,
                    self.metrics.padding,
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
                rectangles.clip = Some(workspace.pane_viewport);
                rectangles.push_rounded_outline(
                    pane_chrome_rect(pane.rect, self.metrics),
                    self.metrics.padding,
                    SceneColor {
                        r: 126,
                        g: 231,
                        b: 185,
                    },
                );
            }
            rectangles.clip = None;
            if let Some(label) = workspace.popup_label()
                && let Some(terminal) = workspace.visible_terminal()
            {
                rectangles.push_rounded_outline(
                    chrome,
                    self.metrics.padding,
                    SceneColor {
                        r: 82,
                        g: 104,
                        b: 124,
                    },
                );
                let label_height = 12.0 * self.metrics.scale;
                let label_top = workspace.chrome.top;
                if terminal.top + self.metrics.padding
                    >= label_top + label_height + self.metrics.scale * 2.0
                    && terminal.height >= self.metrics.padding * 2.0 + self.metrics.height
                    && terminal.width >= self.metrics.padding * 2.0 + self.metrics.width * 4.0
                {
                    self.push_text_clipped(
                        label,
                        terminal.left + self.metrics.padding,
                        label_top,
                        terminal.width - self.metrics.padding * 2.0,
                        terminal.width - self.metrics.padding * 2.0,
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

    fn build_header_tooltip(
        &mut self,
        workspace: &WorkspaceScene,
        rectangles: &mut RectangleBatch,
    ) {
        let hovered_tab = workspace
            .tabs
            .iter()
            .find(|tab| self.header_hovered(WorkspaceFocus::Tabs, &tab.id));
        let (label, anchor) = if let Some(quota) = self
            .quota_hovered
            .and_then(|index| workspace.quotas.get(index))
        {
            (quota.description.as_str(), quota.rect)
        } else if self.version_hovered
            && let Some(version) = &workspace.version
        {
            (version.description.as_str(), version.rect)
        } else if let Some(tab) = hovered_tab {
            (tab.accessible_label(), tab.rect)
        } else if let Some(control) = workspace
            .controls
            .iter()
            .find(|control| self.header_hovered(WorkspaceFocus::Header(control.kind), ""))
        {
            (control.kind.label(), control.rect)
        } else {
            return;
        };
        let padding = self.metrics.padding;
        let top = workspace.header.bottom() + padding / 3.0;
        let width = (self.config.width as f32 - padding * 2.0).min(self.metrics.font_size * 40.0);
        let height = self.config.height as f32 - top - padding;
        if width <= padding * 2.0 || height <= padding * 2.0 {
            return;
        }
        let before = self.text.len();
        self.push_text(
            label,
            0.0,
            top + padding,
            width - padding * 2.0,
            height - padding * 2.0,
            SceneColor {
                r: 239,
                g: 244,
                b: 248,
            },
            DrawStyleKind::Status(Wrap::WordOrGlyph),
        );
        let Some(text) = self.text.get_mut(before) else {
            return;
        };
        let mut text_width = 0.0_f32;
        let mut text_height = 0.0_f32;
        for run in text.buffer.layout_runs() {
            text_width = text_width.max(run.line_w);
            text_height = text_height.max(run.line_top + run.line_height);
        }
        let width = (text_width.ceil() + padding * 2.0).min(width);
        let height = (text_height.ceil() + padding * 2.0).min(height);
        let left = anchor.left.clamp(
            padding,
            (self.config.width as f32 - width - padding).max(padding),
        );
        let tooltip = SceneRect {
            left,
            top,
            width,
            height,
        };
        if self
            .terminal
            .scrollback_rect
            .is_some_and(|pill| pill.intersection(tooltip).is_some())
        {
            self.terminal.scrollback_rect = None;
        }
        text.left = left + padding;
        text.bound_left = text.left.floor() as i32;
        text.right = (left + width - padding).ceil() as i32;
        text.bound_top = (top + padding).floor() as i32;
        text.bottom = (top + height - padding).ceil() as i32;
        self.text_overlay = Some((
            before,
            TextBounds {
                left: left.floor() as i32,
                top: top.floor() as i32,
                right: (left + width).ceil() as i32,
                bottom: (top + height).ceil() as i32,
            },
        ));
        rectangles.push_rounded(
            tooltip,
            padding / 2.0,
            SceneColor {
                r: 37,
                g: 49,
                b: 64,
            },
        );
    }

    fn build_shortcut_viewer(
        &mut self,
        viewer: &ShortcutViewerScene,
        rectangles: &mut RectangleBatch,
    ) {
        let first = self.text.len();
        let padding = self.metrics.padding.min(viewer.bounds.width / 8.0);
        rectangles.clip = None;
        rectangles.push_rounded(
            viewer.bounds,
            padding / 2.0,
            SceneColor {
                r: 16,
                g: 22,
                b: 32,
            },
        );
        rectangles.push_rounded_outline(
            viewer.bounds,
            padding / 2.0,
            SceneColor {
                r: 58,
                g: 75,
                b: 91,
            },
        );
        self.text_overlay = Some((
            first,
            TextBounds {
                left: viewer.bounds.left.floor() as i32,
                top: viewer.bounds.top.floor() as i32,
                right: viewer.bounds.right().ceil() as i32,
                bottom: viewer.bounds.bottom().ceil() as i32,
            },
        ));

        self.push_text_clipped(
            "Shortcuts",
            viewer.title.left,
            viewer.title.top,
            viewer.title.width,
            viewer.title.width,
            viewer.title.height,
            SceneColor {
                r: 239,
                g: 244,
                b: 248,
            },
            DrawStyleKind::Heading,
            viewer.bounds,
        );
        self.push_text_clipped(
            "Eon surface · physical keys",
            viewer.subtitle.left,
            viewer.subtitle.top,
            viewer.subtitle.width,
            viewer.subtitle.width,
            viewer.subtitle.height,
            SceneColor {
                r: 162,
                g: 174,
                b: 190,
            },
            DrawStyleKind::PopupLabel,
            viewer.bounds,
        );

        rectangles.clip = Some(viewer.content);
        let key_width = (self.metrics.font_size * 15.0).min(viewer.content.width * 0.4);
        let gap = padding.min(self.metrics.width);
        for group in &viewer.groups {
            self.push_text_clipped(
                &group.title,
                group.heading.left,
                group.heading.top,
                group.heading.width,
                group.heading.width,
                group.heading.height,
                SceneColor {
                    r: 126,
                    g: 231,
                    b: 185,
                },
                DrawStyleKind::PopupLabel,
                viewer.content,
            );
            for row in &group.rows {
                self.push_text_clipped(
                    &row.shortcut,
                    row.rect.left,
                    row.rect.top,
                    key_width,
                    key_width,
                    row.rect.height,
                    SceneColor {
                        r: 162,
                        g: 204,
                        b: 230,
                    },
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    viewer.content,
                );
                let action_left = row.rect.left + key_width + gap;
                self.push_text_clipped(
                    &row.action,
                    action_left,
                    row.rect.top,
                    (row.rect.right() - action_left).max(1.0),
                    (row.rect.right() - action_left).max(1.0),
                    row.rect.height,
                    SceneColor {
                        r: 219,
                        g: 226,
                        b: 234,
                    },
                    DrawStyleKind::Status(Wrap::WordOrGlyph),
                    viewer.content,
                );
            }
        }
        if viewer.max_scroll > 0.0 {
            let track = SceneRect {
                left: viewer.content.right() - self.metrics.scale * 3.0,
                top: viewer.content.top,
                width: self.metrics.scale * 2.0,
                height: viewer.content.height,
            };
            rectangles.push_rounded(
                track,
                track.width / 2.0,
                SceneColor {
                    r: 43,
                    g: 53,
                    b: 67,
                },
            );
            let thumb_height = (track.height * viewer.content.height
                / (viewer.content.height + viewer.max_scroll))
                .max(track.width * 2.0)
                .min(track.height);
            let thumb_top =
                track.top + (track.height - thumb_height) * viewer.scroll / viewer.max_scroll;
            rectangles.push_rounded(
                SceneRect {
                    top: thumb_top,
                    height: thumb_height,
                    ..track
                },
                track.width / 2.0,
                SceneColor {
                    r: 126,
                    g: 231,
                    b: 185,
                },
            );
        }
        rectangles.clip = None;
        self.push_text_clipped(
            "Wheel or ↑↓ to scroll · Esc or Alt+/ to close",
            viewer.content.left,
            viewer.content.bottom() + self.metrics.height * 0.35,
            viewer.content.width,
            viewer.content.width,
            self.metrics.height,
            SceneColor {
                r: 162,
                g: 174,
                b: 190,
            },
            DrawStyleKind::PopupLabel,
            viewer.bounds,
        );
    }

    fn build_workspace(
        &mut self,
        workspace: &WorkspaceScene,
        focus: WorkspaceFocus,
        rectangles: &mut RectangleBatch,
    ) {
        let accent = SceneColor {
            r: 126,
            g: 231,
            b: 185,
        };
        let selected = SceneColor {
            r: 28,
            g: 43,
            b: 58,
        };
        let idle = SceneColor {
            r: 16,
            g: 22,
            b: 32,
        };
        rectangles.push(
            workspace.header.left,
            workspace.header.top,
            workspace.header.width,
            workspace.header.height,
            DEFAULT_BACKGROUND,
            1.0,
        );
        if let Some(hints) = &workspace.hints {
            // Clip the upper curves to keep the shared separator straight.
            let bar = SceneRect {
                top: hints.rect.top - self.metrics.padding,
                height: hints.rect.height + self.metrics.padding,
                ..hints.rect
            };
            rectangles.clip = Some(hints.rect);
            rectangles.push_rounded(bar, self.metrics.padding, idle);
            rectangles.push_rounded_outline(
                bar,
                self.metrics.padding,
                SceneColor {
                    r: 58,
                    g: 75,
                    b: 91,
                },
            );
            rectangles.push(
                hints.rect.left,
                hints.rect.top,
                hints.rect.width,
                1.0,
                SceneColor {
                    r: 58,
                    g: 75,
                    b: 91,
                },
                1.0,
            );
            rectangles.clip = None;
            for hint in &hints.pieces {
                if !hint.modifiers.is_empty() {
                    rectangles.push_rounded(
                        SceneRect {
                            width: hint.key_offset - self.metrics.padding,
                            ..hint.rect
                        },
                        self.metrics.padding / 2.0,
                        selected,
                    );
                }
                for (text, left, width, color) in [
                    (
                        hint.modifiers.as_str(),
                        hint.rect.left + self.metrics.padding / 2.0,
                        hint.key_offset - self.metrics.padding * 1.5,
                        SceneColor {
                            r: 255,
                            g: 107,
                            b: 107,
                        },
                    ),
                    (
                        hint.key.as_str(),
                        hint.rect.left + hint.key_offset,
                        hint.label_offset - hint.key_offset,
                        accent,
                    ),
                    (
                        hint.label.as_str(),
                        hint.rect.left + hint.label_offset,
                        hint.rect.width - hint.label_offset,
                        SceneColor {
                            r: 162,
                            g: 174,
                            b: 190,
                        },
                    ),
                ]
                .into_iter()
                .filter(|(text, _, _, _)| !text.is_empty())
                {
                    self.push_text_clipped(
                        text,
                        left,
                        hint.rect.top,
                        width,
                        width,
                        hint.rect.height,
                        color,
                        DrawStyleKind::Status(Wrap::None),
                        hint.rect,
                    );
                }
            }
        }
        rectangles.clip = Some(workspace.tab_viewport);
        for tab in &workspace.tabs {
            if tab.rect.intersection(workspace.tab_viewport).is_none() {
                continue;
            }
            let radius = tab.rect.height / 2.0;
            let fill = if tab.selected {
                selected
            } else if self.header_hovered(WorkspaceFocus::Tabs, &tab.id) {
                SceneColor {
                    r: 23,
                    g: 34,
                    b: 46,
                }
            } else {
                idle
            };
            rectangles.push_rounded(tab.rect, radius, fill);
            if tab.selected && focus == WorkspaceFocus::Tabs {
                rectangles.push_rounded_outline(tab.rect, radius, accent);
            }
            let label_width = (tab.rect.width - self.metrics.padding * 2.0).max(1.0);
            self.push_text_clipped(
                tab.label(),
                tab.rect.left + self.metrics.padding,
                tab.rect.top + (tab.rect.height - self.metrics.height) / 2.0,
                label_width,
                label_width,
                self.metrics.height,
                if tab.selected {
                    SceneColor {
                        r: 239,
                        g: 244,
                        b: 248,
                    }
                } else {
                    SceneColor {
                        r: 162,
                        g: 174,
                        b: 190,
                    }
                },
                DrawStyleKind::Status(Wrap::None),
                workspace.tab_viewport,
            );
        }
        rectangles.clip = None;
        for quota in &workspace.quotas {
            let visual = header_item_visual(quota.rect, self.metrics.padding);
            rectangles.push_rounded(
                visual,
                visual.height / 2.0,
                SceneColor {
                    r: 23,
                    g: 34,
                    b: 46,
                },
            );
            self.push_quota_icon(quota.provider, quota.logo);
            let label_left = quota.logo.right() + self.metrics.padding / 2.0;
            let label_width = (quota.rect.right() - self.metrics.padding - label_left).max(0.0);
            self.push_text_clipped(
                &quota.label,
                label_left,
                quota.rect.top + (quota.rect.height - self.metrics.height).max(0.0) / 2.0,
                label_width,
                label_width,
                self.metrics.height.min(quota.rect.height),
                SceneColor {
                    r: 162,
                    g: 174,
                    b: 190,
                },
                DrawStyleKind::Status(Wrap::None),
                quota.rect,
            );
        }
        if let Some(version) = &workspace.version {
            let width = (version.rect.width - self.metrics.padding * 2.0).max(0.0);
            self.push_text_clipped(
                &version.label,
                version.rect.left + self.metrics.padding,
                version.rect.top + (version.rect.height - self.metrics.height).max(0.0) / 2.0,
                width,
                width,
                self.metrics.height.min(version.rect.height),
                SceneColor {
                    r: 124,
                    g: 196,
                    b: 216,
                },
                DrawStyleKind::Monospace,
                version.rect,
            );
        }
        for control in workspace.controls {
            if control.rect.width <= 0.0 {
                continue;
            }
            let visual = header_item_visual(control.rect, self.metrics.padding);
            let hovered = self.header_hovered(WorkspaceFocus::Header(control.kind), "");
            let pressed = self.pressed_header == Some(control.kind);
            let focused = focus == WorkspaceFocus::Header(control.kind);
            let fill = if pressed {
                selected
            } else if hovered {
                SceneColor {
                    r: 23,
                    g: 34,
                    b: 46,
                }
            } else {
                DEFAULT_BACKGROUND
            };
            rectangles.push_rounded(visual, visual.height / 2.0, fill);
            if focused {
                rectangles.push_rounded_outline(visual, visual.height / 2.0, accent);
            }
            let glyph_width = self.metrics.width.min(control.rect.width);
            self.push_text_clipped(
                control.kind.glyph(),
                control.rect.left + (control.rect.width - glyph_width).max(0.0) / 2.0,
                control.rect.top + (control.rect.height - self.metrics.height).max(0.0) / 2.0,
                glyph_width,
                glyph_width,
                self.metrics.height.min(control.rect.height),
                if pressed || focused {
                    SceneColor {
                        r: 239,
                        g: 244,
                        b: 248,
                    }
                } else {
                    SceneColor {
                        r: 162,
                        g: 174,
                        b: 190,
                    }
                },
                DrawStyleKind::Status(Wrap::None),
                control.rect,
            );
        }
        for (panes, selected_column) in workspace.column_panes() {
            for pane in panes {
                // Only the selected column's expanded header shows selection and scrollback.
                let current = pane.selected && selected_column;
                let Some(rect) = pane.rect.intersection(workspace.pane_viewport) else {
                    continue;
                };
                rectangles.push(
                    rect.left,
                    rect.top,
                    rect.width,
                    rect.height,
                    DEFAULT_BACKGROUND,
                    1.0,
                );
                rectangles.clip = Some(rect);
                if pane.selected || self.header_hovered(WorkspaceFocus::Panes, &pane.id) {
                    rectangles.push_rounded(
                        pane_chrome_rect(
                            SceneRect {
                                left: pane.rect.left,
                                width: pane.rect.width,
                                ..workspace.pane_viewport
                            },
                            self.metrics,
                        ),
                        self.metrics.padding,
                        if current { selected } else { idle },
                    );
                }
                rectangles.clip = None;
                let scrollback_width = if current {
                    self.build_scrollback(
                        SceneRect {
                            left: pane.rect.left + self.metrics.width * 4.0,
                            width: (pane.rect.width - self.metrics.width * 4.0).max(0.0),
                            ..pane.rect
                        },
                        workspace.pane_viewport,
                        true,
                        None,
                        rectangles,
                    )
                } else {
                    0.0
                };
                let width = pane.rect.width - self.metrics.padding * 2.0 - scrollback_width;
                let label = (scrollback_width > 0.0).then(|| {
                    fit_header_text(
                        &mut self.fonts.font_system,
                        self.metrics,
                        pane.label(),
                        width,
                    )
                    .0
                });
                self.push_text_clipped(
                    label.as_deref().unwrap_or(pane.label()),
                    pane.rect.left + self.metrics.padding,
                    pane.rect.top + (pane.rect.height - self.metrics.height) / 2.0,
                    width,
                    width,
                    self.metrics.height,
                    if current {
                        SceneColor {
                            r: 239,
                            g: 244,
                            b: 248,
                        }
                    } else if !pane.live {
                        SceneColor {
                            r: 221,
                            g: 126,
                            b: 126,
                        }
                    } else {
                        SceneColor {
                            r: 162,
                            g: 174,
                            b: 190,
                        }
                    },
                    DrawStyleKind::Status(Wrap::None),
                    workspace.pane_viewport,
                );
            }
        }
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
        let rect = self.notice_rect(workspace);
        if self
            .terminal
            .scrollback_rect
            .is_some_and(|pill| pill.intersection(rect).is_some())
        {
            self.terminal.scrollback_rect = None;
        }
        // Text is drawn after rectangles; clip underlying rows out of this overlay.
        for text in &mut self.text {
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
        self.push_text(
            status,
            self.metrics.padding * 2.0,
            rect.top + self.metrics.height * 0.3,
            self.config.width as f32 - self.metrics.padding * 4.0,
            rect.height - self.metrics.height * 0.3,
            SceneColor {
                r: 245,
                g: 192,
                b: 94,
            },
            if self.terminal.hyperlink.is_some() {
                DrawStyleKind::Monospace
            } else {
                DrawStyleKind::Status(Wrap::None)
            },
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

    fn push_quota_icon(&mut self, provider: QuotaProvider, rect: SceneRect) {
        let (id, color) = match provider {
            QuotaProvider::Codex => (OPENAI_BLOSSOM_ID, Color::rgb(255, 255, 255)),
            QuotaProvider::Claude => (CLAUDE_SPLAT_ID, Color::rgb(217, 119, 87)),
        };
        let mut buffer = Buffer::new_empty(Metrics::new(rect.height, rect.height));
        buffer.set_size(Some(rect.width), Some(rect.height));
        self.text.push(PlacedText {
            buffer,
            custom_glyph: Some(CustomGlyph {
                id,
                width: rect.width,
                height: rect.height,
                snap_to_physical_pixel: true,
                ..CustomGlyph::default()
            }),
            left: rect.left,
            top: rect.top,
            right: rect.right().ceil() as i32,
            bottom: rect.bottom().ceil() as i32,
            bound_left: rect.left.floor() as i32,
            bound_top: rect.top.floor() as i32,
            color,
        });
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

fn fitted_cell_font(
    font_system: &mut FontSystem,
    metrics: CellMetrics,
    family: Option<&'static str>,
    configured: bool,
) -> CellFont {
    let top_offset = ((metrics.height - metrics.font_size) / 2.0)
        .round()
        .max(0.0);
    let mut buffer = Buffer::new(font_system, Metrics::new(metrics.font_size, metrics.height));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        " ",
        &Attrs::new().family(family.map_or(Family::Monospace, Family::Name)),
        Shaping::Advanced,
        None,
    );
    let advance = buffer
        .line_layout(font_system, 0)
        .and_then(|lines| lines.first())
        .map(|line| line.w)
        .filter(|width| width.is_finite() && *width > 0.0);
    advance.map_or(
        CellFont {
            family,
            size: metrics.font_size,
            top_offset,
            letter_spacing: 0.0,
            box_size: metrics.font_size,
            box_letter_spacing: 0.0,
        },
        |advance| {
            let fitted_size = (metrics.font_size * metrics.width / advance)
                .round()
                .max(1.0);
            let size = (fitted_size - 1.0).clamp(1.0, metrics.font_size);
            let box_size = if configured {
                (fitted_size + 1.0).max(metrics.height)
            } else {
                fitted_size + 1.0
            };
            CellFont {
                family,
                size,
                top_offset,
                letter_spacing: metrics.width / size - advance / metrics.font_size,
                box_size,
                box_letter_spacing: metrics.width / box_size - advance / metrics.font_size,
            }
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellTextKind {
    Text,
    Braille,
    Box,
}

fn cell_text_kind(character: char) -> CellTextKind {
    match character {
        '\u{2500}'..='\u{257f}' => CellTextKind::Box,
        '\u{2800}'..='\u{28ff}' => CellTextKind::Braille,
        _ => CellTextKind::Text,
    }
}

fn cell_text_segments(text: &str) -> Vec<(&str, CellTextKind)> {
    let Some(first) = text.chars().next() else {
        return Vec::new();
    };
    let mut segments = Vec::new();
    let mut start = 0;
    let mut kind = cell_text_kind(first);
    for (index, character) in text.char_indices().skip(1) {
        let next_kind = cell_text_kind(character);
        if next_kind != kind {
            segments.push((&text[start..index], kind));
            start = index;
            kind = next_kind;
        }
    }
    segments.push((&text[start..], kind));
    segments
}

fn is_nerd_font_symbol(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{f2fff}'
        )
    })
}

fn block_span(text: &str, narrow: bool) -> Option<(f32, f32)> {
    match (narrow, text) {
        (true, "█") => Some((0.0, 1.0)),
        (true, "▀") => Some((0.0, 0.5)),
        (true, "▄") => Some((0.5, 0.5)),
        _ => None,
    }
}

fn cell_ink_right(run: &GlyphRun, next: Option<&GlyphRun>, columns: u16) -> u16 {
    let cell_right = run.column.saturating_add(run.columns);
    if is_nerd_font_symbol(&run.text) {
        next.map_or(columns, |next| next.column).max(cell_right)
    } else {
        cell_right
    }
}

fn cell_text_attrs(cell_font: CellFont, text: &str, style: DrawStyle) -> Attrs<'static> {
    let mut attrs = cell_font.attrs();
    if cell_font.family.is_none()
        && text
            .chars()
            .any(|character| matches!(character, '\u{100000}'..='\u{101fff}'))
    {
        attrs = attrs.family(Family::Name(ACCELERANDO_FAMILY));
    }
    if is_nerd_font_symbol(text) {
        // The packaged symbol face is Regular-only; bold can select an ambient mono face.
        if cell_font.family.is_none() {
            attrs = attrs.family(Family::Name(NERD_FONT_FAMILY));
        }
        return attrs;
    }
    if style.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    attrs
}

fn set_cell_layer(
    buffer: &mut Buffer,
    text: &str,
    segments: &[(&str, CellTextKind)],
    attrs: &Attrs<'static>,
    braille_attrs: &Attrs<'static>,
    box_attrs: &Attrs<'static>,
    box_drawing: bool,
) {
    let hidden = attrs.clone().color(Color::rgba(0, 0, 0, 0));
    buffer.set_rich_text(
        segments.iter().map(|(segment, kind)| {
            (
                *segment,
                match (*kind, box_drawing) {
                    (CellTextKind::Text, false) => attrs.clone(),
                    (CellTextKind::Braille, false) => braille_attrs.clone(),
                    (CellTextKind::Box, true) => box_attrs.clone(),
                    _ => hidden.clone(),
                },
            )
        }),
        attrs,
        shaping(text),
        None,
    );
}

fn shaping(text: &str) -> Shaping {
    if text.is_ascii() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

fn fit_header_text(
    fonts: &mut FontSystem,
    metrics: CellMetrics,
    text: &str,
    width: f32,
) -> (String, f32) {
    let mut buffer = Buffer::new(fonts, Metrics::new(metrics.font_size, metrics.height));
    buffer.set_size(None, Some(metrics.height));
    buffer.set_wrap(Wrap::None);
    let measure = |buffer: &mut Buffer, fonts: &mut FontSystem, text: &str| {
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            shaping(text),
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
    };
    let measured = measure(&mut buffer, fonts, text);
    if measured <= width {
        return (text.to_owned(), measured);
    }

    // Keep the positional prefix; cuts use the shaper's original cluster boundaries.
    let prefix = text.find("  ").map_or(0, |index| index + 2);
    let mut boundaries: Vec<_> = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .flat_map(|glyph| [glyph.start, glyph.end])
        .filter(|index| *index >= prefix)
        .collect();
    boundaries.extend([prefix, text.len()]);
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut low = 0;
    let mut high = boundaries.len().saturating_sub(2);
    let identity = text[..prefix].trim_end();
    let identity_width = measure(&mut buffer, fonts, identity);
    let mut best = if identity_width <= width {
        (identity.to_owned(), identity_width)
    } else {
        (String::new(), 0.0)
    };
    while low <= high {
        let kept = low + (high - low) / 2;
        let candidate = format!(
            "{}…{}",
            &text[..boundaries[kept.div_ceil(2)]],
            &text[boundaries[boundaries.len() - 1 - kept / 2]..]
        );
        let measured = measure(&mut buffer, fonts, &candidate);
        if measured <= width {
            best = (candidate, measured);
            low = kept + 1;
        } else if kept == 0 {
            break;
        } else {
            high = kept - 1;
        }
    }
    best
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
        let radius = radius
            .min(rect.width / 2.0)
            .min(rect.height / 2.0)
            .max(0.0)
            .floor();
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
        for row in 0..radius as u32 {
            let dy = radius - row as f32 - 0.5;
            let inset = radius - (radius * radius - dy * dy).sqrt();
            let left = rect.left + inset;
            let right = rect.right() - inset;
            for top in [rect.top + row as f32, rect.bottom() - row as f32 - 1.0] {
                self.push_span(left, right, top, color);
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

fn build_scene_rectangles(
    rectangles: &mut RectangleBatch,
    scene: &Scene,
    blink_visible: bool,
    metrics: CellMetrics,
    viewport: SceneRect,
    draw_cursor: bool,
) -> u32 {
    for (row_index, row) in scene.content.iter().enumerate() {
        build_row_rectangles(
            rectangles,
            row,
            blink_visible,
            metrics,
            viewport.left + metrics.padding,
            viewport.top + metrics.padding + row_index as f32 * metrics.height,
        );
    }
    if draw_cursor
        && let Some(cursor) = scene
            .cursor
            .filter(|cursor| cursor.visible && (blink_visible || !cursor.blinking))
        && let Some(bounds) = cursor_bounds(scene, cursor, metrics, viewport)
    {
        push_cursor(rectangles, cursor, bounds, metrics, None);
    }
    vertex_count(&rectangles.bytes)
}

fn build_preview_rectangles(
    rectangles: &mut RectangleBatch,
    scene: &Scene,
    preview: Option<&ScenePreview>,
    blink_visible: bool,
    metrics: CellMetrics,
    origin: SceneRect,
) {
    if let Some((direction, rows)) = preview_rows(preview) {
        for (index, row) in rows.iter().enumerate() {
            let top = preview_row_top(scene, direction, index, metrics, origin);
            if rectangles
                .clip
                .is_some_and(|clip| !row_intersects_clip(top, metrics.height, clip))
            {
                continue;
            }
            build_row_rectangles(
                rectangles,
                row,
                blink_visible,
                metrics,
                origin.left + metrics.padding,
                top,
            );
        }
    }
}

fn build_row_rectangles(
    rectangles: &mut RectangleBatch,
    row: &DrawRow,
    blink_visible: bool,
    metrics: CellMetrics,
    row_left: f32,
    row_top: f32,
) {
    let mut start = 0_usize;
    while start < row.cells.len() {
        let background = row.cells[start].style.background;
        let background_is_default = row.cells[start].style.background_is_default;
        let mut end = start + 1;
        while end < row.cells.len()
            && row.cells[end].style.background == background
            && row.cells[end].style.background_is_default == background_is_default
        {
            end += 1;
        }
        if !background_is_default {
            rectangles.push(
                row_left + start as f32 * metrics.width,
                row_top,
                (end - start) as f32 * metrics.width,
                metrics.height,
                background,
                1.0,
            );
        }
        start = end;
    }
    for (column, cell) in row.cells.iter().enumerate() {
        if !cell.style.foreground_visible(blink_visible) {
            continue;
        }
        let left = row_left + column as f32 * metrics.width;
        let top = row_top;
        if let Some((offset, height)) = block_span(&cell.text, cell.width == CellWidth::Narrow) {
            rectangles.push(
                left,
                top + metrics.height * offset,
                metrics.width,
                metrics.height * height,
                cell.style.foreground,
                f32::from(cell.style.foreground_alpha()) / 255.0,
            );
        }
        let thickness = (metrics.height / 14.0).max(1.0);
        match cell.style.underline {
            Underline::None => {}
            Underline::Single => rectangles.push(
                left,
                top + metrics.height - thickness * 2.0,
                metrics.width,
                thickness,
                cell.style.underline_color,
                1.0,
            ),
            Underline::Curly => {
                let segment = (metrics.width / 4.0).max(1.0);
                for part in 0..4 {
                    rectangles.push(
                        left + part as f32 * segment,
                        top + metrics.height - thickness * if part % 2 == 0 { 3.0 } else { 1.5 },
                        segment,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
            Underline::Dotted => {
                let dot = thickness.max(1.0);
                let mut x = left;
                while x < left + metrics.width {
                    rectangles.push(
                        x,
                        top + metrics.height - thickness * 2.0,
                        dot,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                    x += dot * 2.0;
                }
            }
            Underline::Dashed => {
                let dash = (metrics.width / 3.0).max(1.0);
                for part in [0.0, 2.0] {
                    rectangles.push(
                        left + part * dash,
                        top + metrics.height - thickness * 2.0,
                        dash,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
            Underline::Double => {
                for offset in [2.0, 4.0] {
                    rectangles.push(
                        left,
                        top + metrics.height - thickness * offset,
                        metrics.width,
                        thickness,
                        cell.style.underline_color,
                        1.0,
                    );
                }
            }
        }
        if cell.style.strikethrough {
            rectangles.push(
                left,
                top + metrics.height * 0.52,
                metrics.width,
                thickness,
                cell.style.foreground,
                1.0,
            );
        }
        if cell.style.overline {
            rectangles.push(
                left,
                top + thickness,
                metrics.width,
                thickness,
                cell.style.foreground,
                1.0,
            );
        }
    }
}

fn cursor_bounds(
    scene: &Scene,
    cursor: DrawCursor,
    metrics: CellMetrics,
    viewport: SceneRect,
) -> Option<SceneRect> {
    if !valid_rect(viewport)
        || !metrics.width.is_finite()
        || !metrics.height.is_finite()
        || !metrics.padding.is_finite()
        || metrics.width <= 0.0
        || metrics.height <= 0.0
        || cursor.row >= scene.rows
    {
        return None;
    }
    let column = cursor.leading_column();
    let wide = cursor.at_wide_tail
        || scene
            .content
            .get(usize::from(cursor.row))
            .and_then(|row| row.cells.get(usize::from(column)))
            .is_some_and(|cell| cell.width == CellWidth::Wide);
    let columns = if wide { 2 } else { 1 };
    if column.checked_add(columns)? > scene.columns {
        return None;
    }
    Some(SceneRect {
        left: viewport.left + metrics.padding + f32::from(column) * metrics.width,
        top: viewport.top + metrics.padding + f32::from(cursor.row) * metrics.height,
        width: metrics.width * f32::from(columns),
        height: metrics.height,
    })
}

fn push_cursor(
    rectangles: &mut RectangleBatch,
    cursor: DrawCursor,
    bounds: SceneRect,
    metrics: CellMetrics,
    outline: Option<SceneColor>,
) {
    let thickness = (metrics.width / 7.0).max(1.0);
    if let Some(color) = outline {
        match cursor.shape {
            CursorShape::Bar => rectangles.push(
                bounds.left,
                bounds.top,
                thickness + metrics.scale,
                bounds.height,
                color,
                1.0,
            ),
            CursorShape::Underline => rectangles.push(
                bounds.left,
                bounds.bottom() - thickness - metrics.scale,
                bounds.width,
                thickness + metrics.scale,
                color,
                1.0,
            ),
            CursorShape::BlockHollow => rectangles.push_hollow(
                bounds.left,
                bounds.top,
                bounds.width,
                bounds.height,
                thickness + metrics.scale,
                color,
            ),
            CursorShape::Block => {}
        }
    }
    let (x, y, width, height, alpha) = match cursor.shape {
        CursorShape::Bar => (bounds.left, bounds.top, thickness, bounds.height, 1.0),
        CursorShape::Underline => (
            bounds.left,
            bounds.bottom() - thickness,
            bounds.width,
            thickness,
            1.0,
        ),
        CursorShape::Block => (bounds.left, bounds.top, bounds.width, bounds.height, 0.55),
        CursorShape::BlockHollow => {
            rectangles.push_hollow(
                bounds.left,
                bounds.top,
                bounds.width,
                bounds.height,
                thickness,
                cursor.color,
            );
            return;
        }
    };
    rectangles.push(x, y, width, height, cursor.color, alpha);
    if cursor.shape == CursorShape::Block
        && let Some(color) = outline
    {
        rectangles.push_hollow(
            bounds.left,
            bounds.top,
            bounds.width,
            bounds.height,
            metrics.scale,
            color,
        );
    }
}

fn relative_luminance(color: SceneColor) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast_ratio(left: SceneColor, right: SceneColor) -> f64 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn cursor_outline(fill: SceneColor) -> SceneColor {
    let black = SceneColor { r: 0, g: 0, b: 0 };
    let white = SceneColor {
        r: 255,
        g: 255,
        b: 255,
    };
    let target = if contrast_ratio(fill, black) >= contrast_ratio(fill, white) {
        black
    } else {
        white
    };
    (1..=255)
        .map(|amount| {
            let blend = |source: u8, target: u8| {
                ((u16::from(source) * (255 - amount) + u16::from(target) * amount + 127) / 255)
                    as u8
            };
            SceneColor {
                r: blend(fill.r, target.r),
                g: blend(fill.g, target.g),
                b: blend(fill.b, target.b),
            }
        })
        .find(|outline| contrast_ratio(fill, *outline) >= 3.0)
        .unwrap_or(target)
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
    use super::cursor_trail::inset_cursor_quad;
    use super::*;
    use crate::{DrawCell, DrawCursor, DrawRow};
    use glyphon::{cosmic_text::SubpixelBin, fontdb};
    use orbit_protocol::{CellWidth, Screen};

    #[test]
    fn quota_icons_preserve_transparency_and_shape_across_scales() {
        for (id, original) in [
            (OPENAI_BLOSSOM_ID, OPENAI_BLOSSOM_ALPHA),
            (CLAUDE_SPLAT_ID, CLAUDE_SPLAT_ALPHA),
        ] {
            for size in [16, 20, 32, 104] {
                let image = rasterize_quota_icon(RasterizeCustomGlyphRequest {
                    id,
                    width: size,
                    height: size,
                    x_bin: SubpixelBin::Zero,
                    y_bin: SubpixelBin::Zero,
                    scale: f32::from(size) / 16.0,
                })
                .unwrap();
                assert_eq!(image.content_type, ContentType::Mask);
                assert_eq!(image.data.len(), usize::from(size * size));
                assert_eq!(image.data[0], 0, "transparent corner remains clear");
                assert!(
                    image.data.iter().any(|alpha| *alpha > 200),
                    "the mark remains visible"
                );
                if usize::from(size) == QUOTA_ICON_SIZE {
                    assert_eq!(image.data.as_slice(), original);
                }
            }
        }
        assert!(
            rasterize_quota_icon(RasterizeCustomGlyphRequest {
                id: CLAUDE_SPLAT_ID + 1,
                width: 16,
                height: 16,
                x_bin: SubpixelBin::Zero,
                y_bin: SubpixelBin::Zero,
                scale: 1.0,
            })
            .is_none()
        );
    }

    #[test]
    fn accelerando_specialty_routing_preserves_explicit_fonts_and_styles() {
        let mut fonts = FontSystem::new();
        let cell_font = fitted_cell_font(&mut fonts, CellMetrics::for_scale(1.0), None, false);
        let style = DrawStyle {
            bold: true,
            italic: true,
            ..plain_style()
        };
        for codepoint in 0x100000..=0x101fff {
            let text = char::from_u32(codepoint).unwrap().to_string();
            let attrs = cell_text_attrs(cell_font, &text, style);
            assert_eq!(attrs.family, Family::Name("Accelerando Terminal"));
            assert_eq!(attrs.weight, Weight::BOLD);
            assert_eq!(attrs.style, Style::Italic);
            let configured = CellFont {
                family: Some("DejaVu Sans Mono"),
                ..cell_font
            };
            assert_eq!(
                cell_text_attrs(configured, &text, style).family,
                Family::Name("DejaVu Sans Mono")
            );
        }
        for text in ["A", "\u{fffff}", "\u{102000}"] {
            assert_eq!(
                cell_text_attrs(cell_font, text, style).family,
                Family::Monospace
            );
        }
    }

    #[test]
    fn configured_fonts_resolve_fallbacks_and_scale_one_grid() {
        let original = FontSystem::new();
        let named = |name: &str| {
            original
                .db()
                .face(
                    original
                        .db()
                        .query(&fontdb::Query {
                            families: &[Family::Name(name)],
                            ..Default::default()
                        })
                        .expect("the accepted font environment supplies the tested face"),
                )
                .unwrap()
                .clone()
        };
        let mono = named("DejaVu Sans Mono");
        let symbol = named(NERD_FONT_FAMILY);
        let make_system = || {
            let mut db = fontdb::Database::new();
            db.push_face_info(mono.clone());
            for name in ["Venus Fallback A", "Venus Fallback B"] {
                let mut face = symbol.clone();
                face.families.truncate(1);
                face.families[0].0 = name.into();
                db.push_face_info(face);
            }
            FontSystem::new_with_locale_and_db("en-US".into(), db)
        };
        let mut settings = FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            fallbacks: vec!["Venus Fallback A".into(), "Venus Fallback B".into()],
            size: 20.0,
            line_height: 1.5,
        };
        for expected in ["Venus Fallback A", "Venus Fallback B"] {
            let mut fonts = FontSetup::resolve(&settings, make_system()).unwrap();
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let metrics = fonts.metrics(scale);
                assert_eq!(metrics.height, (30.0 * scale as f32).round());
                assert_eq!(metrics.font_size, 20.0 * scale as f32);
                let cell_font =
                    fitted_cell_font(&mut fonts.font_system, metrics, fonts.family, true);
                let mut buffer = Buffer::new(
                    &mut fonts.font_system,
                    Metrics::new(cell_font.size, metrics.height),
                );
                buffer.set_monospace_width(Some(metrics.width));
                buffer.set_text(
                    "\u{f015}",
                    &cell_text_attrs(
                        cell_font,
                        "\u{f015}",
                        DrawStyle {
                            bold: true,
                            ..plain_style()
                        },
                    ),
                    Shaping::Advanced,
                    None,
                );
                buffer.shape_until_scroll(&mut fonts.font_system, false);
                let glyph = &buffer.layout_runs().next().unwrap().glyphs[0];
                assert_ne!(glyph.glyph_id, 0);
                assert_eq!(
                    fonts.font_system.db().face(glyph.font_id).unwrap().families[0].0,
                    expected
                );
                let buffer = grid_buffer(
                    &mut fonts.font_system,
                    metrics,
                    cell_font,
                    "abc",
                    false,
                    cell_font.attrs(),
                );
                for (column, glyph) in buffer
                    .layout_runs()
                    .next()
                    .unwrap()
                    .glyphs
                    .iter()
                    .enumerate()
                {
                    assert!((glyph.x - column as f32 * metrics.width).abs() < 0.01);
                }
            }
            settings.fallbacks.reverse();
        }
        settings.family = Some("Missing Venus Font".into());
        assert!(
            FontSetup::resolve(&settings, make_system())
                .err()
                .unwrap()
                .to_string()
                .contains("unavailable")
        );
        settings.family = None;
        settings.fallbacks = vec!["Missing Venus Fallback".into()];
        assert!(
            FontSetup::resolve(&settings, make_system())
                .err()
                .unwrap()
                .to_string()
                .contains("unavailable")
        );
        settings.family = Some("DejaVu Sans".into());
        settings.fallbacks.clear();
        assert!(
            FontSetup::resolve(&settings, FontSystem::new())
                .err()
                .unwrap()
                .to_string()
                .contains("not monospace")
        );
        let defaults = FontSetup::new(&FontSettings::default()).unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            assert_eq!(defaults.metrics(scale), CellMetrics::for_scale(scale));
        }
    }

    #[test]
    fn tab_text_fits_shaped_width_without_splitting_combining_clusters() {
        let mut fonts = FontSystem::new();
        for scale in [1.0, 1.25, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let available = (crate::scene::tab_max_width(metrics, 1000.0 * scale as f32)
                - metrics.padding * 2.0)
                .floor();
            for label in ["1  eon", "2  machine_vs_aliens"] {
                let fitted = fit_header_text(&mut fonts, metrics, label, available);
                assert_eq!(fitted.0, label);
                assert!(fitted.1 > 0.0 && fitted.1 <= available);
            }
            let label = format!("3  {}", "e\u{301}".repeat(80));
            let (fitted, measured) = fit_header_text(&mut fonts, metrics, &label, available);
            assert!(measured <= available);
            let (head, tail) = fitted.split_once('…').expect("long label must elide");
            assert!(head.starts_with("3  ") && head.ends_with('\u{301}'));
            assert!(tail.starts_with('e') && tail.ends_with('\u{301}'));
            assert!(label.starts_with(head) && label.ends_with(tail));
            let exact = fit_header_text(&mut fonts, metrics, &fitted, f32::INFINITY);
            assert!((exact.1 - measured).abs() < 0.01);

            let identity_width = fit_header_text(&mut fonts, metrics, "64", f32::INFINITY).1;
            let fitted =
                fit_header_text(&mut fonts, metrics, "64  machine_vs_aliens", identity_width);
            assert_eq!(
                fitted.0, "64",
                "keep a fitting identity when the name cannot fit"
            );
            assert!(fitted.1 <= identity_width);
            assert_eq!(
                fit_header_text(&mut fonts, metrics, "64  eon", 0.0),
                (String::new(), 0.0)
            );
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
    fn long_workspace_labels_stay_on_the_visible_line() {
        use eon_workspace_protocol::v10::{Pane, Snapshot, Tab};
        use winit::{
            application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop,
            platform::wayland::EventLoopBuilderExtWayland, window::WindowId,
        };

        struct Probe(bool);
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let window = Arc::new(
                    event_loop
                        .create_window(Window::default_attributes())
                        .unwrap(),
                );
                let mut renderer = pollster::block_on(Renderer::new(
                    window,
                    event_loop,
                    1.0,
                    None,
                    true,
                    FontSetup::new(&FontSettings::default()).unwrap(),
                ))
                .unwrap();
                let snapshot = Snapshot {
                    active_tab: "t2".into(),
                    geometry: eon_workspace_protocol::v10::PopupGeometry {
                        side_margin: 8.0,
                        vertical_margin: 4.0,
                    },
                    entries: Vec::new(),
                    tabs: ["eon", "machines_vs_aliens"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, directory)| Tab {
                            pending: false,
                            selected_popup: None,
                            popups: Vec::new(),
                            id: format!("t{}", index + 1),
                            directory: format!("/tmp/{directory}").into_bytes(),
                            selected_pane: Some(format!("p{}", index + 1)),
                            columns: vec![eon_workspace_protocol::v10::Column {
                                panes: vec![Pane {
                                    id: format!("p{}", index + 1),
                                    session: format!("session-{}", index + 1),
                                    endpoint: format!("/tmp/session-{}.sock", index + 1)
                                        .into_bytes(),
                                    live: true,
                                }],
                                expanded: format!("p{}", index + 1),
                            }],
                        })
                        .collect(),
                    codex_quota: None,
                    claude_quota: None,
                };
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                renderer.build_workspace(
                    &workspace,
                    WorkspaceFocus::Terminal,
                    &mut RectangleBatch::new(900, 600),
                );
                let label = &renderer.text[1];
                let line = label.buffer.layout_runs().next().unwrap();
                assert_eq!(
                    line.glyphs.last().unwrap().end,
                    workspace.tabs[1].label().len(),
                    "the directory name wrapped below the visible tab header"
                );
                assert!(workspace.tabs[1].rect.width > workspace.tabs[0].rect.width);
                assert!(line.line_w <= (label.right - label.bound_left) as f32);
                let underscore = line
                    .glyphs
                    .iter()
                    .find(|glyph| &line.text[glyph.start..glyph.end] == "_")
                    .unwrap()
                    .physical((label.left, label.top), 1.0);
                let ink = renderer
                    .swash_cache
                    .get_image(&mut renderer.fonts.font_system, underscore.cache_key)
                    .as_ref()
                    .unwrap();
                let top = line.line_y.round() as i32 + underscore.y - ink.placement.top;
                assert!(ink.placement.height > 0);
                assert!(
                    top >= label.bound_top && top + ink.placement.height as i32 <= label.bottom,
                    "the directory underscore is clipped outside the header"
                );
                renderer.set_hovered_header(Some((WorkspaceFocus::Tabs, "t2".into())));
                renderer.build_header_tooltip(&workspace, &mut RectangleBatch::new(900, 600));
                let (first, overlay) = renderer.text_overlay.unwrap();
                let areas: Vec<_> =
                    text_areas(&renderer.text[..first], renderer.text_overlay).collect();
                assert!(
                    areas.iter().all(|area| {
                        let b = area.bounds;
                        b.right <= overlay.left
                            || b.left >= overlay.right
                            || b.bottom <= overlay.top
                            || b.top >= overlay.bottom
                    }),
                    "underlying text must not paint over the hover path"
                );
                assert_eq!(
                    renderer.text[first].buffer.lines[0].text(),
                    "Tab 2 of 2  /tmp/machines_vs_aliens"
                );
                let mut snapshot = snapshot;
                snapshot.tabs[1].columns[0].panes.push(Pane {
                    id: "p3".into(),
                    session: "s3".into(),
                    endpoint: b"/tmp/p3.sock".to_vec(),
                    live: false,
                });
                snapshot.tabs[1].columns[0].panes[0].live = false;
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label),
                );
                renderer.pane_frames = false;
                renderer.set_hovered_header(None);
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    1,
                );
                let pane_color = |label| {
                    renderer
                        .text
                        .iter()
                        .find(|text| text.buffer.lines[0].text() == label)
                        .unwrap()
                        .color
                };
                assert_ne!(
                    pane_color("p2 offline"),
                    pane_color("p3 offline"),
                    "offline pane selection disappears with decorative frames off"
                );
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    None,
                    None,
                    WorkspaceFocus::Terminal,
                    &"Sessions unavailable. ".repeat(32),
                    false,
                    "",
                    1,
                );
                assert!(renderer.text[1].buffer.layout_runs().count() > 1);
                snapshot.tabs[1].columns[0].panes[0].live = true;
                let metadata = crate::scene::PaneMetadata::Available {
                    working_directory: format!("/{}mountain/leaf", "W".repeat(46)),
                };
                let workspace = WorkspaceScene::from_snapshot_with_metadata(
                    &snapshot,
                    None,
                    PhysicalSize::new(900, 600),
                    renderer.metrics(),
                    0.0,
                    0.0,
                    |_| Some(&metadata),
                    |_, label| renderer.fit_tab_text(label),
                    None,
                );
                renderer.set_scrollback_label(Some("↓ 240 rows".into()));
                renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    2,
                );
                let indicator = renderer
                    .text
                    .iter()
                    .find(|text| text.buffer.lines[0].text() == "↓ 240 rows")
                    .unwrap();
                let pill = renderer.terminal.scrollback_rect.unwrap();
                assert!(pill.contains(pill.left + 1.0, pill.top + 1.0));
                let pane = workspace.panes.iter().find(|pane| pane.selected).unwrap();
                assert!(
                    fit_header_text(
                        &mut renderer.fonts.font_system,
                        renderer.metrics,
                        pane.label(),
                        f32::INFINITY
                    )
                    .1 <= pane.rect.width - renderer.metrics.padding * 2.0,
                    "the label fits before reserving the scrollback count"
                );
                assert!(indicator.top >= pane.rect.top);
                assert!(indicator.bottom as f32 <= pane.rect.bottom());
                let pane_text = renderer
                    .text
                    .iter()
                    .find(|text| text.buffer.lines[0].text().starts_with("p2  "))
                    .unwrap();
                assert!(pane_text.right <= indicator.bound_left);
                let line = pane_text.buffer.layout_runs().next().unwrap();
                assert!(line.text.ends_with("/leaf"));
                assert!(
                    line.line_w <= (pane_text.right - pane_text.bound_left) as f32,
                    "the scrollback count clips the directory leaf"
                );
                renderer.set_scrollback_label(None);
                assert!(renderer.rebuild_if_needed(
                    None,
                    None,
                    0.0,
                    Some(&workspace),
                    None,
                    WorkspaceFocus::Terminal,
                    "",
                    false,
                    "",
                    2
                ));
                assert!(
                    renderer
                        .text
                        .iter()
                        .all(|text| !text.buffer.lines[0].text().starts_with('↓'))
                );
                assert!(renderer.terminal.scrollback_rect.is_none());
                renderer.set_scrollback_label(Some("↓ 18446744073709551615 rows".into()));
                let viewport = SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: 900.0,
                    height: 600.0,
                };
                let mut rectangles = RectangleBatch::new(900, 600);
                assert!(
                    renderer.build_scrollback(viewport, viewport, false, None, &mut rectangles)
                        > 0.0
                );
                let (first, overlay) = renderer.text_overlay.unwrap();
                assert_eq!(
                    renderer.text[first].buffer.lines[0].text(),
                    "↓ 18446744073709551615 rows"
                );
                assert!(
                    text_areas(&renderer.text[..first], renderer.text_overlay).all(|area| {
                        let b = area.bounds;
                        b.right <= overlay.left
                            || b.left >= overlay.right
                            || b.bottom <= overlay.top
                            || b.top >= overlay.bottom
                    })
                );
                assert_eq!(
                    renderer.build_scrollback(
                        viewport,
                        viewport,
                        false,
                        Some(viewport),
                        &mut rectangles
                    ),
                    0.0,
                    "the indicator must yield to an overlapping terminal cursor"
                );
                let narrow = SceneRect {
                    width: 40.0,
                    ..viewport
                };
                let count = renderer.text.len();
                assert_eq!(
                    renderer.build_scrollback(narrow, narrow, false, None, &mut rectangles),
                    0.0
                );
                assert_eq!(
                    renderer.text.len(),
                    count,
                    "never clip digits into a different number"
                );
                self.0 = true;
                event_loop.exit();
            }

            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        let event_loop = EventLoop::builder()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let mut probe = Probe(false);
        event_loop.run_app(&mut probe).unwrap();
        assert!(probe.0);
    }

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
            scroll_offset: 0.0,
            workspace_focus: WorkspaceFocus::Terminal,
            blink_visible: true,
            preedit: String::new(),
            status: String::new(),
            hyperlink: None,
            hovered_header: None,
            quota_hovered: None,
            version_hovered: false,
            pressed_header: None,
            scrollback_label: None,
            shortcut_viewer_scroll,
        };

        assert_ne!(key(1, None), key(2, None));
        assert_ne!(key(1, None), key(1, Some(0.0)));
        assert_ne!(key(1, Some(0.0)), key(1, Some(1.0)));
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

    fn plain_style() -> DrawStyle {
        DrawStyle {
            foreground: SceneColor::default(),
            background: DEFAULT_BACKGROUND,
            underline_color: SceneColor::default(),
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            selected: false,
            background_is_default: true,
            protected: false,
            underline: Underline::None,
        }
    }

    fn grid_buffer(
        font_system: &mut FontSystem,
        metrics: CellMetrics,
        cell_font: CellFont,
        text: &str,
        box_drawing: bool,
        attrs: Attrs<'static>,
    ) -> Buffer {
        let attrs = attrs.letter_spacing(cell_font.letter_spacing);
        let box_attrs = attrs
            .clone()
            .metrics(Metrics::new(cell_font.box_size, metrics.height))
            .letter_spacing(cell_font.box_letter_spacing);
        let braille_attrs = attrs.clone().family(Family::Name(BRAILLE_FAMILY));
        let mut buffer = Buffer::new(font_system, Metrics::new(cell_font.size, metrics.height));
        buffer.set_size(
            Some(metrics.width * text.chars().count() as f32),
            Some(metrics.height),
        );
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(metrics.width));
        set_cell_layer(
            &mut buffer,
            text,
            &cell_text_segments(text),
            &attrs,
            &braille_attrs,
            &box_attrs,
            box_drawing,
        );
        buffer.shape_until_scroll(font_system, false);
        buffer
    }

    #[test]
    fn concealed_cells_draw_no_foreground_elements() {
        let style = DrawStyle {
            blink: true,
            invisible: true,
            strikethrough: true,
            overline: true,
            underline: Underline::Single,
            ..plain_style()
        };
        let scene = Scene {
            revision: 1,
            columns: 1,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground: SceneColor::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: vec![DrawCell {
                    width: CellWidth::Narrow,
                    text: "secret".into(),
                    hyperlink: String::new(),
                    style,
                }],
            }],
        };
        let mut rectangles = RectangleBatch::new(100, 100);

        build_scene_rectangles(
            &mut rectangles,
            &scene,
            true,
            CellMetrics::for_scale(1.0),
            SceneRect {
                left: 0.0,
                top: 0.0,
                width: 100.0,
                height: 100.0,
            },
            true,
        );

        assert!(rectangles.bytes.is_empty());
        assert!(scene.glyph_runs().is_empty());
        assert!(!scene.has_blinking_content());
    }

    #[test]
    fn block_cells_use_exact_cell_rectangles() {
        let colors = [
            SceneColor { r: 255, g: 0, b: 0 },
            SceneColor { r: 255, g: 0, b: 0 },
            SceneColor { r: 0, g: 0, b: 255 },
            SceneColor {
                r: 255,
                g: 0,
                b: 255,
            },
        ];
        for (text, offset, height) in [("█", 0.0, 1.0), ("▀", 0.0, 0.5), ("▄", 0.5, 0.5)] {
            let scene = Scene {
                revision: 1,
                columns: 2,
                rows: 2,
                screen: Screen::Primary,
                title: String::new(),
                working_directory: String::new(),
                background: DEFAULT_BACKGROUND,
                foreground: SceneColor::default(),
                cursor: None,
                content: colors
                    .chunks_exact(2)
                    .map(|row| DrawRow {
                        wrapped: false,
                        wrap_continuation: false,
                        kitty_virtual_placeholder: false,
                        cells: row
                            .iter()
                            .map(|foreground| DrawCell {
                                width: CellWidth::Narrow,
                                text: text.into(),
                                hyperlink: String::new(),
                                style: DrawStyle {
                                    foreground: *foreground,
                                    faint: *foreground == colors[2],
                                    ..plain_style()
                                },
                            })
                            .collect(),
                    })
                    .collect(),
            };

            let glyph_runs = scene.glyph_runs();
            assert_eq!(glyph_runs.len(), 4);
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let metrics = CellMetrics::for_scale(scale);
                let viewport = SceneRect {
                    left: 7.0,
                    top: 11.0,
                    width: 200.0,
                    height: 200.0,
                };
                let mut actual = RectangleBatch::new(300, 300);
                build_scene_rectangles(&mut actual, &scene, true, metrics, viewport, true);
                let mut expected = RectangleBatch::new(300, 300);
                for (index, color) in colors.into_iter().enumerate() {
                    expected.push(
                        viewport.left + metrics.padding + (index % 2) as f32 * metrics.width,
                        viewport.top
                            + metrics.padding
                            + ((index / 2) as f32 + offset) * metrics.height,
                        metrics.width,
                        metrics.height * height,
                        color,
                        if index == 2 { 150.0 / 255.0 } else { 1.0 },
                    );
                }
                assert_eq!(actual.bytes, expected.bytes, "{text}, scale {scale}");
            }
        }
    }

    #[test]
    fn half_blocks_preserve_clipped_preview_styles() {
        let foreground = SceneColor { r: 255, g: 0, b: 0 };
        let background = SceneColor { r: 0, g: 0, b: 255 };
        let row = DrawRow {
            wrapped: false,
            wrap_continuation: false,
            kitty_virtual_placeholder: false,
            cells: [
                ("▄", true, false, false),
                ("▀", false, true, false),
                ("▄", false, false, true),
            ]
            .into_iter()
            .map(|(text, faint, blink, invisible)| DrawCell {
                width: CellWidth::Narrow,
                text: text.into(),
                hyperlink: String::new(),
                style: DrawStyle {
                    foreground,
                    background,
                    background_is_default: false,
                    faint,
                    blink,
                    invisible,
                    ..plain_style()
                },
            })
            .collect(),
        };
        let scene = Scene {
            revision: 1,
            columns: 3,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground,
            cursor: None,
            content: vec![row.clone()],
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let origin = SceneRect {
                left: 7.0,
                top: 50.0,
                width: 200.0,
                height: 200.0,
            };
            for direction in [VerticalDirection::Up, VerticalDirection::Down] {
                let preview = ScenePreview::Viewport {
                    frame_revision: 1,
                    direction,
                    edge_reached: false,
                    rows: vec![row.clone()],
                };
                let left = origin.left + metrics.padding;
                let top = origin.top
                    + metrics.padding
                    + if direction == VerticalDirection::Up {
                        -metrics.height
                    } else {
                        metrics.height
                    };
                let clip = SceneRect {
                    left: left + metrics.width * 0.5,
                    top: top + metrics.height * 0.25,
                    width: metrics.width * 2.0,
                    height: metrics.height * 0.5,
                };
                for blink_visible in [false, true] {
                    let mut actual = RectangleBatch::new(300, 300);
                    actual.clip = Some(clip);
                    build_preview_rectangles(
                        &mut actual,
                        &scene,
                        Some(&preview),
                        blink_visible,
                        metrics,
                        origin,
                    );
                    let mut expected = RectangleBatch::new(300, 300);
                    expected.clip = Some(clip);
                    expected.push(
                        left,
                        top,
                        metrics.width * 3.0,
                        metrics.height,
                        background,
                        1.0,
                    );
                    expected.push(
                        left,
                        top + metrics.height * 0.5,
                        metrics.width,
                        metrics.height * 0.5,
                        foreground,
                        150.0 / 255.0,
                    );
                    if blink_visible {
                        expected.push(
                            left + metrics.width,
                            top,
                            metrics.width,
                            metrics.height * 0.5,
                            foreground,
                            1.0,
                        );
                    }
                    assert_eq!(
                        actual.bytes, expected.bytes,
                        "{direction:?}, {scale}, blink {blink_visible}"
                    );
                }
            }
        }
        for (text, narrow) in [
            ("▀\u{301}", true),
            ("▄▄", true),
            ("▌", true),
            ("text", true),
            ("▀", false),
            ("▄", false),
        ] {
            assert!(
                block_span(text, narrow).is_none(),
                "{text}, narrow {narrow}"
            );
        }
    }

    #[test]
    fn wide_cell_cursors_cover_the_complete_glyph() {
        let style = plain_style();
        let mut scene = Scene {
            revision: 1,
            columns: 3,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground: SceneColor::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: [CellWidth::Narrow, CellWidth::Wide, CellWidth::SpacerTail]
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
        let metrics = CellMetrics::for_scale(1.0);
        let cursor = DrawCursor {
            visible: true,
            blinking: false,
            password_input: false,
            shape: CursorShape::Block,
            column: 0,
            row: 0,
            at_wide_tail: false,
            color: SceneColor::default(),
            explicit_color: false,
        };
        let mut draw_cursor = |cursor| {
            scene.cursor = Some(cursor);
            let mut rectangles = RectangleBatch::new(100, 100);
            build_scene_rectangles(
                &mut rectangles,
                &scene,
                true,
                metrics,
                SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                true,
            );
            rectangles.bytes
        };
        let expected = |column: f32, columns: f32| {
            let mut rectangles = RectangleBatch::new(100, 100);
            rectangles.push(
                metrics.padding + column * metrics.width,
                metrics.padding,
                columns * metrics.width,
                metrics.height,
                cursor.color,
                0.55,
            );
            rectangles.bytes
        };

        assert_eq!(draw_cursor(cursor), expected(0.0, 1.0));
        assert_eq!(
            draw_cursor(DrawCursor {
                column: 1,
                ..cursor
            }),
            expected(1.0, 2.0)
        );
        assert_eq!(
            draw_cursor(DrawCursor {
                column: 2,
                at_wide_tail: true,
                ..cursor
            }),
            expected(1.0, 2.0)
        );
    }

    #[test]
    fn cursor_outline_is_contrasting_and_one_logical_pixel() {
        for fill in [
            SceneColor { r: 0, g: 0, b: 0 },
            SceneColor {
                r: 0x7f,
                g: 0x7f,
                b: 0x7f,
            },
            SceneColor {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
        ]
        .into_iter()
        .chain(
            [
                0xff3b30, 0xffd23f, 0xb7f34a, 0x35c978, 0x7ddcff, 0x5271ff, 0xa970ff, 0xff5da2,
            ]
            .map(|value| SceneColor {
                r: (value >> 16) as u8,
                g: (value >> 8) as u8,
                b: value as u8,
            }),
        ) {
            let outline = cursor_outline(fill);
            assert!(contrast_ratio(fill, outline) >= 3.0, "{fill:?} {outline:?}");
            if relative_luminance(fill) > 0.5 {
                assert!(relative_luminance(outline) < relative_luminance(fill));
            } else if relative_luminance(fill) < 0.05 {
                assert!(relative_luminance(outline) > relative_luminance(fill));
            }
        }

        assert_eq!(
            inset_cursor_quad(
                [
                    CursorPoint { x: 10.0, y: 20.0 },
                    CursorPoint { x: 30.0, y: 20.0 },
                    CursorPoint { x: 30.0, y: 50.0 },
                    CursorPoint { x: 10.0, y: 50.0 },
                ],
                2.0,
            ),
            [
                CursorPoint { x: 12.0, y: 22.0 },
                CursorPoint { x: 28.0, y: 22.0 },
                CursorPoint { x: 28.0, y: 48.0 },
                CursorPoint { x: 12.0, y: 48.0 },
            ]
        );
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
    fn default_font_matches_nova_at_supported_scales() {
        let mut font_system = FontSystem::new();
        for (scale, width, height, font_size, padding) in [
            (1.0, 10.0, 18.0, 16.0, 12.0),
            (1.25, 13.0, 23.0, 20.0, 15.0),
            (1.5, 15.0, 27.0, 24.0, 18.0),
            (2.0, 20.0, 36.0, 32.0, 24.0),
        ] {
            let metrics = CellMetrics::for_scale(scale);
            assert_eq!(
                (
                    metrics.width,
                    metrics.height,
                    metrics.font_size,
                    metrics.padding
                ),
                (width, height, font_size, padding)
            );
            assert_eq!(
                fitted_cell_font(&mut font_system, metrics, None, false).size,
                font_size
            );
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
    fn background_opacity_skips_only_default_cell_backgrounds() {
        let mut scene = Scene {
            revision: 1,
            columns: 4,
            rows: 1,
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: DEFAULT_BACKGROUND,
            foreground: SceneColor::default(),
            cursor: None,
            content: vec![DrawRow {
                wrapped: false,
                wrap_continuation: false,
                kitty_virtual_placeholder: false,
                cells: (0..4)
                    .map(|_| DrawCell {
                        width: CellWidth::Narrow,
                        text: String::new(),
                        hyperlink: String::new(),
                        style: plain_style(),
                    })
                    .collect(),
            }],
        };
        scene.content[0].cells[1].style.background_is_default = false;
        scene.content[0].cells[2].style.background_is_default = false;
        scene.content[0].cells[3].style.background_is_default = false;

        let metrics = CellMetrics::for_scale(1.0);
        let viewport = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let mut actual = RectangleBatch::new(100, 100);
        build_scene_rectangles(&mut actual, &scene, true, metrics, viewport, true);
        let mut expected = RectangleBatch::new(100, 100);
        expected.push(
            metrics.padding + metrics.width,
            metrics.padding,
            metrics.width * 3.0,
            metrics.height,
            DEFAULT_BACKGROUND,
            1.0,
        );

        assert_eq!(actual.bytes, expected.bytes);
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
    fn text_runs_follow_the_cell_grid_and_box_borders_connect() {
        let mut font_system = FontSystem::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for (text, box_drawing, attrs) in [
                ("narrow text", false, cell_font.attrs()),
                ("EON 9.8.7-alpha.2+proof", false, cell_font.attrs()),
                (
                    "┌──┬──┐",
                    true,
                    cell_font
                        .attrs()
                        .weight(Weight::BOLD)
                        .color(Color::rgb(80, 200, 160)),
                ),
                ("italic text", false, cell_font.attrs().style(Style::Italic)),
            ] {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    text,
                    box_drawing,
                    attrs,
                );
                let glyphs = buffer
                    .layout_runs()
                    .next()
                    .expect("the terminal run should shape")
                    .glyphs;
                assert_eq!(glyphs.len(), text.chars().count());
                for (column, glyph) in glyphs.iter().enumerate() {
                    assert!((glyph.x - column as f32 * metrics.width).abs() < 0.01);
                    assert!(
                        (glyph.w - metrics.width).abs() < 0.01,
                        "{text:?} column {column} at scale {scale} has width {} instead of {}",
                        glyph.w,
                        metrics.width
                    );
                }
            }

            let mixed = "e\u{301}│abc│";
            let segments = cell_text_segments(mixed);
            assert_eq!(
                segments,
                [
                    ("e\u{301}", CellTextKind::Text),
                    ("│", CellTextKind::Box),
                    ("abc", CellTextKind::Text),
                    ("│", CellTextKind::Box)
                ]
            );
            let layer_widths = [false, true].map(|box_drawing| {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    mixed,
                    box_drawing,
                    cell_font.attrs(),
                );
                shaped_preedit_placement(&buffer).1
            });
            for layer_width in layer_widths {
                assert!(
                    (layer_width - metrics.width * 6.0).abs() < 0.01,
                    "mixed combining and box text at scale {scale} occupies {layer_width} instead of {}",
                    metrics.width * 6.0
                );
            }
            let fallback_widths = [false, true].map(|box_drawing| {
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    "אבג│",
                    box_drawing,
                    cell_font.attrs(),
                );
                shaped_preedit_placement(&buffer).1
            });
            assert!(
                (fallback_widths[0] - fallback_widths[1]).abs() < 0.01,
                "mixed fallback and box layers diverge at scale {scale}: {fallback_widths:?}"
            );

            let buffer = grid_buffer(
                &mut font_system,
                metrics,
                cell_font,
                "│",
                true,
                cell_font.attrs(),
            );
            let run = buffer
                .layout_runs()
                .next()
                .expect("the vertical border should shape");
            let glyph = run
                .glyphs
                .first()
                .expect("the vertical border should exist");
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let image = SwashCache::new()
                .get_image_uncached(&mut font_system, physical.cache_key)
                .expect("the vertical border glyph should rasterize");
            let top = run.line_y.round() as i32 + physical.y - image.placement.top;
            let bottom = top + image.placement.height as i32;
            assert!(
                top <= 0 && bottom >= metrics.height as i32,
                "vertical border at scale {scale} occupies {top}..{bottom} in a {} px row",
                metrics.height
            );
        }

        let metrics = CellMetrics::for_scale(1.0);
        let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
        let buffer = grid_buffer(
            &mut font_system,
            metrics,
            cell_font,
            "─────",
            true,
            cell_font.attrs(),
        );
        let glyphs = buffer
            .layout_runs()
            .next()
            .expect("the table border should shape")
            .glyphs;
        let mut swash_cache = SwashCache::new();
        let ink = glyphs
            .iter()
            .map(|glyph| {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the table border glyph should rasterize");
                let left = physical.x + image.placement.left;
                (left, left + image.placement.width as i32)
            })
            .collect::<Vec<_>>();
        for pair in ink.windows(2) {
            assert!(
                pair[0].1 >= pair[1].0,
                "adjacent table-border cells must not expose a gap: {pair:?}"
            );
        }
    }

    #[test]
    fn braille_progress_uses_distinct_packaged_glyphs_on_the_cell_grid() {
        const SPINNER: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
        assert_eq!(
            cell_text_segments(&format!("{SPINNER}│X")),
            [
                (SPINNER, CellTextKind::Braille),
                ("│", CellTextKind::Box),
                ("X", CellTextKind::Text),
            ]
        );

        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for frame in SPINNER.chars() {
                let text = format!("{frame}│X");
                let buffer = grid_buffer(
                    &mut font_system,
                    metrics,
                    cell_font,
                    &text,
                    false,
                    cell_font.attrs(),
                );
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the Braille frame should shape");
                assert_eq!(run.glyphs.len(), 3);
                let glyph = &run.glyphs[0];
                let face = font_system
                    .db()
                    .face(glyph.font_id)
                    .expect("the Braille face should remain loaded");
                assert!(
                    face.families.iter().any(|(name, _)| name == BRAILLE_FAMILY),
                    "{frame} at scale {scale} used {}",
                    face.post_script_name
                );
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the Braille frame should rasterize");
                let left = physical.x + image.placement.left;
                let right = left + image.placement.width as i32;
                let top = run.line_y.round() as i32 + physical.y
                    - image.placement.top
                    - cell_font.top_offset as i32;
                let bottom = top + image.placement.height as i32;
                assert!(
                    left >= 0
                        && right <= metrics.width as i32
                        && top >= 0
                        && bottom <= metrics.height as i32,
                    "{frame} at scale {scale} occupies {left}..{right} by {top}..{bottom} in a {} by {} cell",
                    metrics.width,
                    metrics.height
                );
            }
        }
    }

    #[test]
    fn descender_rasters_fit_the_cell_row_at_supported_scales() {
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let mut failures = Vec::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let metrics = CellMetrics::for_scale(scale);
            let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
            for (style, attrs) in [
                ("regular", cell_font.attrs()),
                ("bold", cell_font.attrs().weight(Weight::BOLD)),
                ("italic", cell_font.attrs().style(Style::Italic)),
            ] {
                let text = "gjpqy";
                let mut buffer = Buffer::new(
                    &mut font_system,
                    Metrics::new(cell_font.size, metrics.height),
                );
                buffer.set_size(
                    Some(metrics.width * text.chars().count() as f32),
                    Some(metrics.height),
                );
                buffer.set_wrap(Wrap::None);
                buffer.set_monospace_width(Some(metrics.width));
                buffer.set_text(text, &attrs, shaping(text), None);
                buffer.shape_until_scroll(&mut font_system, false);
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the descender corpus should shape");
                assert_eq!(run.glyphs.len(), text.chars().count());
                for (character, glyph) in text.chars().zip(run.glyphs) {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    let image = swash_cache
                        .get_image_uncached(&mut font_system, physical.cache_key)
                        .expect("the descender glyph should rasterize");
                    let top = run.line_y.round() as i32 + physical.y
                        - image.placement.top
                        - cell_font.top_offset as i32;
                    let bottom = top + image.placement.height as i32;
                    if top < 0 || bottom > metrics.height as i32 {
                        failures.push(format!(
                            "{style} {character:?} at scale {scale} occupies {top}..{bottom} in a {} px row",
                            metrics.height
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn nerd_font_symbols_use_the_packaged_face_and_bounded_blank_space() {
        assert!(is_nerd_font_symbol("\u{e000}"));
        assert!(is_nerd_font_symbol("\u{f8ff}"));
        assert!(is_nerd_font_symbol("\u{f0000}"));
        assert!(is_nerd_font_symbol("\u{f2fff}"));
        assert!(!is_nerd_font_symbol("\u{f900}"));
        assert!(!is_nerd_font_symbol("\u{f3000}"));

        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let metrics = CellMetrics::for_scale(1.0);
        let cell_font = fitted_cell_font(&mut font_system, metrics, None, false);
        let mut observed_overhang = false;
        for symbol in ["\u{e5ff}", "\u{f015}", "\u{f15b}", "\u{f489}", "\u{f0868}"] {
            let geometry = [false, true].map(|bold| {
                let attrs = cell_text_attrs(
                    cell_font,
                    symbol,
                    DrawStyle {
                        bold,
                        ..plain_style()
                    },
                );
                assert_eq!(attrs.weight, Weight::NORMAL);
                let buffer =
                    grid_buffer(&mut font_system, metrics, cell_font, symbol, false, attrs);
                let run = buffer
                    .layout_runs()
                    .next()
                    .expect("the Nerd Font symbol should shape");
                let glyph = run
                    .glyphs
                    .first()
                    .expect("the Nerd Font symbol should emit one glyph");
                let face = font_system
                    .db()
                    .face(glyph.font_id)
                    .expect("the selected symbol face should remain loaded");
                assert!(
                    face.families
                        .iter()
                        .any(|(family, _)| family == NERD_FONT_FAMILY),
                    "{symbol:?} used {}",
                    face.post_script_name
                );
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = swash_cache
                    .get_image_uncached(&mut font_system, physical.cache_key)
                    .expect("the Nerd Font symbol should rasterize");
                let left = physical.x + image.placement.left;
                let right = left + image.placement.width as i32;
                observed_overhang |= right > metrics.width as i32;
                assert!(
                    left >= 0 && right <= (metrics.width * 2.0) as i32,
                    "{symbol:?} occupies {left}..{right} with one blank cell available"
                );
                (
                    glyph.font_id,
                    glyph.font_size.to_bits(),
                    glyph.w.to_bits(),
                    left,
                    right,
                )
            });
            assert_eq!(geometry[0], geometry[1]);
        }
        assert!(
            observed_overhang,
            "the corpus must exercise the clipping defect"
        );

        let symbol = GlyphRun {
            column: 0,
            row: 0,
            columns: 1,
            text: "\u{f015}".into(),
            style: plain_style(),
        };
        let label = GlyphRun {
            column: 2,
            text: "x".into(),
            ..symbol.clone()
        };
        assert_eq!(cell_ink_right(&symbol, Some(&label), 4), 2);
        assert_eq!(
            cell_ink_right(
                &symbol,
                Some(&GlyphRun {
                    column: 1,
                    ..label.clone()
                }),
                4,
            ),
            1
        );
        assert_eq!(cell_ink_right(&label, None, 4), 3);
    }

    #[test]
    fn ascii_uses_basic_shaping_without_weakening_unicode() {
        assert_eq!(shaping("plain ASCII"), Shaping::Basic);
        assert_eq!(shaping("e\u{301}"), Shaping::Advanced);
    }
}
