//! Terminal images (VEN-C31): the resource store and its budget, textures, the textured-quad
//! pipeline, and the band each terminal's images and markers occupy between its cell backgrounds
//! and its foreground.

use super::{
    CellMetrics, RectangleBatch, RenderError, SceneRect, upload_vertices, valid_rect, vertex_count,
};
use crate::{
    model::{ImageKey, ImageResource},
    scene::{Color as SceneColor, SceneGraphics, ScenePlacement},
};
use orbit_protocol::{PlacementState, session::PixelFormat};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    time::{Duration, Instant},
};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    BufferDescriptor, BufferUsages, COPY_BYTES_PER_ROW_ALIGNMENT, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, ErrorFilter, Extent3d, FilterMode, FragmentState, MultisampleState,
    Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor, PollType, PrimitiveState,
    RenderPass, RenderPipeline, RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension, VertexAttribute,
    VertexBufferLayout, VertexFormat, VertexState, VertexStepMode, WriteOnly,
};

/// Position and texture coordinate, two `f32` pairs.
const IMAGE_VERTEX_SIZE: u64 = 16;

/// Image textures across the whole window, counted at 4 bytes per pixel (VEN-C31).
const IMAGE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// How long a pending entry draws nothing, so a resource that promptly follows its frame never
/// flashes the marker (VEN-C31).
const MARKER_DELAY: Duration = Duration::from_millis(250);

/// The marker's outline and fill opacity over its terminal's foreground color. The colors derive
/// from each terminal's own foreground, so they follow its theme without a setting.
const MARKER_OUTLINE_ALPHA: f32 = 0.5;
const MARKER_FILL_ALPHA: f32 = 0.12;

/// One image in a terminal's image band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SceneImage {
    /// The held resource whose texture draws.
    texture: ImageKey,
    /// The drawn part of the texture, from 0 to 1 on each axis.
    source: SceneRect,
    /// Where the image lands, in cells from the grid's top-left corner, so the current cell size
    /// places and scales it.
    target: SceneRect,
}

/// One entry of a terminal's image band, in the frame's order (VEN-C31).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BandItem {
    Image(SceneImage),
    /// The missing-image marker over an entry's target, in cells from the grid's top-left corner.
    Marker(SceneRect),
}

/// What a terminal's current frame lacks that a reattach could recover (VEN-C31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageResync {
    /// The frame references a resource the budget freed.
    Freed,
    /// The frame references a resource whose allocation failed, or one the budget refused and
    /// now has room for.
    Refused,
    /// The frame references a resource the budget refused and still has no room for, which a
    /// reattach would not recover.
    NoRoom,
}

/// What the store knows of one resource.
enum Entry<T> {
    Held {
        texture: T,
        width: u32,
        height: u32,
        retired: bool,
    },
    /// Freed for the budget while no frame referenced it.
    Freed,
    /// Refused for the budget, or because its allocation failed within it (`budget` false).
    Refused { bytes: u64, budget: bool },
}

fn texture_bytes(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) * 4
}

/// The window's image resources and their budget (VEN-C31), generic over the texture so the
/// accounting and the band derivation run without a GPU.
pub(super) struct ImageStore<T> {
    entries: HashMap<ImageKey, Entry<T>>,
    /// Bytes of held textures.
    resident: u64,
    budget: u64,
    /// Attachments whose frames draw their images; any other frame shows only markers.
    live: HashSet<u64>,
    /// Attachments whose pending entries show the marker at once.
    at_once: HashSet<u64>,
    /// When each entry pending in the last derived bands was first seen pending.
    pending_since: HashMap<ImageKey, Instant>,
    /// When the next pending entry's marker comes due.
    next_marker: Option<Instant>,
}

impl<T> ImageStore<T> {
    fn new(budget: u64) -> Self {
        Self {
            entries: HashMap::new(),
            resident: 0,
            budget,
            live: HashSet::new(),
            at_once: HashSet::new(),
            pending_since: HashMap::new(),
            next_marker: None,
        }
    }

    pub(super) fn resident(&self) -> u64 {
        self.resident
    }

    pub(super) fn next_marker(&self) -> Option<Instant> {
        self.next_marker
    }

    fn texture(&self, key: ImageKey) -> Option<&T> {
        match self.entries.get(&key) {
            Some(Entry::Held { texture, .. }) => Some(texture),
            _ => None,
        }
    }

    /// Bytes of held textures that no current frame references, which the budget may free.
    fn freeable(&self, referenced: &HashSet<ImageKey>) -> u64 {
        self.entries
            .iter()
            .filter(|(key, _)| !referenced.contains(key))
            .map(|(_, entry)| match entry {
                Entry::Held { width, height, .. } => texture_bytes(*width, *height),
                _ => 0,
            })
            .sum()
    }

    /// Whether `bytes` fit the budget once every unreferenced texture is freed.
    fn fits(&self, bytes: u64, referenced: &HashSet<ImageKey>) -> bool {
        self.resident - self.freeable(referenced) + bytes <= self.budget
    }

    /// Makes room for `bytes` under `key` by freeing unreferenced textures, smallest key first;
    /// or, when even freeing all of them leaves no room, refuses `key`, frees nothing and returns
    /// false.
    fn admit(&mut self, key: ImageKey, bytes: u64, referenced: &HashSet<ImageKey>) -> bool {
        self.remove(key);
        if !self.fits(bytes, referenced) {
            self.refuse(key, bytes, true);
            return false;
        }
        let mut unreferenced = self
            .entries
            .iter()
            .filter(|(key, entry)| matches!(entry, Entry::Held { .. }) && !referenced.contains(key))
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        unreferenced.sort_unstable();
        for unreferenced in unreferenced {
            if self.resident + bytes <= self.budget {
                break;
            }
            // A retired texture is forgotten, since no frame can reference it again.
            if !self.remove(unreferenced) {
                self.entries.insert(unreferenced, Entry::Freed);
            }
        }
        true
    }

    /// Forgets `key`, returning whether it was a retired texture.
    fn remove(&mut self, key: ImageKey) -> bool {
        match self.entries.remove(&key) {
            Some(Entry::Held {
                width,
                height,
                retired,
                ..
            }) => {
                self.resident -= texture_bytes(width, height);
                retired
            }
            _ => false,
        }
    }

    fn hold(&mut self, key: ImageKey, texture: T, width: u32, height: u32) {
        self.remove(key);
        self.resident += texture_bytes(width, height);
        self.entries.insert(
            key,
            Entry::Held {
                texture,
                width,
                height,
                retired: false,
            },
        );
    }

    fn refuse(&mut self, key: ImageKey, bytes: u64, budget: bool) {
        self.remove(key);
        self.entries.insert(key, Entry::Refused { bytes, budget });
    }

