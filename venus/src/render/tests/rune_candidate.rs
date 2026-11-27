//! The hosted renderer under the private Rune candidate (eon-yp6vg.7.7).

use super::*;
use crate::rune_candidate::{
    Entry, Failure, Fit, STEP_COUNT, SlotGeometry, fixtures, header_pilot, rune_host,
};
use images::DrawStep;
use rune::{Compositor, render::KIND_TRIANGLE};

#[path = "../../../tests/support/canonical.rs"]
mod canonical;
mod differential;
mod overlap;

#[test]
fn hosted_steps_prepare_images_and_text_in_painter_order() {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let mut renderer = Renderer::hosted(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        (1000, 640),
        1.0,
        clipping_test_fonts(),
    )
    .unwrap();
    // `assemble`, which shipping `Renderer::new` shares, allocates no backdrop: only a
    // hosted prepare makes one.
    assert!(renderer.hosted_backdrop.is_none());
    let mut entry = Entry::new().unwrap();
    let prepare = |renderer: &mut Renderer, entry: &mut Entry| {
        entry.sync_images(renderer);
        renderer
            .prepare_hosted(
                entry.scene(),
                (None, 0.0),
                (true, 0.0),
                entry.generation,
                "",
                "",
            )
            .unwrap();
        assert!(
            matches!(renderer.hosted_backdrop, Some((_, 6))),
            "one backdrop rectangle"
        );
    };
    prepare(&mut renderer, &mut entry);
    let standalone = (renderer.vertex_count, renderer.clear);
    let workspace = fixtures::header_workspace(&mut renderer, 0);
    renderer
        .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
        .unwrap();
    assert!(matches!(renderer.hosted_backdrop, Some((_, 0))));
    assert_eq!(renderer.clear, wgpu::Color::TRANSPARENT);
    prepare(&mut renderer, &mut entry);
    assert_eq!((renderer.vertex_count, renderer.clear), standalone);
    let prepared = renderer.glyphon_prepares;
    prepare(&mut renderer, &mut entry);
    assert_eq!(renderer.glyphon_prepares, prepared);
    // Shaped text buffers by the grid row they draw in; a row's layers sit near its top.
    let rows = |renderer: &Renderer| {
        let metrics = renderer.metrics();
        let mut rows = std::collections::BTreeMap::<i64, HashSet<_>>::new();
        for text in &renderer.text {
            let row = ((text.top - metrics.padding) / metrics.height).round();
            rows.entry(row as i64)
                .or_default()
                .insert(std::ptr::from_ref(&*text.buffer));
        }
        rows
    };
    let before = rows(&renderer);
    entry.advance();
    prepare(&mut renderer, &mut entry);
    let after = rows(&renderer);
    let reshaped = before
        .iter()
        .filter(|(top, buffers)| after.get(top) != Some(buffers))
        .count();
    assert_eq!(
        reshaped,
        1,
        "a one-cell change reshapes only its own row, of {} rows with text",
        before.len()
    );
    for _ in 0..3 {
        entry.advance();
        prepare(&mut renderer, &mut entry);
    }
    assert_eq!(entry.position().2, "image");
    let steps = draw_steps(
        renderer.vertex_count,
        (renderer.dynamic_vertex_count > 0).then_some(renderer.cursor_vertex_boundary),
        &renderer.images.draws,
    );
    let image = steps
        .iter()
        .position(|step| matches!(step, DrawStep::Image(_)))
        .expect("the image step draws its image");
    assert!(
        steps[..image]
            .iter()
            .any(|step| matches!(step, DrawStep::Rectangles(_))),
        "the grid's rectangles draw before the image: {steps:?}"
    );
    assert!(!renderer.text.is_empty(), "text draws last, over the image");
}

/// Readbacks and the probe's PNGs: `EON_RUNE_PROOF_DIR`, when set, keeps every pair.
struct Probe {
    out: Option<std::path::PathBuf>,
    failures: Vec<String>,
}

impl Probe {
    fn compare(
        &mut self,
        name: &str,
        (width, height): (u32, u32),
        candidate: &[u8],
        parent: &[u8],
    ) {
        if let Some(out) = &self.out {
            for (side, pixels) in [("rune", candidate), ("parent", parent)] {
                let path = out.join(format!("{name}-{side}.png"));
                rune::render::write_png(&path, width, height, pixels).unwrap();
            }
        }
        let difference = differential::pixel_difference((width, height), [parent, candidate]);
        let count = difference.map_or(0, |(_, _, count)| count);
        eprintln!("probe {name}: {count} differing pixels");
        if let Some(difference) = difference {
            self.failures.push(format!("{name}: {difference:?}"));
        }
    }
}

