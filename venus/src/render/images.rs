//! Terminal images (VEN-C31): textures, the textured-quad pipeline, and the band each terminal's
//! images occupy between its cell backgrounds and its foreground.

use super::{
    CellMetrics, RectangleBatch, RenderError, SceneRect, upload_vertices, valid_rect, vertex_count,
};
use crate::scene::SceneImage;
use std::{collections::HashMap, ops::Range};
use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites, ErrorFilter, Extent3d,
    FilterMode, FragmentState, MultisampleState, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PrimitiveState, RenderPass, RenderPipeline, RenderPipelineDescriptor,
    SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor,
    TextureViewDimension, VertexAttribute, VertexBufferLayout, VertexFormat, VertexState,
    VertexStepMode,
};

/// Position and texture coordinate, two `f32` pairs.
const IMAGE_VERTEX_SIZE: u64 = 16;

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
    texture: u64,
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
    /// Adds a terminal's images at this point of the stream, under the batch's clips, with each
    /// target measured in cells from the grid corner (`left`, `top`).
    pub(super) fn push_images(
        &mut self,
        images: &[SceneImage],
        left: f32,
        top: f32,
        metrics: CellMetrics,
    ) {
        for image in images {
            let target = SceneRect {
                left: left + image.target.left * metrics.width,
                top: top + image.target.top * metrics.height,
                width: image.target.width * metrics.width,
                height: image.target.height * metrics.height,
            };
            if !valid_rect(target) || !valid_rect(image.source) {
                continue;
            }
            let Some(visible) = self
                .clip
                .map_or(Some(target), |clip| target.intersection(clip))
            else {
                continue;
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

/// The image pipeline, the uploaded textures by key, and this frame's image quads.
pub(super) struct Images {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: wgpu::Sampler,
    format: TextureFormat,
    textures: HashMap<u64, BindGroup>,
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
            textures: HashMap::new(),
            vertices,
            vertex_capacity: IMAGE_VERTEX_SIZE,
            draws: Vec::new(),
        }
    }

    /// Uploads `rgba` pixels as texture `key`, replacing any texture under that key. A side
    /// outside the device's limit, a wrong pixel length, or an allocation error the device reports
    /// refuses the image and leaves no texture under `key`.
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), RenderError> {
        self.textures.remove(&key);
        let limit = device.limits().max_texture_dimension_2d;
        if !(1..=limit).contains(&width) || !(1..=limit).contains(&height) {
            return Err(RenderError(format!(
                "a {width} by {height} image does not fit the GPU's {limit}-pixel texture side"
            )));
        }
        if u64::from(width) * u64::from(height) * 4 != rgba.len() as u64 {
            return Err(RenderError(format!(
                "a {width} by {height} RGBA image cannot have {} bytes",
                rgba.len()
            )));
        }
        let texture = self.allocate(device, queue, width, height, rgba)?;
        self.textures.insert(key, texture);
        Ok(())
    }

    /// Creates and fills a texture inside error scopes, so a failure is a refusal rather than an
    /// uncaptured error.
    fn allocate(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<BindGroup, RenderError> {
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
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            rgba,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: Some(height),
            },
            size,
        );
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
        match validation.or(memory) {
            Some(error) => Err(RenderError(format!("the GPU refused an image: {error}"))),
            None => Ok(bind_group),
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
        let Some(texture) = self.textures.get(&draw.texture) else {
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
    use crate::{
        Color as SceneColor, DrawCell, DrawCursor, DrawRow, Scene, SceneGraphics, ScenePreview,
        scene::CardFill,
    };
    use orbit_protocol::{CellWidth, CursorShape, Screen, Underline, session::VerticalDirection};

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

    fn scene(content: Vec<DrawRow>, images: Vec<SceneImage>) -> Scene {
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
            images,
        }
    }

    fn whole(texture: u64, target: SceneRect) -> SceneImage {
        SceneImage {
            texture,
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
        let scene = scene(
            vec![row("ab", Some(RED)), row("cd", None)],
            vec![
                whole(
                    1,
                    SceneRect {
                        left: 0.0,
                        top: 0.0,
                        width: 2.0,
                        height: 3.0,
                    },
                ),
                whole(
                    2,
                    SceneRect {
                        left: 1.0,
                        top: 1.0,
                        width: 1.0,
                        height: 1.0,
                    },
                ),
            ],
        );
        let mut batch = RectangleBatch::new(400, 300);
        let cursor = build_scene_rectangles(
            &mut batch,
            &scene,
            Some(&preview),
            true,
            metrics,
            origin,
            true,
        );

        // The live and the preview row backgrounds come first, then both images in order.
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
            [(after, 1), (after, 2)]
        );
        // Underlines of both live rows and the cursor follow; preview foregrounds come last.
        let underline = 6;
        assert_eq!(cursor, after + 4 * underline + 6);
        assert_eq!(vertex_count(&batch.bytes), cursor + 2 * underline);
        // The first image reaches into the preview row but stops at the live rows.
        let live_bottom = metrics.padding + 2.0 * metrics.height;
        assert!(
            image_vertices(&batch)
                .iter()
                .all(|[_, y, ..]| live_bottom - y > -0.001)
        );

        // A terminal clip that misses the grid, as when padding is wider than the terminal,
        // leaves no image in that padding, even one placed to reach into it.
        let reaching = Scene {
            graphics: SceneGraphics::default(),
            images: vec![whole(
                1,
                SceneRect {
                    left: -1.0,
                    top: 0.0,
                    width: 2.0,
                    height: 1.0,
                },
            )],
            ..scene
        };
        let mut batch = RectangleBatch::new(400, 300);
        batch.clip = Some(SceneRect {
            left: 2.0,
            top: 0.0,
            width: 8.0,
            height: 300.0,
        });
        build_scene_rectangles(&mut batch, &reaching, None, true, metrics, origin, true);
        assert!(batch.images.is_empty() && batch.image_bytes.is_empty());
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
            texture: 3,
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
            batch.push_images(&[image], grid.left, grid.top, metrics);

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
            batch.push_images(&[image], grid.left, grid.top, metrics);
        }
        assert!(batch.images.is_empty() && batch.image_bytes.is_empty());
    }

    #[test]
    fn draw_steps_put_images_under_later_rectangles_and_the_trailing_cursor() {
        let image = |after| ImageDraw {
            after,
            texture: 0,
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
        use eon_workspace_protocol::v22::{
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
                    FontSetup::new(&FontSettings::default()).unwrap(),
                ))
                .unwrap();
                let limit = renderer.device.limits().max_texture_dimension_2d;
                assert!(renderer.upload_image(1, limit + 1, 1, &[]).is_err());
                assert!(renderer.upload_image(1, 2, 2, &[0; 15]).is_err());
                // Past the limit check, the error scope turns the device's error into a refusal
                // instead of the uncaptured-error panic.
                let oversize = vec![0; (limit as usize + 1) * 4];
                assert!(
                    renderer
                        .images
                        .allocate(&renderer.device, &renderer.queue, limit + 1, 1, &oversize)
                        .is_err()
                );
                renderer.upload_image(1, 64, 64, &checkerboard()).unwrap();
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
                let mut scene = scene(
                    content,
                    vec![whole(
                        1,
                        SceneRect {
                            left: 4.0,
                            top: 1.0,
                            width: 200.0,
                            height: 80.0,
                        },
                    )],
                );
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