    /// Orbit retired `key`: a held texture goes once no current frame references it, and
    /// anything else at once.
    pub(super) fn retire(&mut self, key: ImageKey) {
        match self.entries.get_mut(&key) {
            Some(Entry::Held { retired, .. }) => *retired = true,
            Some(_) => {
                self.entries.remove(&key);
            }
            None => {}
        }
    }

    /// Forgets every resource of an attachment that is not `live`, as on detach, `Exited` or a
    /// reconnect, and every retired texture that no current frame references (VEN-C31).
    /// Pending entries of `at_once` attachments show the marker without waiting.
    pub(super) fn sweep(
        &mut self,
        live: HashSet<u64>,
        at_once: HashSet<u64>,
        referenced: &HashSet<ImageKey>,
    ) {
        let gone = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                !live.contains(&key.attachment)
                    || matches!(entry, Entry::Held { retired: true, .. })
                        && !referenced.contains(key)
            })
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        for key in gone {
            self.remove(key);
        }
        self.live = live;
        self.at_once = at_once;
    }

    /// What a live attachment's frame, with these placements, lacks: a freed resource first,
    /// then a refused one that a reattach may now upload, then one the budget still refuses.
    pub(super) fn resync(
        &self,
        attachment: u64,
        placements: &[ScenePlacement],
        referenced: &HashSet<ImageKey>,
    ) -> Option<ImageResync> {
        let mut lacking = None;
        for placement in placements {
            let key = ImageKey {
                attachment,
                resource: placement.resource,
            };
            match self.entries.get(&key) {
                Some(Entry::Freed) => return Some(ImageResync::Freed),
                Some(Entry::Refused { bytes, budget }) => {
                    if !budget || self.fits(*bytes, referenced) {
                        lacking = Some(ImageResync::Refused);
                    } else {
                        lacking.get_or_insert(ImageResync::NoRoom);
                    }
                }
                _ => {}
            }
        }
        lacking
    }

    /// Each scene's image band in frame order (VEN-C31). An entry whose resource is held draws,
    /// whatever its state; any other shows the marker, except a pending entry seen pending for
    /// less than [`MARKER_DELAY`], which draws nothing until its marker comes due. A frame of an
    /// attachment that is not live shows the marker over every entry.
    pub(super) fn derive(
        &mut self,
        scenes: &[Option<&SceneGraphics>],
        now: Instant,
    ) -> Vec<Vec<BandItem>> {
        let mut pending_since = HashMap::new();
        let mut next_marker = None::<Instant>;
        let mut bands = Vec::with_capacity(scenes.len());
        for graphics in scenes {
            let mut band = Vec::new();
            let (attachment, placements) = graphics.map_or((0, &[][..]), |graphics| {
                (graphics.attachment, &graphics.placements)
            });
            let live = self.live.contains(&attachment);
            for placement in placements {
                let key = ImageKey {
                    attachment,
                    resource: placement.resource,
                };
                let marker = BandItem::Marker(placement.target);
                if !live {
                    band.push(marker);
                    continue;
                }
                match self.entries.get(&key) {
                    Some(Entry::Held { width, height, .. }) => {
                        band.extend(
                            held_image(key, placement, *width, *height).map(BandItem::Image),
                        );
                    }
                    None if placement.state == PlacementState::Pending => {
                        let since = *pending_since.entry(key).or_insert_with(|| {
                            self.pending_since.get(&key).copied().unwrap_or(now)
                        });
                        let due = since + MARKER_DELAY;
                        if now >= due || self.at_once.contains(&attachment) {
                            band.push(marker);
                        } else {
                            next_marker = Some(next_marker.map_or(due, |next| next.min(due)));
                        }
                    }
                    _ => band.push(marker),
                }
            }
            bands.push(band);
        }
        self.pending_since = pending_since;
        self.next_marker = next_marker;
        bands
    }
}

/// The part of a held resource an entry draws: its source clipped to the resource's bounds, and
/// its target cut by the same proportion, so the visible part keeps its scale and position. The
/// source starts at or past the resource's top-left corner, so only its right and bottom can be
/// cut; one that lies wholly outside draws nothing.
fn held_image(
    key: ImageKey,
    placement: &ScenePlacement,
    width: u32,
    height: u32,
) -> Option<SceneImage> {
    let source = placement.source;
    let visible = |start: u32, length: u32, bound: u32| {
        (u64::from(start) + u64::from(length))
            .min(u64::from(bound))
            .checked_sub(u64::from(start))
            .filter(|visible| *visible > 0)
            .map(|visible| visible as f32)
    };
    let columns = visible(source.x, source.width, width)?;
    let rows = visible(source.y, source.height, height)?;
    Some(SceneImage {
        texture: key,
        source: SceneRect {
            left: source.x as f32 / width as f32,
            top: source.y as f32 / height as f32,
            width: columns / width as f32,
            height: rows / height as f32,
        },
        target: SceneRect {
            width: placement.target.width * columns / source.width as f32,
            height: placement.target.height * rows / source.height as f32,
            ..placement.target
        },
    })
}

fn channels(format: PixelFormat) -> usize {
    match format {
        PixelFormat::Rgb => 3,
        PixelFormat::Rgba => 4,
    }
}

/// Writes rows of `width` pixels in `format` into `destination` as RGBA rows `stride` bytes
/// apart, giving RGB opaque alpha. The upload writes the assembly straight into the staging
/// mapping this way, so the staging holds the only RGBA copy (VEN-C31).
fn write_rgba_rows(
    mut destination: WriteOnly<'_, [u8]>,
    stride: usize,
    pixels: &[u8],
    format: PixelFormat,
    width: u32,
) {
    let row = width as usize * 4;
    for (index, source) in pixels
        .chunks_exact(width as usize * channels(format))
        .enumerate()
    {
        let target = destination.slice(index * stride..index * stride + row);
        match format {
            PixelFormat::Rgba => target.write_iter(source.iter().copied()),
            PixelFormat::Rgb => target.write_iter(
                source
                    .chunks_exact(3)
                    .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], u8::MAX]),
            ),
        }
    }
}

const IMAGE_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@vertex
fn vertex(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = uv;
    return output;
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, input.uv);
}
"#;

/// One image quad set, drawn after the first `after` rectangle vertices.
pub(super) struct ImageDraw {
    after: u32,
    texture: ImageKey,
    vertices: Range<u32>,
}

/// One step of a frame's draw sequence.
#[derive(Debug, PartialEq)]
pub(super) enum DrawStep {
    Rectangles(Range<u32>),
    /// The trailing cursor's own vertices.
    Cursor,
    Image(usize),
}