const OFFSCREEN: wgpu::TextureFormat = rune::render::OFFSCREEN_FORMAT;

fn packaged_fonts() -> FontSetup {
    FontSetup::with_directories(
        &FontSettings::default(),
        std::env::var_os("EON_VENUS_FONT_DIRS").as_deref(),
    )
    .unwrap()
}

fn probe_terminal(
    gpu: &rune::render::Gpu,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    scale: f64,
) -> Renderer {
    Renderer::hosted(
        gpu.device.clone(),
        gpu.queue.clone(),
        format,
        size,
        scale,
        packaged_fonts(),
    )
    .unwrap()
}

/// Draws one candidate frame of `rt` through the production compositor into a fresh target.
fn candidate_frame(
    gpu: &rune::render::Gpu,
    ui: &mut rune::render::Renderer,
    rt: &mut rune::Runtime<rune_host::State>,
    size: (u32, u32),
    render: impl FnOnce(&mut rune::Frame<'_, rune_host::State>),
) -> Vec<u8> {
    let format = ui.format();
    let target = rune::render::Offscreen::with_format(&gpu.device, size.0, size.1, format);
    read_candidate(gpu, ui, rt, size, &target, render)
}

fn read_candidate<S: 'static>(
    gpu: &rune::render::Gpu,
    ui: &mut rune::render::Renderer,
    rt: &mut rune::Runtime<S>,
    size: (u32, u32),
    target: &rune::render::Offscreen,
    render: impl FnOnce(&mut rune::Frame<'_, S>),
) -> Vec<u8> {
    crate::rune_candidate::offscreen_frame(
        (&gpu.device, &gpu.queue),
        ui,
        rt,
        (size, target),
        (0.0, true),
        render,
    )
    .unwrap()
}

/// The windowed path's own recording: the scene prepared as `render` prepares it, in a pass
/// cleared with the scene background, with no backdrop.
fn parent_frame(gpu: &rune::render::Gpu, parent: &mut Renderer, entry: &mut Entry) -> Vec<u8> {
    entry.sync_images(parent);
    prepare_parent(parent, entry.scene(), entry.generation, true);
    let size = parent.size();
    let target = rune::render::Offscreen::new(&gpu.device, size.width, size.height);
    read_parent(gpu, parent, &target, true)
}

fn prepare_parent(parent: &mut Renderer, scene: &Scene, generation: u64, blink: bool) {
    parent
        .prepare_frame(
            Some(scene),
            None,
            0.0,
            None,
            &[],
            None,
            WorkspaceFocus::Terminal,
            "",
            (blink, None),
            "",
            generation,
        )
        .unwrap();
}

fn read_parent(
    gpu: &rune::render::Gpu,
    parent: &mut Renderer,
    target: &rune::render::Offscreen,
    visible: bool,
) -> Vec<u8> {
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(if visible {
                        parent.clear
                    } else {
                        wgpu::Color::BLACK
                    }),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if visible {
            parent.draw_into(&mut pass).unwrap();
        }
    }
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    parent.atlas.trim();
    target.read(&gpu.device)
}

fn candidate_runtime(
    view: fn(&rune_host::State, &mut rune::Cx<rune_host::State>) -> rune::El,
    points: (f32, f32),
    scale: f32,
) -> rune::Runtime<rune_host::State> {
    let mut rt = rune::Runtime::new(
        rune_host::State::new(Entry::new().unwrap(), None),
        view,
        points,
        rune::Env::new(true, scale),
    );
    rt.set_srgb_blend(false);
    rt.track_marks = true;
    rt
}

fn rune_renderer(gpu: &rune::render::Gpu) -> rune::render::Renderer {
    let mut ui = rune::render::Renderer::new(&gpu.device, OFFSCREEN);
    ui.set_srgb_blend(false);
    ui
}

#[test]
fn rune_header_pilot_rejects_unadmitted_font_bytes() {
    let path = std::path::PathBuf::from(std::env::var_os("EON_VENUS_TEST_SANS_FONT").unwrap());
    let mut decoy = std::fs::read(&path).unwrap();
    decoy.push(1);
    let mono = std::fs::read(path.with_file_name("DejaVuSansMono.ttf")).unwrap();
    let mut fonts = packaged_fonts();
    assert!(header_pilot::fonts(&mut fonts, false, None).is_err());
    for bytes in [b"malformed SFNT".to_vec(), mono, decoy] {
        assert!(fonts.set_packaged_header_font(bytes.into()).is_err());
    }
}

