//! The hosted renderer under the private Rune candidate (eon-yp6vg.7.7).

use super::*;
use crate::rune_candidate::{Entry, Failure, Fit, STEP_COUNT, SlotGeometry, rune_host};
use images::DrawStep;
use rune::Compositor;

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
            .prepare_hosted(entry.scene(), true, entry.generation, "", "")
            .unwrap();
        assert!(
            matches!(renderer.hosted_backdrop, Some((_, 6))),
            "one backdrop rectangle"
        );
    };
    prepare(&mut renderer, &mut entry);
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
            rune::render::write_png(
                &out.join(format!("{name}-rune.png")),
                width,
                height,
                candidate,
            )
            .unwrap();
            rune::render::write_png(
                &out.join(format!("{name}-parent.png")),
                width,
                height,
                parent,
            )
            .unwrap();
        }
        let differing = candidate
            .as_chunks::<4>()
            .0
            .iter()
            .zip(parent.as_chunks::<4>().0)
            .filter(|(a, b)| a != b)
            .count();
        eprintln!("probe {name}: {differing} differing pixels");
        if candidate.len() != parent.len() || differing > 0 {
            self.failures
                .push(format!("{name}: {differing} pixels differ"));
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
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut frame = rune::Frame {
            device: &gpu.device,
            queue: &gpu.queue,
            encoder: &mut encoder,
            target: &target.view,
            format,
            size,
            dt: 0.0,
            update_ms: 0.0,
            rt,
            ui,
        };
        render(&mut frame);
    }
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    target.read(&gpu.device)
}

/// The windowed path's own recording: the scene prepared as `render` prepares it, in a pass
/// cleared with the scene background, with no backdrop.
fn parent_frame(gpu: &rune::render::Gpu, parent: &mut Renderer, entry: &mut Entry) -> Vec<u8> {
    entry.sync_images(parent);
    parent
        .prepare_frame(
            Some(entry.scene()),
            None,
            0.0,
            None,
            &[],
            None,
            WorkspaceFocus::Terminal,
            "",
            true,
            "",
            entry.generation,
        )
        .unwrap();
    let size = parent.size();
    let target = rune::render::Offscreen::new(&gpu.device, size.width, size.height);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(parent.clear),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        parent.draw_into(&mut pass).unwrap();
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
        rune_host::State {
            entry: Entry::new().unwrap(),
            drawn: None,
        },
        view,
        points,
        rune::Env::new(true, scale),
    );
    rt.set_srgb_blend(false);
    rt
}

fn rune_renderer(gpu: &rune::render::Gpu) -> rune::render::Renderer {
    let mut ui = rune::render::Renderer::new(&gpu.device, OFFSCREEN);
    ui.set_srgb_blend(false);
    ui
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
    let mut renderer = Renderer::hosted(
        gpu.device.clone(),
        gpu.queue.clone(),
        format,
        size,
        1.0,
        packaged_fonts(),
    )
    .unwrap();
    let entry = Entry::new().unwrap();
    renderer
        .prepare_hosted(entry.scene(), true, entry.generation, "", "")
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
        let mut parent = Renderer::hosted(
            gpu.device.clone(),
            gpu.queue.clone(),
            OFFSCREEN,
            size,
            f64::from(scale),
            packaged_fonts(),
        )
        .unwrap();
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
    let mut parent = Renderer::hosted(
        gpu.device.clone(),
        gpu.queue.clone(),
        OFFSCREEN,
        (SLOT[2] as u32, SLOT[3] as u32),
        1.0,
        packaged_fonts(),
    )
    .unwrap();
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