/// Interleaves the static rectangles with the image draws and the trailing cursor, which draws
/// after the vertex `cursor` and after any image at that vertex.
pub(super) fn draw_steps(
    total: u32,
    mut cursor: Option<u32>,
    images: &[ImageDraw],
) -> Vec<DrawStep> {
    let mut steps = Vec::new();
    let mut drawn = 0;
    let mut flush = |steps: &mut Vec<DrawStep>, to: u32| {
        let to = to.min(total);
        if to > drawn {
            steps.push(DrawStep::Rectangles(drawn..to));
            drawn = to;
        }
    };
    for (index, image) in images.iter().enumerate() {
        if let Some(at) = cursor.take_if(|at| *at < image.after) {
            flush(&mut steps, at);
            steps.push(DrawStep::Cursor);
        }
        flush(&mut steps, image.after);
        steps.push(DrawStep::Image(index));
    }
    if let Some(at) = cursor {
        flush(&mut steps, at);
        steps.push(DrawStep::Cursor);
    }
    flush(&mut steps, total);
    steps
}

impl RectangleBatch {
    /// Adds a terminal's image band at this point of the stream, images and markers in order,
    /// under the batch's clips, with each target measured in cells from the grid corner (`left`,
    /// `top`). Markers take the terminal's `foreground` color.
    pub(super) fn push_band(
        &mut self,
        band: &[BandItem],
        left: f32,
        top: f32,
        metrics: CellMetrics,
        foreground: SceneColor,
    ) {
        let to_pixels = |target: SceneRect| SceneRect {
            left: left + target.left * metrics.width,
            top: top + target.top * metrics.height,
            width: target.width * metrics.width,
            height: target.height * metrics.height,
        };
        for item in band {
            match item {
                BandItem::Image(image) => self.push_image(image, to_pixels(image.target)),
                BandItem::Marker(target) => {
                    self.push_marker(to_pixels(*target), metrics.scale, foreground);
                }
            }
        }
    }

    fn push_image(&mut self, image: &SceneImage, target: SceneRect) {
        if !valid_rect(target) || !valid_rect(image.source) {
            return;
        }
        let Some(visible) = self
            .clip
            .map_or(Some(target), |clip| target.intersection(clip))
        else {
            return;
        };
        let pieces = match self.card_clip {
            Some(card) if visible.intersection(card.middle()) != Some(visible) => card
                .bands()
                .filter_map(|band| band.intersection(visible))
                .collect(),
            _ => vec![visible],
        };
        let start = image_vertex_count(&self.image_bytes);
        for piece in pieces {
            self.write_image_piece(piece, target, image.source);
        }
        let end = image_vertex_count(&self.image_bytes);
        if end > start {
            self.images.push(ImageDraw {
                after: vertex_count(&self.bytes),
                texture: image.texture,
                vertices: start..end,
            });
        }
    }

    /// The missing-image marker: a faint fill inside a one-logical-pixel outline (VEN-C31).
    fn push_marker(&mut self, target: SceneRect, scale: f32, color: SceneColor) {
        if !valid_rect(target) {
            return;
        }
        let stroke = scale.max(1.0);
        let inner = SceneRect {
            left: target.left + stroke,
            top: target.top + stroke,
            width: target.width - stroke * 2.0,
            height: target.height - stroke * 2.0,
        };
        if !valid_rect(inner) {
            let SceneRect {
                left,
                top,
                width,
                height,
            } = target;
            self.push(left, top, width, height, color, MARKER_OUTLINE_ALPHA);
            return;
        }
        self.push(
            inner.left,
            inner.top,
            inner.width,
            inner.height,
            color,
            MARKER_FILL_ALPHA,
        );
        for edge in [
            SceneRect {
                height: stroke,
                ..target
            },
            SceneRect {
                top: inner.bottom(),
                height: stroke,
                ..target
            },
            SceneRect {
                top: inner.top,
                width: stroke,
                height: inner.height,
                ..target
            },
            SceneRect {
                left: inner.right(),
                top: inner.top,
                width: stroke,
                height: inner.height,
            },
        ] {
            self.push(
                edge.left,
                edge.top,
                edge.width,
                edge.height,
                color,
                MARKER_OUTLINE_ALPHA,
            );
        }
    }