#[test]
#[ignore = "requires a closure-only fontconfig on Linux"]
fn rune_header_pilot_mounted_title_uses_the_measured_clip() {
    use crate::contour::TITLE_FIT_WORK;

    let look = crate::render::HostedLook {
        panel_heading: PanelHeading::Notch,
        pane_frames: true,
        background_opacity: 0.8,
        border_width: 2,
        focus_border_width: 2,
        ..HOSTED_LOOK
    };
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let mut admitted = packaged_fonts();
    let path = std::env::var_os("EON_VENUS_TEST_SANS_FONT");
    let family = header_pilot::fonts(&mut admitted, true, path.clone()).unwrap();
    let bytes = std::fs::read(path.as_ref().unwrap()).unwrap();
    let mut parent_fonts = packaged_fonts();
    header_pilot::fonts(&mut parent_fonts, false, path).unwrap();
    let mut parent = Renderer::hosted_with_look(
        device.clone(),
        queue.clone(),
        OFFSCREEN,
        (1280, 800),
        1.0,
        parent_fonts,
        look,
    )
    .unwrap();
    let emoji_family = if cfg!(target_vendor = "apple") {
        "Apple Color Emoji"
    } else {
        "Noto Color Emoji"
    };
    let mut renderer =
        Renderer::hosted_with_look(device, queue, OFFSCREEN, (1280, 800), 1.0, admitted, look)
            .unwrap();
    renderer.enable_header_pilot(true);
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let points = (1280.0, 800.0);
        renderer.resize(
            PhysicalSize::new((points.0 * scale) as u32, (points.1 * scale) as u32),
            f64::from(scale),
        );
        let mut entry = Entry::new().unwrap();
        let mut rt = candidate_runtime(rune_host::view, points, scale);
        for step in 0..STEP_COUNT {
            let workspace = fixtures::header_workspace(&mut renderer, step);
            entry.sync_images(&mut renderer);
            TITLE_FIT_WORK.set([0; 3]);
            renderer
                .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
                .unwrap();
            let [fits, measurements, _] = TITLE_FIT_WORK.get();
            if [1, 4].contains(&step) {
                assert_eq!([fits, measurements], [0, 0]);
            } else {
                assert!((1..=2).contains(&fits));
                assert!(measurements > 0 && measurements <= 24);
            }
            renderer
                .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
                .unwrap();
            assert_eq!(TITLE_FIT_WORK.get(), [fits, measurements, 0]);
            let paint = &renderer.header_paint;
            let title = paint.title.clone().unwrap();
            assert!(title.source.starts_with("1  "));
            if step == 2 {
                assert!(title.source.contains('…') && title.source.ends_with("end"));
            }
            assert!(!paint.outline.is_empty() && paint.outline.len() <= 512);
            rt.update(|state, _| state.header = Some((family, Some(title.clone()))));
            rt.frame(0.0);
            let elements = rt.elements();
            let element = elements
                .iter()
                .find(|e| e.key.as_deref() == Some("popup-title"))
                .unwrap();
            assert_eq!(element.text.as_deref(), Some(title.source.as_str()));
            assert!(!element.wrap && !element.truncate && !element.text_cut);
            assert!(
                element.rect[..2]
                    .iter()
                    .zip(title.origin)
                    .all(|(point, px)| (point * scale - px).abs() <= 1.0 / 64.0)
            );
            assert!(elements[element.parent.unwrap()].clips);
            assert!(
                (element.rect[2] * scale - (title.clip.right() - title.origin[0])).abs()
                    <= 1.0 / 64.0
            );
            assert_eq!(
                element.text_style.as_ref().unwrap().family.as_deref(),
                Some(family)
            );
            let work = |rt: &rune::Runtime<rune_host::State>| {
                (
                    rt.text.shaped,
                    rt.text.rebroken,
                    rt.text.cached_layout_count(),
                )
            };
            let count = work(&rt);
            let layout = rt
                .text
                .layout_rich(&title.source, &[], title.style(family), scale, None);
            assert_eq!(work(&rt), count, "mounted layout cache hit");
            assert_eq!(layout.len(), 1);
            let line = layout.get(0).unwrap();
            parent.resize(renderer.size(), f64::from(scale));
            entry.sync_images(&mut parent);
            parent
                .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
                .unwrap();
            let reference = parent
                .text
                .iter()
                .find(|text| text.buffer.lines[0].text() == title.source)
                .unwrap();
            let venus = reference.buffer.layout_runs().next().unwrap();
            assert_eq!([reference.left, reference.top], title.origin);
            assert_eq!(reference.bounds(), title.clip);
            assert!((0.0..=2.0).contains(&(line.metrics().baseline - venus.line_y.round())));
            let db = parent.fonts.font_system.db();
            let emoji_id = db.query(&glyphon::fontdb::Query {
                families: &[Family::Name(emoji_family)],
                ..Default::default()
            });
            let mut quads = Vec::new();
            rt.text
                .glyphs_for_layout(&layout, scale, title.origin, true, &mut quads);
            assert!(line.items().all(|item| match item {
                rune::text::parley::PositionedLayoutItem::GlyphRun(run) =>
                    run.glyphs().all(|glyph| glyph.id != 0),
                _ => false,
            }));
            for venus_glyph in venus.glyphs {
                let byte = venus_glyph.start;
                let cluster = &title.source[byte..venus_glyph.end];
                let run = line
                    .runs()
                    .find(|run| run.text_range().contains(&byte))
                    .unwrap();
                if matches!(cluster, "😀" | "🎉" | "👩‍💻" | "👨‍👩‍👧‍👦" | "🏳️‍🌈")
                {
                    let id = if cluster == "😀" {
                        emoji_id
                    } else {
                        Some(venus_glyph.font_id)
                    };
                    assert_eq!(
                        id.and_then(|id| db.with_face_data(id, |data, index| (
                            data == run.font().data.as_ref(),
                            index == run.font().index
                        ))),
                        Some((true, true)),
                        "face for {cluster}"
                    );
                    let mut color = quads
                        .iter()
                        .filter(|quad| quad.byte as usize == byte && quad.color != 0);
                    let quad = color.next().unwrap();
                    assert!(color.next().is_none(), "split color cluster {cluster}");
                    let [left, top, _, _] = quad.rect;
                    rt.text
                        .color_glyph(quad.color)
                        .with_rgba(|width, _, rgba| {
                            let mut count = 0;
                            for (i, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
                                if pixel[3] != 0 {
                                    count += 1;
                                    assert!(
                                        title.clip.contains(
                                            left + (i as u32 % width) as f32 + 0.5,
                                            top + (i as u32 / width) as f32 + 0.5
                                        ),
                                        "{scale}× {cluster}: ink past shared clip"
                                    );
                                }
                            }
                            assert!(count > 0);
                        })
                        .unwrap();
                } else {
                    assert_eq!(run.font().index, 0);
                    assert_eq!(run.font().data.as_ref(), bytes);
                }
            }
            assert!(
                line.metrics().advance <= title.clip.right() - title.origin[0] + 1.0 / 256.0,
                "{scale}× step {step} {:?}: advance {} beyond clip {:?}, origin {:?}",
                title.source,
                line.metrics().advance,
                title.clip,
                title.origin
            );
            let mut marked = 0;
            for instance in rt
                .instances
                .iter()
                .zip(&rt.marks)
                .filter_map(|(i, &mark)| (mark == header_pilot::TITLE).then_some(i))
            {
                marked += 1;
                assert_eq!(
                    instance.clip,
                    [
                        title.clip.left,
                        title.clip.top,
                        title.clip.right(),
                        title.clip.bottom()
                    ]
                );
            }
            assert!(marked > 0);
            entry.advance();
        }
    }
}

#[test]
#[ignore = "needs a real GPU adapter and the closure fonts"]
fn rune_header_pilot_outline_and_grid_match_venus() {
    let gpu = rune::render::Gpu::headless();
    eprintln!("adapter: {:?}", gpu.adapter.get_info());
    let mut probe = Probe {
        out: std::env::var_os("EON_RUNE_PROOF_DIR").map(Into::into),
        failures: Vec::new(),
    };
    let look = HostedLook {
        background_opacity: 0.8,
        panel_heading: PanelHeading::Notch,
        pane_frames: true,
        border_width: 2,
        focus_border_width: 2,
        ..HOSTED_LOOK
    };
    for (points, scale, look) in [
        ((1280.0, 800.0), 1.0, look),
        ((1280.0, 800.0), 2.0, look),
        ((320.0, 180.0), 1.25, HOSTED_LOOK),
    ] {
        let size = ((points.0 * scale) as u32, (points.1 * scale) as u32);
        let mut fonts = packaged_fonts();
        let path = std::env::var_os("EON_VENUS_TEST_SANS_FONT");
        let family = header_pilot::fonts(&mut fonts, true, path.clone()).unwrap();
        let failure = Failure::default();
        let mut compositor = rune_host::GridCompositor::new(fonts, failure.clone(), None);
        compositor.look = look;
        let mut ui = rune_renderer(&gpu);
        let mut rt = candidate_runtime(rune_host::view, points, scale);
        rt.transparent = look.background_opacity < 1.0;
        rt.update(|state, _| state.header = Some((family, None)));

        let mut fonts = packaged_fonts();
        header_pilot::fonts(&mut fonts, false, path).unwrap();
        let mut parent = Renderer::hosted_with_look(
            gpu.device.clone(),
            gpu.queue.clone(),
            OFFSCREEN,
            size,
            f64::from(scale),
            fonts,
            look,
        )
        .unwrap();
        let target = rune::render::Offscreen::with_format(&gpu.device, size.0, size.1, OFFSCREEN);
        let mut entry = Entry::new().unwrap();
        for index in 0..=STEP_COUNT {
            let step = index % STEP_COUNT;
            let mut candidate = Vec::new();
            let before = rt.state.follow_ups;
            let text_work = (rt.text.shaped, rt.text.rebroken);
            let passes = if index == 0 { 2 } else { 1 };
            for _ in 0..passes {
                rt.frame(0.0);
                candidate = candidate_frame(&gpu, &mut ui, &mut rt, size, |f| compositor.render(f));
                assert_eq!(failure.get(), None);
                assert_eq!(compositor.hook_calls, 1);
                if rt.state.drawn.is_some() {
                    break;
                }
            }
            assert_eq!(
                rt.state.drawn.map(|(cycle, step, _)| (cycle, step)),
                Some((u64::from(index == STEP_COUNT), step)),
                "first warm frame must present the current title at {scale}× {step}"
            );
            let subsequent = rt.state.follow_ups - before;
            assert!(subsequent <= u64::from(index == 0));
            if [1, 4].contains(&step) {
                assert_eq!((rt.text.shaped, rt.text.rebroken), text_work);
            }
            let workspace = fixtures::header_workspace(&mut parent, step);
            entry.sync_images(&mut parent);
            parent
                .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
                .unwrap();
            let expected = read_parent(&gpu, &mut parent, &target, true);
            if index == 0 {
                parent.enable_header_pilot(false);
                parent
                    .prepare_hosted_workspace(entry.scene(), &workspace, entry.generation)
                    .unwrap();
                assert_eq!(read_parent(&gpu, &mut parent, &target, true), expected);
            }
            let case = format!("header-{scale}-{index}");
            if let Some(out) = &probe.out {
                for (name, pixels) in [("rune", &candidate), ("venus", &expected)] {
                    rune::render::write_png(
                        &out.join(format!("{case}-{name}-original.png")),
                        size.0,
                        size.1,
                        pixels,
                    )
                    .unwrap();
                }
            }
            if let Some((_, Some(title))) = &rt.state.header {
                let clip = title.clip;
                let instances = std::mem::take(&mut rt.instances);
                let marks = std::mem::take(&mut rt.marks);
                for (&instance, &mark) in instances.iter().zip(&marks) {
                    if mark != header_pilot::TITLE {
                        rt.instances.push(instance);
                        rt.marks.push(mark);
                    }
                }
                let without_title = read_candidate(&gpu, &mut ui, &mut rt, size, &target, |f| {
                    compositor.render(f)
                });
                rt.instances = instances;
                rt.marks = marks;
                let (_, Some(title)) = rt.state.header.as_ref().unwrap() else {
                    unreachable!()
                };
                parent.text.retain(|text| {
                    text.bounds() != clip || text.buffer.lines[0].text() != title.source
                });
                parent
                    .text_renderer
                    .prepare_with_custom(
                        &parent.device,
                        &parent.queue,
                        &mut parent.fonts.font_system,
                        &mut parent.atlas,
                        &parent.viewport,
                        text_areas(&parent.text, parent.text_overlay.as_ref()),
                        &mut parent.swash_cache,
                        |request| rasterize_bar_icon(request, None),
                    )
                    .unwrap();
                let venus_without_title = read_parent(&gpu, &mut parent, &target, true);
                probe.compare(&case, size, &without_title, &venus_without_title);
                for (index, ((rune, venus), context)) in candidate
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.as_chunks::<4>().0)
                    .zip(without_title.as_chunks::<4>().0)
                    .enumerate()
                {
                    if rune != context || venus != context {
                        let x = (index % size.0 as usize) as f32 + 0.5;
                        let y = (index / size.0 as usize) as f32 + 0.5;
                        assert!(
                            clip.contains(x, y),
                            "{case}: title affected pixel ({x}, {y}) past clip"
                        );
                    }
                }
            } else {
                probe.compare(&case, size, &candidate, &expected);
            }
            entry.advance();
            rt.update(|state, _| {
                state.advance();
                state.drawn = None;
            });
        }
    }
    assert!(probe.failures.is_empty(), "{:?}", probe.failures);
}