    /// Two triangles covering `piece`, sampling the part of `source` that `piece` covers of
    /// `target`.
    fn write_image_piece(&mut self, piece: SceneRect, target: SceneRect, source: SceneRect) {
        let to_x = |value: f32| value / self.width as f32 * 2.0 - 1.0;
        let to_y = |value: f32| 1.0 - value / self.height as f32 * 2.0;
        let u = |x: f32| source.left + (x - target.left) / target.width * source.width;
        let v = |y: f32| source.top + (y - target.top) / target.height * source.height;
        let (left, right, top, bottom) = (piece.left, piece.right(), piece.top, piece.bottom());
        for (x, y) in [
            (left, top),
            (left, bottom),
            (right, bottom),
            (left, top),
            (right, bottom),
            (right, top),
        ] {
            for value in [to_x(x), to_y(y), u(x), v(y)] {
                self.image_bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
    }
}

fn image_vertex_count(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len() / IMAGE_VERTEX_SIZE as usize)
        .expect("bounded draw inputs fit the vertex count")
}

/// The image pipeline, the resource store with its textures, and this frame's image quads.
pub(super) struct Images {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: wgpu::Sampler,
    format: TextureFormat,
    pub(super) store: ImageStore<BindGroup>,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    pub(super) draws: Vec<ImageDraw>,
}

impl Images {
    pub(super) fn new(device: &wgpu::Device, target: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("venus images"),
            source: ShaderSource::Wgsl(IMAGE_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("venus image bind group layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("venus image pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("venus image pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: IMAGE_VERTEX_SIZE,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
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
                entry_point: Some("fragment"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: target,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("venus image sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let vertices = device.create_buffer(&BufferDescriptor {
            label: Some("venus image vertices"),
            size: IMAGE_VERTEX_SIZE,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            // Like glyphon's color atlas: sRGB targets sample linear values, others pass through.
            format: if target.is_srgb() {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            },
            store: ImageStore::new(IMAGE_BUDGET_BYTES),
            vertices,
            vertex_capacity: IMAGE_VERTEX_SIZE,
            draws: Vec::new(),
        }
    }

    /// Uploads a completely received resource, or refuses it so its entries show the marker
    /// (VEN-C31). The budget refuses a resource that does not fit even after freeing every
    /// texture outside `referenced`, and frees nothing then; a side past the device's limit or an
    /// allocation error the device reports refuses it too. Returns once the GPU has finished the
    /// copy and released its staging.
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resource: ImageResource,
        referenced: &HashSet<ImageKey>,
    ) -> Result<(), RenderError> {
        let key = ImageKey {
            attachment: resource.attachment,
            resource: resource.resource,
        };
        let (width, height) = (resource.width, resource.height);
        let bytes = texture_bytes(width, height);
        let limit = device.limits().max_texture_dimension_2d;
        if !(1..=limit).contains(&width) || !(1..=limit).contains(&height) {
            self.store.refuse(key, bytes, false);
            return Err(RenderError(format!(
                "a {width} by {height} image does not fit the GPU's {limit}-pixel texture side"
            )));
        }
        let expected = u64::from(width) * u64::from(height) * channels(resource.format) as u64;
        if resource.pixels.len() as u64 != expected {
            self.store.refuse(key, bytes, false);
            return Err(RenderError(format!(
                "a {width} by {height} image cannot have {} bytes",
                resource.pixels.len()
            )));
        }
        if !self.store.admit(key, bytes, referenced) {
            return Err(RenderError(format!(
                "a {width} by {height} image does not fit the {} MiB image budget",
                IMAGE_BUDGET_BYTES >> 20
            )));
        }
        // Textures freed here or by a sweep are dropped, but a frame in flight may still hold
        // them; wait for the GPU to release them before allocating, so the budget is never
        // exceeded.
        let _ = device.poll(PollType::wait_indefinitely());
        match self.allocate(device, queue, resource) {
            Ok(texture) => {
                self.store.hold(key, texture, width, height);
                Ok(())
            }
            Err(error) => {
                self.store.refuse(key, bytes, false);
                Err(error)
            }
        }
    }

    /// Creates and fills a texture inside error scopes, so a failure is a refusal rather than an
    /// uncaptured error, and waits until the GPU has finished the copy and released its staging.
    /// The resource's assembly goes as soon as its rows are in the staging buffer, so at most
    /// the assembly and one staging copy coexist (VEN-C31).
    fn allocate(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resource: ImageResource,
    ) -> Result<BindGroup, RenderError> {
        let (width, height) = (resource.width, resource.height);
        let refused = |error| RenderError(format!("the GPU refused an image: {error}"));
        let memory = device.push_error_scope(ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(ErrorFilter::Validation);
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("venus image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let stride = width
            .saturating_mul(4)
            .next_multiple_of(COPY_BYTES_PER_ROW_ALIGNMENT);
        // A mappable buffer maps itself at creation; without `MAP_WRITE`, wgpu would stage the
        // mapping in a second allocation of the same size.
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("venus image staging"),
            size: u64::from(stride) * u64::from(height),
            usage: BufferUsages::MAP_WRITE | BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        // A buffer or texture the device refused is never mapped or copied.
        let validation = pollster::block_on(validation.pop());
        let memory = pollster::block_on(memory.pop());
        if let Some(error) = validation.or(memory) {
            return Err(refused(error));
        }
        let mut mapping = staging
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|error| RenderError(format!("cannot stage an image: {error}")))?;
        write_rgba_rows(
            mapping.slice(..),
            stride as usize,
            &resource.pixels,
            resource.format,
            width,
        );
        drop(resource);
        drop(mapping);
        staging.unmap();
        let memory = device.push_error_scope(ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("venus image upload"),
        });
        encoder.copy_buffer_to_texture(
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            size,
        );
        let submission = queue.submit([encoder.finish()]);
        drop(staging);
        let copied = device.poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("venus image bind group"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let validation = pollster::block_on(validation.pop());
        let memory = pollster::block_on(memory.pop());
        match (validation.or(memory), copied) {
            (Some(error), _) => Err(refused(error)),
            (None, Err(error)) => Err(RenderError(format!(
                "the GPU did not finish an image copy: {error}"
            ))),
            (None, Ok(_)) => Ok(bind_group),
        }
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        draws: Vec<ImageDraw>,
    ) {
        upload_vertices(
            device,
            queue,
            &mut self.vertices,
            &mut self.vertex_capacity,
            bytes,
            "venus image vertices",
        );
        self.draws = draws;
    }

    /// Draws one of this frame's images; a key with no texture draws nothing.
    pub(super) fn draw(&self, pass: &mut RenderPass<'_>, index: usize) {
        let draw = &self.draws[index];
        let Some(texture) = self.store.texture(draw.texture) else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, texture, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(draw.vertices.clone(), 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::cells::{build_scene_rectangles, tests::plain_style};
    use super::*;
    use crate::{DrawCell, DrawCursor, DrawRow, Scene, ScenePreview, scene::CardFill};
    use orbit_protocol::{
        CellWidth, CursorShape, Screen, SourceRect, Underline, session::VerticalDirection,
    };

    const RED: SceneColor = SceneColor { r: 200, g: 0, b: 0 };

    fn row(text: &str, background: Option<SceneColor>) -> DrawRow {
        DrawRow {
            wrapped: false,
            wrap_continuation: false,
            cells: text
                .chars()
                .map(|character| {
                    let mut style = plain_style();
                    if let Some(color) = background {
                        style.background = color;
                        style.background_is_default = false;
                    }
                    style.underline = Underline::Single;
                    DrawCell {
                        width: CellWidth::Narrow,
                        text: character.into(),
                        hyperlink: String::new(),
                        style,
                    }
                })
                .collect(),
        }
    }

    fn scene(content: Vec<DrawRow>) -> Scene {
        Scene {
            revision: 1,
            columns: u16::try_from(content[0].cells.len()).unwrap(),
            rows: u16::try_from(content.len()).unwrap(),
            screen: Screen::Primary,
            title: String::new(),
            working_directory: String::new(),
            background: SceneColor::default(),
            foreground: SceneColor::default(),
            cursor: Some(DrawCursor {
                visible: true,
                blinking: false,
                password_input: false,
                shape: CursorShape::Block,
                column: 0,
                row: 0,
                at_wide_tail: false,
                color: SceneColor::default(),
                explicit_color: false,
            }),
            content,
            graphics: SceneGraphics::default(),
        }
    }

    fn key(resource: u64) -> ImageKey {
        ImageKey {
            attachment: 1,
            resource,
        }
    }

    fn whole(resource: u64, target: SceneRect) -> SceneImage {
        SceneImage {
            texture: key(resource),
            source: SceneRect {
                left: 0.0,
                top: 0.0,
                width: 1.0,
                height: 1.0,
            },
            target,
        }
    }

    /// Each image vertex as window pixels and texture coordinates.
    fn image_vertices(batch: &RectangleBatch) -> Vec<[f32; 4]> {
        batch
            .image_bytes
            .chunks_exact(IMAGE_VERTEX_SIZE as usize)
            .map(|vertex| {
                let value = |index: usize| {
                    f32::from_ne_bytes(vertex[index * 4..index * 4 + 4].try_into().unwrap())
                };
                [
                    (value(0) + 1.0) / 2.0 * batch.width as f32,
                    (1.0 - value(1)) / 2.0 * batch.height as f32,
                    value(2),
                    value(3),
                ]
            })
            .collect()
    }

    #[test]
    fn images_draw_above_every_cell_background_and_below_foregrounds_and_cursor() {
        let metrics = CellMetrics::for_scale(1.0);
        let origin = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 400.0,
            height: 300.0,
        };
        let preview = ScenePreview::Viewport {
            frame_revision: 1,
            direction: VerticalDirection::Down,
            edge_reached: false,
            rows: vec![row("ab", Some(RED))],
        };
        let scene = scene(vec![row("ab", Some(RED)), row("cd", None)]);
        let marker = SceneRect {
            left: 0.0,
            top: 1.0,
            width: 1.0,
            height: 1.0,
        };
        let band = [
            BandItem::Image(whole(
                1,
                SceneRect {
                    left: 0.0,
                    top: 0.0,
                    width: 2.0,
                    height: 3.0,
                },
            )),
            BandItem::Marker(marker),
            BandItem::Image(whole(
                2,
                SceneRect {
                    left: 1.0,
                    top: 1.0,
                    width: 1.0,
                    height: 1.0,
                },
            )),
        ];
        let mut batch = RectangleBatch::new(400, 300);
        let cursor = build_scene_rectangles(
            &mut batch,
            &scene,
            &band,
            Some(&preview),
            true,
            metrics,
            origin,
            true,
        );

        // The live and the preview row backgrounds come first, then the band in frame order: an
        // image, the marker's fill and outline in the terminal's foreground, and an image.
        let mut backgrounds = RectangleBatch::new(400, 300);
        for top in [0.0, 2.0] {
            backgrounds.push(
                metrics.padding,
                metrics.padding + top * metrics.height,
                metrics.width * 2.0,
                metrics.height,
                RED,
                1.0,
            );
        }
        let after = vertex_count(&backgrounds.bytes);
        let mut band_rectangles = RectangleBatch::new(400, 300);
        band_rectangles.push_marker(
            SceneRect {
                left: metrics.padding,
                top: metrics.padding + metrics.height,
                width: metrics.width,
                height: metrics.height,
            },
            metrics.scale,
            scene.foreground,
        );
        let marked = after + vertex_count(&band_rectangles.bytes);
        backgrounds.bytes.extend(band_rectangles.bytes);
        assert_eq!(marked - after, 5 * 6, "a fill and four edges");
        assert_eq!(
            batch.bytes[..backgrounds.bytes.len()],
            backgrounds.bytes[..]
        );
        assert_eq!(
            batch
                .images
                .iter()
                .map(|draw| (draw.after, draw.texture))
                .collect::<Vec<_>>(),
            [(after, key(1)), (marked, key(2))]
        );
        // Underlines of both live rows and the cursor follow; preview foregrounds come last.
        let underline = 6;
        assert_eq!(cursor, marked + 4 * underline + 6);
        assert_eq!(vertex_count(&batch.bytes), cursor + 2 * underline);
        // The first image reaches into the preview row but stops at the live rows.
        let live_bottom = metrics.padding + 2.0 * metrics.height;
        assert!(
            image_vertices(&batch)
                .iter()
                .all(|[_, y, ..]| live_bottom - y > -0.001)
        );

        // A terminal clip that misses the grid, as when padding is wider than the terminal,
        // leaves no image or marker in that padding, even one placed to reach into it.
        let reaching = SceneRect {
            left: -1.0,
            top: 0.0,
            width: 2.0,
            height: 1.0,
        };
        let mut batch = RectangleBatch::new(400, 300);
        batch.clip = Some(SceneRect {
            left: 2.0,
            top: 0.0,
            width: 8.0,
            height: 300.0,
        });
        let mut no_band = RectangleBatch::new(400, 300);
        no_band.clip = batch.clip;
        build_scene_rectangles(&mut no_band, &scene, &[], None, true, metrics, origin, true);
        build_scene_rectangles(
            &mut batch,
            &scene,
            &[
                BandItem::Image(whole(1, reaching)),
                BandItem::Marker(reaching),
            ],
            None,
            true,
            metrics,
            origin,
            true,
        );
        assert!(batch.images.is_empty() && batch.image_bytes.is_empty());
        assert_eq!(batch.bytes, no_band.bytes);
    }

    #[test]
    fn images_clip_to_the_grid_and_rounded_card_and_keep_their_texture_mapping() {
        let metrics = CellMetrics::for_scale(1.0);
        let grid = SceneRect {
            left: 20.0,
            top: 10.0,
            width: 100.0,
            height: 60.0,
        };
        let card = CardFill {
            bounds: grid,
            radius: 8.0,
        };
        let source = SceneRect {
            left: 0.5,
            top: 0.0,
            width: 0.5,
            height: 0.5,
        };
        // Starts a cell left of the grid and runs past its right and bottom edges.
        let image = SceneImage {
            texture: key(3),
            source,
            target: SceneRect {
                left: -1.0,
                top: 0.5,
                width: 20.0,
                height: 10.0,
            },
        };
        let target = SceneRect {
            left: grid.left - metrics.width,
            top: grid.top + metrics.height * 0.5,
            width: metrics.width * 20.0,
            height: metrics.height * 10.0,
        };
        for card_clip in [None, Some(card)] {
            let mut batch = RectangleBatch::new(200, 100);
            batch.clip = Some(grid);
            batch.card_clip = card_clip;
            batch.push_band(&[BandItem::Image(image)], grid.left, grid.top, metrics, RED);

            let vertices = image_vertices(&batch);
            assert_eq!(batch.images.len(), 1);
            assert_eq!(
                batch.images[0].vertices,
                0..u32::try_from(vertices.len()).unwrap()
            );
            for [x, y, u, v] in &vertices {
                assert!(x - grid.left > -0.001 && grid.right() - x > -0.001, "x {x}");
                assert!(y - grid.top > -0.001 && grid.bottom() - y > -0.001, "y {y}");
                let expected_u = source.left + (x - target.left) / target.width * source.width;
                let expected_v = source.top + (y - target.top) / target.height * source.height;
                assert!((u - expected_u).abs() < 1e-4 && (v - expected_v).abs() < 1e-4);
            }
            let pieces = vertices
                .chunks_exact(6)
                .map(|piece| SceneRect {
                    left: piece[0][0],
                    top: piece[0][1],
                    width: piece[2][0] - piece[0][0],
                    height: piece[2][1] - piece[0][1],
                })
                .collect::<Vec<_>>();
            if card_clip.is_some() {
                // The bottom corners are cut, so no piece reaches the grid's bottom-left corner.
                assert!(pieces.len() > 1);
                assert!(pieces.iter().all(|piece| {
                    card.bands().any(|band| {
                        piece.left - band.left > -0.001
                            && piece.top - band.top > -0.001
                            && band.right() - piece.right() > -0.001
                            && band.bottom() - piece.bottom() > -0.001
                    })
                }));
                assert!(
                    !pieces
                        .iter()
                        .any(|piece| piece.contains(grid.left + 0.5, grid.bottom() - 0.5))
                );
            } else {
                let [piece] = pieces[..] else {
                    panic!("one piece expected: {pieces:?}");
                };
                for (actual, expected) in [
                    (piece.left, grid.left),
                    (piece.top, target.top),
                    (piece.right(), grid.right()),
                    (piece.bottom(), grid.bottom()),
                ] {
                    assert!((actual - expected).abs() < 0.001, "{piece:?}");
                }
            }
        }

        // Nothing visible, an empty source or a non-finite target draws nothing.
        let mut batch = RectangleBatch::new(200, 100);
        batch.clip = Some(grid);
        for image in [
            SceneImage {
                target: SceneRect {
                    left: 50.0,
                    ..image.target
                },
                ..image
            },
            SceneImage {
                source: SceneRect {
                    width: 0.0,
                    ..source
                },
                ..image
            },
            SceneImage {
                target: SceneRect {
                    width: f32::NAN,
                    ..image.target
                },
                ..image
            },
        ] {
            batch.push_band(&[BandItem::Image(image)], grid.left, grid.top, metrics, RED);
        }
        assert!(batch.images.is_empty() && batch.image_bytes.is_empty());
    }

    /// Each entry's kind, by resource of attachment 1.
    fn entries(store: &ImageStore<()>, resources: Range<u64>) -> Vec<Option<&'static str>> {
        resources
            .map(|resource| {
                store.entries.get(&key(resource)).map(|entry| match entry {
                    Entry::Held { retired: false, .. } => "held",
                    Entry::Held { retired: true, .. } => "retired",
                    Entry::Freed => "freed",
                    Entry::Refused { budget: true, .. } => "refused",
                    Entry::Refused { budget: false, .. } => "failed",
                })
            })
            .collect()
    }

    fn keys(resources: &[u64]) -> HashSet<ImageKey> {
        resources.iter().map(|resource| key(*resource)).collect()
    }

    /// Admits and holds a `width` by `height` resource, as an upload whose allocation succeeds.
    fn upload(
        store: &mut ImageStore<()>,
        resource: u64,
        (width, height): (u32, u32),
        referenced: &[u64],
    ) -> bool {
        let admitted = store.admit(
            key(resource),
            texture_bytes(width, height),
            &keys(referenced),
        );
        if admitted {
            store.hold(key(resource), (), width, height);
        }
        admitted
    }

    #[test]
    fn the_budget_frees_only_unreferenced_textures_and_refuses_without_freeing_any() {
        // 64 bytes hold four 2 by 2 textures.
        let mut store = ImageStore::<()>::new(64);
        assert!(upload(&mut store, 1, (2, 2), &[]));
        assert!(upload(&mut store, 2, (2, 2), &[]));
        assert!(upload(&mut store, 3, (2, 4), &[1]));
        assert_eq!(store.resident(), 64);
        // Room comes from unreferenced textures, smallest key first, and only as much as needed.
        assert!(upload(&mut store, 4, (2, 2), &[3]));
        assert_eq!(
            entries(&store, 1..5),
            [Some("freed"), Some("held"), Some("held"), Some("held")]
        );
        // When even freeing every unreferenced texture leaves no room, nothing is freed.
        assert!(!upload(&mut store, 5, (2, 4), &[3, 4]));
        assert_eq!(
            entries(&store, 1..6),
            [
                Some("freed"),
                Some("held"),
                Some("held"),
                Some("held"),
                Some("refused")
            ]
        );
        assert_eq!(store.resident(), 64);
        // A retired texture freed for room is forgotten, since no frame can reference it again.
        store.retire(key(2));
        store.retire(key(5));
        assert!(upload(&mut store, 6, (2, 2), &[3, 4]));
        assert_eq!(
            entries(&store, 1..7),
            [
                Some("freed"),
                None,
                Some("held"),
                Some("held"),
                None,
                Some("held")
            ]
        );
        // A retired texture stays while a current frame references it; a dead attachment's
        // resources all go.
        store.retire(key(4));
        store.sweep(HashSet::from([1]), HashSet::new(), &keys(&[4]));
        assert_eq!(
            entries(&store, 3..7),
            [Some("held"), Some("retired"), None, Some("held")]
        );
        store.sweep(HashSet::from([1]), HashSet::new(), &keys(&[]));
        assert_eq!(
            entries(&store, 3..7),
            [Some("held"), None, None, Some("held")]
        );
        assert_eq!(store.resident(), 48);
        store.sweep(HashSet::from([2]), HashSet::new(), &keys(&[]));
        assert!(store.entries.is_empty());
        assert_eq!(store.resident(), 0);
        // An older texture with a smaller key that a frame references is skipped for a newer one
        // that none does.
        assert!(upload(&mut store, 7, (2, 4), &[]));
        assert!(upload(&mut store, 8, (2, 4), &[]));
        assert!(upload(&mut store, 9, (2, 2), &[7]));
        assert_eq!(
            entries(&store, 7..10),
            [Some("held"), Some("freed"), Some("held")]
        );
    }

    #[test]
    fn frames_resync_for_freed_resources_and_refusals_a_reattach_may_recover() {
        let mut store = ImageStore::<()>::new(64);
        let placements = |resources: &[u64]| {
            resources
                .iter()
                .map(|resource| ScenePlacement {
                    resource: *resource,
                    state: PlacementState::Ready,
                    source: SourceRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    target: SceneRect::default(),
                })
                .collect::<Vec<_>>()
        };
        assert!(upload(&mut store, 1, (2, 4), &[]));
        assert!(upload(&mut store, 2, (4, 4), &[]));
        assert!(!upload(&mut store, 3, (2, 2), &[2]));
        store.refuse(key(4), 16, false);
        assert_eq!(
            entries(&store, 1..5),
            [Some("freed"), Some("held"), Some("refused"), Some("failed")]
        );
        let resync = |resources: &[u64], referenced: &[u64]| {
            store.resync(1, &placements(resources), &keys(referenced))
        };
        assert_eq!(resync(&[2], &[2]), None);
        assert_eq!(resync(&[3, 1], &[1, 3]), Some(ImageResync::Freed));
        // A budget refusal resyncs only once the budget has room for it.
        assert_eq!(resync(&[3], &[2, 3]), Some(ImageResync::NoRoom));
        assert_eq!(resync(&[3], &[3]), Some(ImageResync::Refused));
        // A failed allocation may succeed on another attempt.
        assert_eq!(resync(&[3, 4], &[2, 3, 4]), Some(ImageResync::Refused));
    }

    #[test]
    fn bands_draw_held_images_clipped_and_mark_every_missing_one_in_frame_order() {
        let mut store = ImageStore::<()>::new(IMAGE_BUDGET_BYTES);
        store.hold(key(1), (), 10, 10);
        store.refuse(key(3), 4, true);
        store.entries.insert(key(4), Entry::Freed);
        store.sweep(HashSet::from([1]), HashSet::new(), &keys(&[1]));
        let target = SceneRect {
            left: 1.0,
            top: 2.0,
            width: 4.0,
            height: 2.0,
        };
        let placement = |resource, state, x| ScenePlacement {
            resource,
            state,
            source: SourceRect {
                x,
                y: 0,
                width: 10,
                height: 10,
            },
            target,
        };
        use PlacementState::{Pending, Ready, Unavailable};
        let graphics = SceneGraphics {
            attachment: 1,
            placements: vec![
                // Held, whatever the entry's state, with a source running half past the right
                // edge: the visible half draws at its scale, over the target's left half.
                placement(1, Pending, 5),
                placement(2, Unavailable, 0),
                placement(3, Ready, 0),
                placement(4, Ready, 0),
                // Ready, yet never received.
                placement(5, Ready, 0),
                placement(6, Pending, 0),
                // A source wholly outside the resource draws nothing.
                placement(1, Ready, 10),
            ],
            placements_omitted: false,
        };
        // A scene of an attachment that is not live, as one kept through a reconnect.
        let kept = SceneGraphics {
            attachment: 2,
            placements: vec![placement(1, Ready, 0)],
            placements_omitted: false,
        };
        let image = BandItem::Image(SceneImage {
            texture: key(1),
            source: SceneRect {
                left: 0.5,
                top: 0.0,
                width: 0.5,
                height: 1.0,
            },
            target: SceneRect {
                width: 2.0,
                ..target
            },
        });
        let marker = BandItem::Marker(target);
        let start = Instant::now();
        assert_eq!(
            store.derive(&[Some(&graphics), None, Some(&kept)], start),
            [
                vec![image, marker, marker, marker, marker],
                vec![],
                vec![marker]
            ]
        );
        assert_eq!(store.next_marker(), Some(start + MARKER_DELAY));
        let early = start + MARKER_DELAY - Duration::from_millis(1);
        assert_eq!(store.derive(&[Some(&graphics)], early)[0].len(), 5);
        assert_eq!(
            store.derive(&[Some(&graphics)], start + MARKER_DELAY),
            [vec![image, marker, marker, marker, marker, marker]]
        );
        assert_eq!(store.next_marker(), None);

        // After a resync, the new attachment's pending entries show the marker at once.
        let fresh = SceneGraphics {
            attachment: 1,
            placements: vec![placement(7, Pending, 0)],
            placements_omitted: false,
        };
        store.sweep(HashSet::from([1]), HashSet::from([1]), &keys(&[1]));
        assert_eq!(store.derive(&[Some(&fresh)], start), [vec![marker]]);
    }

    #[test]
    fn rows_reach_the_padded_staging_as_opaque_rgba() {
        // Two rows of two pixels, each staged row padded to 12 bytes.
        let staged = |format, pixels: &[u8]| {
            let mut destination = vec![0; 24];
            write_rgba_rows(
                WriteOnly::from_mut(destination.as_mut_slice()),
                12,
                pixels,
                format,
                2,
            );
            destination
        };
        assert_eq!(
            staged(PixelFormat::Rgb, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]),
            [
                1, 2, 3, 255, 4, 5, 6, 255, 0, 0, 0, 0, 7, 8, 9, 255, 10, 11, 12, 255, 0, 0, 0, 0
            ]
        );
        assert_eq!(
            staged(PixelFormat::Rgba, &(1..=16).collect::<Vec<_>>()),
            [
                1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn draw_steps_put_images_under_later_rectangles_and_the_trailing_cursor() {
        let image = |after| ImageDraw {
            after,
            texture: key(0),
            vertices: 0..6,
        };
        use DrawStep::{Cursor, Image, Rectangles};
        assert_eq!(draw_steps(18, None, &[]), [Rectangles(0..18)]);
        assert_eq!(
            draw_steps(18, Some(12), &[]),
            [Rectangles(0..12), Cursor, Rectangles(12..18)]
        );
        assert_eq!(draw_steps(18, Some(24), &[]), [Rectangles(0..18), Cursor]);
        assert_eq!(
            draw_steps(18, Some(12), &[image(0), image(6), image(12)]),
            [
                Image(0),
                Rectangles(0..6),
                Image(1),
                Rectangles(6..12),
                Image(2),
                Cursor,
                Rectangles(12..18),
            ]
        );
        assert_eq!(
            draw_steps(18, Some(6), &[image(12)]),
            [
                Rectangles(0..6),
                Cursor,
                Rectangles(6..12),
                Image(0),
                Rectangles(12..18),
            ]
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "requires an isolated native Wayland display and Vulkan renderer"]
    fn images_draw_in_panes_and_popups_and_refusals_keep_the_renderer() {
        use super::super::{FontSettings, FontSetup, PaddingColor, PresentOutcome, Renderer};
        use crate::{WorkspaceFocus, WorkspaceScene};
        use eon_workspace_protocol::v26::{
            ALT, Column, Pane, Popup, PopupEntry, PopupGeometry, PopupKey, Shortcut, Sidebar,
            Snapshot, Tab, WorkspaceInfo,
        };
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        use winit::{
            application::ApplicationHandler,
            dpi::PhysicalSize,
            event::WindowEvent,
            event_loop::{ActiveEventLoop, EventLoop},
            platform::wayland::EventLoopBuilderExtWayland,
            window::{Window, WindowId},
        };

        /// The pane, the popup, and the pane under a notice each stay up long enough for an
        /// outside capture.
        const PHASE: Duration = Duration::from_millis(2500);

        /// A 64-pixel checkerboard whose right half is half transparent.
        fn checkerboard() -> Vec<u8> {
            (0..64_u32)
                .flat_map(|y| {
                    (0..64_u32).flat_map(move |x| {
                        let alpha = if x < 32 { 255 } else { 128 };
                        if (x / 8 + y / 8) % 2 == 0 {
                            [30, 90, 230, alpha]
                        } else {
                            [240, 220, 40, alpha]
                        }
                    })
                })
                .collect()
        }

        fn snapshot(popup: bool) -> Snapshot {
            Snapshot {
                workspace: "w1".into(),
                active_workspace: "w1".into(),
                workspaces: vec![WorkspaceInfo {
                    id: "w1".into(),
                    name: None,
                    tab_count: 1,
                    attention: None,
                    recency: 0,
                }],
                active_tab: "t1".into(),
                tabs: vec![Tab {
                    id: "t1".into(),
                    directory: b"/tmp".to_vec(),
                    pending: false,
                    selected_pane: Some("p1".into()),
                    selected_popup: popup.then(|| "u1".into()),
                    columns: vec![Column {
                        panes: vec![Pane {
                            label: "p1".into(),
                            pinned_name: None,
                            id: "p1".into(),
                            session: "s1".into(),
                            endpoint: b"/tmp/s1.sock".to_vec(),
                            live: true,
                            attention: None,
                        }],
                        expanded: "p1".into(),
                    }],
                    popups: vec![Popup {
                        id: "u1".into(),
                        entry: "files".into(),
                        session: "files".into(),
                        endpoint: b"/tmp/files.sock".to_vec(),
                        attention: None,
                    }],
                    attention_rollup: None,
                }],
                geometry: PopupGeometry {
                    side_margin: 8.0,
                    vertical_margin: 4.0,
                },
                entries: vec![PopupEntry {
                    id: "files".into(),
                    label: "Files".into(),
                    shortcut: PopupKey::Bound(Shortcut {
                        modifiers: ALT,
                        key: "KeyF".into(),
                    }),
                }],
                codex_quota: None,
                claude_quota: None,
                sidebar: Sidebar::Open,
                attention_queue: Vec::new(),
            }
        }

        #[derive(Default)]
        struct Probe {
            renderer: Option<Renderer>,
            start: Option<Instant>,
            presented: [u32; 3],
            done: bool,
        }
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                let window = Arc::new(
                    event_loop
                        .create_window(Window::default_attributes())
                        .unwrap(),
                );
                let mut renderer = pollster::block_on(Renderer::new(
                    Arc::clone(&window),
                    event_loop,
                    1.0,
                    PaddingColor::Extend,
                    None,
                    true,
                    SceneColor {
                        r: 178,
                        g: 7,
                        b: 251,
                    },
                    1,
                    2,
                    true,
                    FontSetup::new(&FontSettings::default()).unwrap(),
                ))
                .unwrap();
                let resource = |resource, width, height, pixels| ImageResource {
                    attachment: 1,
                    resource,
                    format: PixelFormat::Rgba,
                    width,
                    height,
                    pixels,
                };
                let none = HashSet::new();
                let limit = renderer.device.limits().max_texture_dimension_2d;
                assert!(
                    renderer
                        .upload_image(resource(1, limit + 1, 1, Vec::new()), &none)
                        .is_err()
                );
                assert!(
                    renderer
                        .upload_image(resource(2, 2, 2, vec![0; 15]), &none)
                        .is_err()
                );
                // Past the limit check, the error scope turns the device's error into a refusal
                // instead of the uncaptured-error panic.
                let oversize = resource(4, limit + 1, 1, vec![0; (limit as usize + 1) * 4]);
                assert!(
                    renderer
                        .images
                        .allocate(&renderer.device, &renderer.queue, oversize)
                        .is_err()
                );
                renderer
                    .upload_image(resource(3, 64, 64, checkerboard()), &none)
                    .unwrap();
                renderer.sweep_images(HashSet::from([1]), HashSet::new(), &none);
                self.renderer = Some(renderer);
                self.start = Some(Instant::now());
                window.request_redraw();
            }

            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                _: WindowId,
                event: WindowEvent,
            ) {
                if !matches!(event, WindowEvent::RedrawRequested) {
                    return;
                }
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                let phase = self.start.unwrap().elapsed().as_millis() / PHASE.as_millis();
                if phase >= 3 {
                    self.done = true;
                    event_loop.exit();
                    return;
                }
                let size = renderer.size();
                let metrics = renderer.metrics();
                let workspace = WorkspaceScene::from_snapshot(
                    &snapshot(phase == 1),
                    PhysicalSize::new(size.width, size.height),
                    metrics,
                    0.0,
                    0.0,
                    |_, label| renderer.fit_tab_text(label, false),
                );
                // The grid fills the terminal, as Orbit sizes it, and the image runs past the
                // grid's right and bottom edges, under the notice.
                let cells = |extent: f32, cell: f32| {
                    ((extent - metrics.padding * 2.0) / cell).floor().max(1.0) as usize
                };
                let columns = cells(workspace.terminal.width, metrics.width);
                let content = (0..cells(workspace.terminal.height, metrics.height))
                    .map(|index| {
                        let text = format!("{index} text above the image band");
                        row(
                            &format!("{text:columns$.columns$}"),
                            (2..7).contains(&index).then_some(RED),
                        )
                    })
                    .collect();
                // The checkerboard, and the marker over a refused resource beside it.
                let mut scene = scene(content);
                let placement = |resource, target| ScenePlacement {
                    resource,
                    state: PlacementState::Ready,
                    source: SourceRect {
                        x: 0,
                        y: 0,
                        width: 64,
                        height: 64,
                    },
                    target,
                };
                scene.graphics = SceneGraphics {
                    attachment: 1,
                    placements: vec![
                        placement(
                            3,
                            SceneRect {
                                left: 4.0,
                                top: 1.0,
                                width: 200.0,
                                height: 80.0,
                            },
                        ),
                        placement(
                            2,
                            SceneRect {
                                left: 1.0,
                                top: 8.0,
                                width: 2.0,
                                height: 2.0,
                            },
                        ),
                    ],
                    placements_omitted: false,
                };
                if let Some(cursor) = scene.cursor.as_mut() {
                    cursor.column = 10;
                    cursor.row = 3;
                    cursor.color = SceneColor {
                        r: 255,
                        g: 255,
                        b: 255,
                    };
                }
                // Long enough to wrap onto several lines, so the notice reaches over the image.
                let status = if phase == 2 {
                    "a notice above the image ".repeat(20)
                } else {
                    String::new()
                };
                let outcome = renderer
                    .render(
                        Some(&scene),
                        None,
                        0.0,
                        Some(&workspace),
                        &[],
                        None,
                        WorkspaceFocus::Terminal,
                        &status,
                        true,
                        "",
                        u64::try_from(phase).unwrap() + 1,
                    )
                    .unwrap();
                if outcome == PresentOutcome::Presented {
                    self.presented[usize::try_from(phase).unwrap()] += 1;
                }
                renderer.window.request_redraw();
            }
        }
        let event_loop = EventLoop::builder()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let mut probe = Probe::default();
        event_loop.run_app(&mut probe).unwrap();
        assert!(probe.done && probe.presented.iter().all(|frames| *frames > 0));
    }
}