#[test]
fn rune_header_pilot_checks_compact_and_surface_geometry() {
    let (device, queue) = wgpu::Device::noop(&DeviceDescriptor::default());
    let size = (96, 180);
    let mut renderer =
        Renderer::hosted(device, queue, OFFSCREEN, size, 1.0, packaged_fonts()).unwrap();
    renderer.enable_header_pilot(true);
    let mut rt = candidate_runtime(rune_host::view, (96.0, 180.0), 1.0);
    let entry = Entry::new().unwrap();
    let scene = entry.scene();
    let mut out = Vec::new();
    for (height, visible) in [(180, true), (80, false)] {
        renderer.resize(PhysicalSize::new(96, height), 1.0);
        let workspace = fixtures::header_workspace(&mut renderer, 3);
        renderer
            .prepare_hosted_workspace(scene, &workspace, entry.generation)
            .unwrap();
        assert!(renderer.workspace_title_tabs(&workspace)[0].is_none());
        let paint = &renderer.header_paint;
        assert_eq!(paint.title.is_some(), visible);
        assert_eq!(!paint.outline.is_empty(), visible);
        if visible {
            rt.update(|state, _| state.header = Some(("DejaVu Sans", paint.title.clone())));
            rt.frame(0.0);
        }
        let title_mark = header_pilot::TITLE;
        let marked = rt.marks.iter().filter(|&&mark| mark == title_mark).count();
        assert!(marked > 0);
        assert!(header_pilot::splice(&rt, Some(paint), &mut out));
        let triangles = 2 * paint.outline.len();
        let count = out.iter().filter(|i| i.params[1] == KIND_TRIANGLE).count();
        assert_eq!(count, triangles);
        let hidden = if visible { 0 } else { marked };
        assert_eq!(out.len() + hidden, rt.instances.len() - 1 + triangles);
        assert!(header_pilot::splice(&rt, None, &mut out));
        assert_eq!(out.len(), rt.instances.len() - 1 - marked);
    }
    rt.marks.pop();
    assert!(!header_pilot::splice(&rt, None, &mut out));
    assert!(out.is_empty());

    let device = &renderer.device;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = rune::render::Offscreen::with_format(device, size.0, size.1, format);
    let mut ui = rune::render::Renderer::new(device, format);
    let mut frame = rune::Frame {
        device,
        queue: &renderer.queue,
        encoder: &mut device.create_command_encoder(&Default::default()),
        target: &target.view,
        format,
        size,
        dt: 0.0,
        update_ms: 0.0,
        rt: &mut rt,
        ui: &mut ui,
    };
    let failure = Failure::default();
    let mut comp = rune_host::GridCompositor::new(packaged_fonts(), failure.clone(), None);
    assert!(comp.prepare(&mut frame).is_none());
    assert!(failure.get().unwrap().contains("requires an sRGB target"));
}

const BEFORE: rune::Color = rune::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const AFTER: rune::Color = rune::Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
/// Inset slot, larger than its clipping ancestor.
const SLOT: [f32; 4] = [37.0, 23.0, 400.0, 260.0];
const CLIP: [f32; 2] = [300.0, 200.0];

fn inset_view(_: &rune_host::State, _: &mut rune::Cx<rune_host::State>) -> rune::El {
    rune::el()
        .w_full()
        .h_full()
        .fill(BEFORE)
        .child(
            rune::el()
                .absolute(SLOT[0], SLOT[1])
                .size(CLIP[0], CLIP[1])
                .clip_rounded(0.0)
                .child(rune::effect(1).key("grid").size(SLOT[2], SLOT[3])),
        )
        .child(
            rune::el()
                .absolute(SLOT[0], SLOT[1])
                .size(16.0, 16.0)
                .fill(AFTER),
        )
}

fn isolated_view(_: &rune_host::State, _: &mut rune::Cx<rune_host::State>) -> rune::El {
    rune::el()
        .w_full()
        .h_full()
        .fill(BEFORE)
        .layer_opacity(0.5)
        .child(rune_host::grid_view())
}

fn disallow_incompatible_passes(gpu: &rune::render::Gpu) {
    let size = (640, 320);
    let points = (size.0 as f32, size.1 as f32);
    let format = OFFSCREEN.remove_srgb_suffix();
    let mut renderer = probe_terminal(gpu, format, size, 1.0);
    let entry = Entry::new().unwrap();
    renderer
        .prepare_hosted(
            entry.scene(),
            (None, 0.0),
            (true, 0.0),
            entry.generation,
            "",
            "",
        )
        .unwrap();
    let mut ui = rune::render::Renderer::new(&gpu.device, format);
    for (view, blend, wrong_pass) in [
        (rune_host::view as _, false, None),
        (
            isolated_view as _,
            false,
            Some("Rgba16Float, in a layer: true, sRGB blend: false"),
        ),
        (
            rune_host::view as _,
            true,
            Some("Rgba8Unorm, in a layer: false, sRGB blend: true"),
        ),
    ] {
        let mut rt = candidate_runtime(view, points, 1.0);
        rt.set_srgb_blend(blend);
        ui.set_srgb_blend(blend);
        rt.frame(0.0);
        candidate_frame(gpu, &mut ui, &mut rt, size, |f| {
            let preflight = rune_host::preflight(f, None);
            if blend {
                assert_eq!(
                    preflight.unwrap_err(),
                    "the Venus grid requires a linear-blended frame"
                );
            } else {
                let target = (f.device_key(), OFFSCREEN);
                assert!(
                    rune_host::preflight(f, Some(target)).is_err(),
                    "same device, wrong format"
                );
                assert!(preflight.is_ok());
            }
            // Bypass preflight only to test the guard at the actual effect pass.
            let width = if wrong_pass.is_some() && !blend {
                points.0 - 20.0
            } else {
                points.0
            };
            let geometry = SlotGeometry::new([0.0, 0.0, width, points.1], 1.0);
            let drawn = rune_host::draw_slot(f, Some((&renderer, geometry)), Fit::Exact);
            assert_eq!(drawn.hook_calls, 1);
            assert!(
                drawn.moved.is_none(),
                "pass mismatch takes precedence over a moved slot"
            );
            if let Some(metadata) = wrong_pass {
                let reason = drawn.refusal.unwrap();
                assert!(reason.contains(metadata), "{reason}");
                assert!(drawn.slot.is_none());
                eprintln!("probe {reason}");
            } else {
                assert!(drawn.refusal.is_none());
                assert!(drawn.slot.is_some());
            }
        });
    }
}

#[test]
#[ignore = "needs a real GPU adapter (lavapipe) and the packaged fonts; see venus/docs/DEVELOPMENT.md"]
fn rune_hosted_grid_matches_the_venus_parent() {
    let gpu = rune::render::Gpu::headless();
    let adapter = gpu.adapter.get_info();
    eprintln!("adapter: {adapter:?}");
    let mut probe = Probe {
        out: std::env::var_os("EON_RUNE_PROOF_DIR").map(Into::into),
        failures: Vec::new(),
    };

    // The packaged font environment: the shipping wrapper's fonts.conf (p2.sh checks the
    // value). Its list also names host directories, as shipping Venus's does, so the generic
    // monospace primary of Venus's default settings resolves as shipping Venus resolves it
    // (recorded, not asserted). Every named family must resolve to its packaged face.
    let fonts = packaged_fonts();
    eprintln!("FONTCONFIG_FILE={:?}", std::env::var_os("FONTCONFIG_FILE"));
    let db = fonts.font_system.db();
    let face_path = |family: glyphon::Family<'_>| {
        let face = db
            .query(&glyphon::fontdb::Query {
                families: &[family],
                ..Default::default()
            })
            .and_then(|id| db.face(id))
            .unwrap_or_else(|| panic!("no face for {family:?}"));
        let glyphon::fontdb::Source::File(path) = &face.source else {
            panic!("{family:?} is not a font file");
        };
        eprintln!("font {family:?}: {}", path.display());
        path.clone()
    };
    face_path(glyphon::Family::Monospace);
    for family in [
        glyphon::Family::Name("DejaVu Sans Mono"),
        glyphon::Family::Name(super::cells::NERD_FONT_FAMILY),
        glyphon::Family::Name("Noto Color Emoji"),
        glyphon::Family::Name("Accelerando Terminal"),
    ] {
        let path = face_path(family);
        assert!(
            path.starts_with("/nix/store"),
            "{family:?} resolves to a host face: {}",
            path.display()
        );
    }
    drop(fonts);

    // Matched: every step of a cycle and the next cycle's first, at scale 1 and 2.
    // Wide enough for the image placement, which crosses the 100-column grid's right edge.
    let points = (1280.0, 800.0);
    for scale in [1.0_f32, 2.0] {
        let mut size = ((points.0 * scale) as u32, (points.1 * scale) as u32);
        let mut rt = candidate_runtime(rune_host::view, points, scale);
        rt.window_focus(true);
        let mut ui = rune_renderer(&gpu);
        let failure = Failure::default();
        let mut compositor =
            rune_host::GridCompositor::new(packaged_fonts(), failure.clone(), None);
        let mut parent = probe_terminal(&gpu, OFFSCREEN, size, f64::from(scale));
        let mut parent_entry = Entry::new().unwrap();
        for index in 0..=STEP_COUNT + usize::from(scale == 1.0) {
            let tiled = index > STEP_COUNT;
            if tiled {
                size = (640, 800);
                rt.set_viewport((640.0, 800.0), scale);
                rt.invalidate();
                rt.window_focus(false);
                parent.resize(PhysicalSize::new(size.0, size.1), f64::from(scale));
            } else if index > 0 {
                rt.update(|state, _| state.entry.advance());
                parent_entry.advance();
            }
            rt.frame(if index == 0 || tiled { 0.0 } else { 1.0 / 60.0 });
            let candidate = candidate_frame(&gpu, &mut ui, &mut rt, size, |f| compositor.render(f));
            assert_eq!(failure.get(), None);
            assert_eq!(
                compositor.hook_calls, 1,
                "one slot, one hook call, one pass"
            );
            assert!(compositor.drew);
            let parent_pixels = parent_frame(&gpu, &mut parent, &mut parent_entry);
            let (_, step, name) = parent_entry.position();
            // The image step must show its image, or its match proves nothing about images.
            let checker = parent_pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| matches!(pixel, [30, 90, 230, 255] | [240, 220, 40, 255]))
                .count();
            assert_eq!(
                name == "image",
                checker > 1000,
                "{name} at x{scale}: {checker} opaque checkerboard pixels"
            );
            let capture = if tiled {
                "tiled-resize".to_owned()
            } else {
                format!("x{scale}-{index}-{step}-{name}")
            };
            probe.compare(&capture, size, &candidate, &parent_pixels);
        }

        // A frame on another device is refused through the same compositor, with no draw.
        let other = rune::render::Gpu::headless();
        let mut other_ui = rune_renderer(&other);
        let _ = candidate_frame(&other, &mut other_ui, &mut rt, size, |f| {
            compositor.render(f)
        });
        assert!(
            !compositor.drew,
            "nothing draws with another device's resources"
        );
        assert!(
            failure
                .get()
                .is_some_and(|message| message.contains("another GPU device")),
            "{:?}",
            failure.get()
        );
    }

    // Viewport, clip and painter order, with the slot inset and cut by its ancestor.
    let points = (800.0, 520.0);
    let size = (800, 520);
    let mut rt = candidate_runtime(inset_view, points, 1.0);
    let mut ui = rune_renderer(&gpu);
    let failure = Failure::default();
    let mut compositor = rune_host::GridCompositor::new(packaged_fonts(), failure.clone(), None);
    rt.frame(0.0);
    let candidate = candidate_frame(&gpu, &mut ui, &mut rt, size, |f| compositor.render(f));
    assert_eq!(failure.get(), None);
    assert!(compositor.drew);
    let mut parent = probe_terminal(&gpu, OFFSCREEN, (SLOT[2] as u32, SLOT[3] as u32), 1.0);
    let parent_pixels = parent_frame(&gpu, &mut parent, &mut Entry::new().unwrap());
    let pixel = |pixels: &[u8], width: u32, x: u32, y: u32| {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    let (left, top) = (SLOT[0] as u32, SLOT[1] as u32);
    let mut mismatched = 0;
    let mut wrong_outside = 0;
    for y in 0..size.1 {
        for x in 0..size.0 {
            let got = pixel(&candidate, size.0, x, y);
            let in_after = (left..left + 16).contains(&x) && (top..top + 16).contains(&y);
            let in_clip = (left..left + CLIP[0] as u32).contains(&x)
                && (top..top + CLIP[1] as u32).contains(&y);
            if in_after {
                if got != [0, 255, 0, 255] {
                    wrong_outside += 1;
                }
            } else if in_clip {
                if got != pixel(&parent_pixels, SLOT[2] as u32, x - left, y - top) {
                    mismatched += 1;
                }
            } else if got != [255, 0, 0, 255] {
                wrong_outside += 1;
            }
        }
    }
    eprintln!("probe inset: {mismatched} slot pixels differ, {wrong_outside} outside pixels wrong");
    if let Some(out) = &probe.out {
        rune::render::write_png(&out.join("inset-rune.png"), size.0, size.1, &candidate).unwrap();
        rune::render::write_png(
            &out.join("inset-parent.png"),
            SLOT[2] as u32,
            SLOT[3] as u32,
            &parent_pixels,
        )
        .unwrap();
    }
    if mismatched > 0 || wrong_outside > 0 {
        probe.failures.push(format!(
            "inset: {mismatched} slot pixels differ, {wrong_outside} outside pixels wrong"
        ));
    }
    disallow_incompatible_passes(&gpu);
    assert!(probe.failures.is_empty(), "{:#?}", probe.failures);
}
