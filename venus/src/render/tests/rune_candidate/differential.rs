//! Generated active-terminal parity. The ordinary candidate suite checks preparation on a
//! noop device. The ignored tests use exact RGBA readback and shrink reproducible failures.
//! See venus/docs/DEVELOPMENT.md for the default campaign, smoke, soak and artifact settings.

use super::{
    canonical::{announce, cell, chunk, column, frame, frame_message, placement, popup_snapshot},
    *,
};
use crate::{DrawRow, EonBar, ImageResource, ModelEffect, SessionModel, WorkspaceModel};
use eon_workspace_protocol::v51 as wire;
use orbit_protocol::{
    CellWidth, CursorShape, CursorViewport, Dimensions, Frame, Graphics, PlacementState, Screen,
    SourceRect, StyleColor,
    session::{PixelFormat, ServerMessage},
};
use std::path::{Path, PathBuf};

const ENDPOINTS: [&[u8]; 3] = [b"/run/eon/p1.sock", b"/run/eon/p2.sock", b"/run/agent.sock"];
const INK: [u8; 4] = [240, 19, 200, 255];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    Hold,
    Text(usize),
    Cursor(usize),
    Image(Option<i32>),
    Resize(bool),
    Split(usize),
    Zoom(bool),
    Tab(bool),
    Popup(bool),
}

type Step = (usize, Op);
type Size = (u32, u32);
type PixelDifference = (Size, [[u8; 4]; 2], usize);

fn blank() -> orbit_protocol::Cell {
    cell("", CellWidth::Narrow, StyleColor::None, "")
}

fn trace(mut seed: u64, count: usize) -> Vec<Step> {
    let scripted = if seed == 0 { 16 } else { 0 };
    (0..count)
        .map(|id| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let choice = (seed >> 32) as usize;
            let row = (choice >> 8) % 4;
            let kinds = [
                Op::Text(row),
                Op::Hold,
                Op::Cursor(row),
                Op::Image(Some(14)),
                Op::Image(Some(6)),
                Op::Image(None),
                Op::Resize(true),
                Op::Split(1),
                Op::Zoom(true),
                Op::Zoom(false),
                Op::Split(0),
                Op::Resize(false),
                Op::Tab(true),
                Op::Popup(true),
                Op::Popup(false),
                Op::Tab(false),
            ];
            let roll = choice % kinds.len();
            (id, kinds[if id < scripted { id } else { roll }])
        })
        .collect()
}

struct Model {
    snapshot: wire::Snapshot,
    workspace: WorkspaceModel,
    terminals: [SessionModel; 3],
    grid: Frame,
    image: Option<ImageResource>,
    zoom: bool,
}

impl Model {
    fn new() -> Self {
        let mut snapshot = popup_snapshot();
        snapshot.sidebar = wire::Sidebar::Closed;
        snapshot.tabs[0].selected_popup = None;
        let mut split = snapshot.tabs[0].clone();
        split.id = "t2".into();
        split.popups.clear();
        split.selected_pane = Some("p1".into());
        split.columns = vec![column("p1"), column("p2")];
        snapshot.active_tab = split.id.clone();
        snapshot.tabs.push(split);
        snapshot.workspaces[0].tab_count = snapshot.tabs.len();
        let mut grid = frame(1, Screen::Primary);
        grid.dimensions = Dimensions { cols: 16, rows: 8 };
        grid.rows[0].cells.resize_with(16, blank);
        grid.rows.resize(8, grid.rows[0].clone());
        let terminals = std::array::from_fn(|index| {
            let mut model = SessionModel::new();
            let mut initial = grid.clone();
            initial.colors.background.r += index as u8 * 17;
            model.apply(ServerMessage::Attached).unwrap();
            model.apply(frame_message(initial)).unwrap();
            model
        });
        Self {
            snapshot,
            workspace: WorkspaceModel::default(),
            terminals,
            grid,
            image: None,
            zoom: false,
        }
    }

    fn place_image(&mut self, id: u64, column: Option<i32>) {
        let terminal = &mut self.terminals[0];
        let Some(col) = column else {
            self.grid.graphics = None;
            if let Some(image) = self.image.take() {
                terminal
                    .apply(ServerMessage::ResourceRetire {
                        resource: image.resource,
                    })
                    .unwrap();
            }
            return;
        };
        if self.image.is_none() {
            terminal
                .apply(announce(id, PixelFormat::Rgba, 8, 8))
                .unwrap();
            let effect = terminal.apply(chunk(id, 0, INK.repeat(64))).unwrap();
            if let Some(ModelEffect::Resource(image)) = effect {
                self.image = Some(image);
            }
        }
        let resource = self.image.as_ref().unwrap().resource;
        let mut entry = placement(resource, col, 1, PlacementState::Ready);
        entry.source = SourceRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        };
        (entry.offset_x, entry.offset_y) = (0, 0);
        self.grid.graphics = Some(Graphics {
            cell_width: 10,
            cell_height: 20,
            placements_omitted: false,
            placements: vec![entry],
        });
    }

    fn edit(&mut self, (id, op): Step) {
        let grid = &mut self.grid;
        match op {
            Op::Text(row) => {
                let cells = &mut grid.rows[row].cells;
                cells[4 + id % 5].text = ["A", "g", "m", "7"][row].into();
                cells[0].style.bold = !cells[0].style.bold;
                cells[0].style.foreground = StyleColor::Palette(2);
                let wide = &mut cells[1].text;
                *wide = if wide == "界" { "語" } else { "界" }.into();
            }
            Op::Cursor(row) => {
                grid.cursor.viewport = Some(CursorViewport {
                    x: 7,
                    y: row as u16,
                    at_wide_tail: false,
                });
                grid.cursor.shape = CursorShape::Underline;
                grid.cursor.blinking = !grid.cursor.blinking;
            }
            Op::Image(col) => self.place_image(id as u64 + 1, col),
            Op::Resize(large) => {
                let cols = if large { 12 } else { 16 };
                grid.dimensions.cols = cols;
                for row in &mut grid.rows {
                    row.cells.resize_with(cols.into(), blank);
                }
            }
            _ => unreachable!("workspace operation"),
        }
        self.grid.revision += 1;
        let frame = frame_message(self.grid.clone());
        self.terminals[0].apply(frame).unwrap();
    }

    fn step(&mut self, step: Step) -> (Option<usize>, Size, f32) {
        match step.1 {
            Op::Hold => {}
            Op::Split(index) => {
                self.snapshot.tabs[1].selected_pane = Some(format!("p{}", index + 1))
            }
            Op::Zoom(zoom) => self.zoom = zoom,
            Op::Tab(popup) => self.snapshot.active_tab = if popup { "t1" } else { "t2" }.into(),
            Op::Popup(show) => self.snapshot.tabs[0].selected_popup = show.then(|| "u1".into()),
            _ => self.edit(step),
        }
        let response = wire::Response::Snapshot(self.snapshot.clone());
        let bytes = wire::encode_response(&response).unwrap();
        self.workspace.apply(wire::decode_response(&bytes).unwrap());
        let selected = self.workspace.active_attachment().map(|(endpoint, live)| {
            assert!(live);
            ENDPOINTS
                .into_iter()
                .position(|known| known == endpoint)
                .unwrap()
        });
        let (outer, scale) = if self.grid.dimensions.cols == 12 {
            ((601, 461), 2.0)
        } else {
            ((512, 416), 1.0)
        };
        let layout = WorkspaceScene::from_snapshot_with_zoom(
            self.workspace.snapshot().unwrap(),
            EonBar::default(),
            PhysicalSize::from(outer),
            CellMetrics::for_scale(f64::from(scale)),
            0.0,
            0.0,
            |_, label| (label.into(), label.len() as f32 * 10.0),
            None,
            self.zoom,
            Default::default(),
        );
        // Inward-round in device pixels, then normalize both targets to the slot's origin.
        let size = layout
            .visible_terminal()
            .map(|rect| {
                let extent = |lo: f32, hi: f32, cap| ((hi.floor() - lo.ceil()) as u32).min(cap);
                (
                    extent(rect.left, rect.right(), 320),
                    extent(rect.top, rect.bottom(), 192),
                )
            })
            .filter(|&(w, h)| w > 0 && h > 0);
        assert_eq!(size.is_some(), selected.is_some());
        (selected, size.unwrap_or((0, 0)), scale)
    }
}

#[derive(Debug, Default)]
struct Prepared {
    clip: Option<[u32; 4]>,
    rows: Vec<(usize, DrawRow, bool)>,
    bounds: Vec<TextBounds>,
    images: Vec<Vec<BandItem>>,
    quads: Vec<u8>,
}

impl Prepared {
    fn read(renderer: &Renderer, before: &HashMap<RowTextSlot, Arc<ShapedRow>>) -> Self {
        let size = renderer.size();
        let mut rows = Vec::new();
        for (key, row) in &renderer.row_text.rows {
            let reused = match (&row.shaped, before.get(key)) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            };
            rows.push((key.row, row.content.clone(), reused));
        }
        rows.sort_by_key(|row| row.0);
        Self {
            clip: Some([0, 0, size.width, size.height]),
            rows,
            bounds: text_areas(&renderer.text, renderer.text_overlay.as_ref())
                .map(|area| area.bounds)
                .collect(),
            images: renderer.content_key.as_ref().unwrap().images.clone(),
            quads: renderer.differential_image_vertices.clone(),
        }
    }

    fn difference(&self, other: &Self) -> Option<&'static str> {
        [
            ("clip", self.clip == other.clip),
            ("rows", self.rows == other.rows),
            ("text bounds", self.bounds == other.bounds),
            ("image bands", self.images == other.images),
            ("image vertices", self.quads == other.quads),
        ]
        .into_iter()
        .find_map(|(key, same)| (!same).then_some(key))
    }
}

fn prepare(
    renderer: &mut Renderer,
    scene: &Scene,
    image: Option<&ImageResource>,
    generation: u64,
    blink: bool,
    hosted: bool,
) -> Prepared {
    // Only the active terminal keeps an image texture.
    let key = image.map(|image| ImageKey {
        attachment: image.attachment,
        resource: image.resource,
    });
    let refs = key.into_iter().collect();
    let live = key.map(|key| key.attachment).into_iter().collect();
    renderer.sweep_images(live, HashSet::new(), &refs);
    if let Some(image) = image.filter(|_| renderer.image_resident() == 0) {
        renderer.upload_image(image.clone(), &refs).unwrap();
    }
    assert_eq!(renderer.image_resident(), 256 * refs.len() as u64);
    let before = renderer
        .row_text
        .rows
        .iter()
        .filter_map(|(key, row)| Some((*key, row.shaped.clone()?)))
        .collect();
    if hosted {
        renderer
            .prepare_hosted(scene, (None, 0.0), (blink, 0.0), generation, "", "")
            .unwrap();
    } else {
        prepare_parent(renderer, scene, generation, blink);
    }
    Prepared::read(renderer, &before)
}

type Inset = Option<(f32, f32, f32)>;

fn view(inset: &Inset, _: &mut rune::Cx<Inset>) -> rune::El {
    let grid = rune_host::grid_view();
    let Some((w, h, one)) = *inset else {
        return grid;
    };
    let clip = rune::el()
        .absolute(0.0, 0.0)
        .size(w - one, h)
        .clip_rounded(0.0);
    rune::el()
        .w_full()
        .h_full()
        .child(clip.child(grid.absolute(0.0, 0.0).size(w, h)))
}

struct FailurePair {
    index: usize,
    step: Step,
    state: [Prepared; 2],
    pixels: [Vec<u8>; 2],
    size: Size,
    // Coordinate, shipping/candidate RGBA and total unequal pixels.
    pixel: Option<PixelDifference>,
}

impl FailurePair {
    fn signature(&self) -> (Option<&'static str>, bool) {
        let [a, b] = &self.state;
        (a.difference(b), self.pixel.is_some())
    }

    fn repeats(&self, first: &Self, steps: &[Step]) -> bool {
        self.index + 1 == steps.len()
            && self.step == first.step
            && self.signature() == first.signature()
    }

    fn report(&self, seed: u64, original: usize, steps: &[Step], directory: &Path) -> String {
        let paths = ["venus.png", "rune.png"].map(|name| directory.join(name));
        let writes = [0, 1].map(|side| {
            let path = &paths[side];
            rune::render::write_png(path, self.size.0, self.size.1, &self.pixels[side])
                .map_err(|error| format!("{}: {error}", path.display()))
        });
        format!(
            "seed={seed} original={original} reduced={} steps={steps:?} state={:?} pixel={:?}\nvenus={:?}\nrune={:?}\nPNG paths={paths:?} writes={writes:?}",
            self.index,
            self.signature().0,
            self.pixel,
            self.state[0],
            self.state[1]
        )
    }
}

pub(super) fn pixel_difference(size: Size, pixels: [&[u8]; 2]) -> Option<PixelDifference> {
    let [a, b] = pixels.map(|bytes| {
        assert_eq!(bytes.len(), (size.0 * size.1 * 4) as usize);
        bytes.as_chunks::<4>().0
    });
    let mut unequal = a.iter().zip(b).enumerate().filter(|(_, (a, b))| a != b);
    let (index, (a, b)) = unequal.next()?;
    let at = (index as u32 % size.0, index as u32 / size.0);
    Some((at, [*a, *b], 1 + unequal.count()))
}

fn replay(
    gpu: Option<&rune::render::Gpu>,
    noop: Option<&(wgpu::Device, wgpu::Queue)>,
    steps: &[Step],
    inject: Option<usize>,
    coverage: bool,
) -> Option<FailurePair> {
    let mut model = Model::new();
    let (device, queue, fonts): (_, _, fn() -> FontSetup) = if let Some(gpu) = gpu {
        (&gpu.device, &gpu.queue, packaged_fonts)
    } else {
        let (device, queue) = noop.unwrap();
        (device, queue, clipping_test_fonts)
    };
    let renderer = || {
        let (device, queue) = (device.clone(), queue.clone());
        Renderer::hosted(device, queue, OFFSCREEN, (1, 1), 1.0, fonts()).unwrap()
    };
    let [mut parent, mut candidate] = [renderer(), renderer()];
    let mut ui = gpu.map(rune_renderer);
    let mut rt = rune::Runtime::new(None, view, (1.0, 1.0), rune::Env::new(true, 1.0));
    rt.set_srgb_blend(false);
    let mut targets = None;
    let mut previous = SlotGeometry::new([0.0, 0.0, 1.0, 1.0], 1.0);
    for (index, &step) in steps.iter().enumerate() {
        let (selected, size, scale) = model.step(step);
        let scene = selected.and_then(|index| model.terminals[index].scene());
        let image = model.image.as_ref().filter(|_| selected == Some(0));
        let visible = scene.is_some();
        let (w, h) = if visible { size } else { previous.size() };
        let geometry = SlotGeometry::new([0.0, 0.0, w as f32, h as f32], scale);
        let generation = scene.map_or(0, |scene| scene.revision * 4 + selected.unwrap() as u64);
        let blink = !matches!(step.1, Op::Cursor(_));
        let injected = inject == Some(step.0);
        let resized = previous.size() != (w, h);
        if previous != geometry {
            parent.resize(PhysicalSize::new(w, h), f64::from(scale));
            rune_host::resize_to(&mut candidate, geometry);
            previous = geometry;
        }
        let [ship, mut hosted] = if let Some(scene) = scene {
            [
                prepare(&mut parent, scene, image, generation, blink, false),
                prepare(&mut candidate, scene, image, generation, blink, true),
            ]
        } else {
            for renderer in [&mut parent, &mut candidate] {
                renderer.sweep_images(HashSet::new(), HashSet::new(), &HashSet::new());
                assert_eq!(renderer.image_resident(), 0);
            }
            Default::default()
        };
        let pixels = if let Some(gpu) = gpu {
            if resized {
                targets = None;
            }
            let target = |()| rune::render::Offscreen::new(device, w, h);
            let [a, b] = targets.get_or_insert_with(|| [(); 2].map(target));
            let points = (w as f32 / scale, h as f32 / scale);
            rt.update(|state, tx| {
                *state = injected.then_some((points.0, points.1, 1.0 / scale));
                tx.instant();
            });
            rt.set_viewport(points, scale);
            rt.frame(0.0);
            let rune = read_candidate(gpu, ui.as_mut().unwrap(), &mut rt, (w, h), b, |f| {
                assert_eq!(rune_host::preflight(f, None).unwrap().0, geometry);
                let draw =
                    rune_host::draw_slot(f, visible.then_some((&candidate, geometry)), Fit::Exact);
                assert_eq!(
                    (draw.hook_calls, draw.refusal, draw.moved, draw.slot),
                    (1, None, None, visible.then_some(geometry))
                );
                hosted.clip = draw.clip.map(|[l, t, r, b]| {
                    let edge = |value: f32, max| (value as u32).min(max);
                    [
                        edge(l.floor(), w),
                        edge(t.floor(), h),
                        edge(r.ceil(), w),
                        edge(b.ceil(), h),
                    ]
                });
            });
            candidate.end_frame_hosted();
            [read_parent(gpu, &mut parent, a, visible), rune]
        } else {
            let cut = u32::from(injected);
            hosted.clip = hosted.clip.map(|[l, t, r, b]| [l, t, r - cut, b]);
            [Vec::new(), Vec::new()]
        };
        let pixel = gpu.and_then(|_| pixel_difference((w, h), [&pixels[0], &pixels[1]]));
        if ship.difference(&hosted).is_some() || pixel.is_some() {
            return Some(FailurePair {
                index,
                step,
                state: [ship, hosted],
                pixels,
                size: (w, h),
                pixel,
            });
        }
        match (coverage, step.0) {
            (true, 1) => assert!(
                ship.rows.iter().any(|row| row.2),
                "unchanged frame reuses rows"
            ),
            (true, 3 | 4) => {
                assert!(!ship.quads.is_empty());
                assert!(gpu.is_none() || pixels[0].as_chunks::<4>().0.contains(&INK));
            }
            (true, 5) => assert!(ship.quads.is_empty()),
            (true, 6) => {
                let wide = ship.rows[0]
                    .1
                    .cells
                    .iter()
                    .any(|cell| cell.width == CellWidth::Wide);
                assert!(scale == 2.0 && ship.rows.len() > 1 && wide);
            }
            (true, 7) => {
                assert_eq!(selected, Some(1));
                assert!(w % 2 == 1, "odd split extent, got {size:?}");
            }
            (true, 8) => assert!(resized, "zoom changes the split extent"),
            (true, 12 | 14) => {
                assert_eq!(selected, None);
                assert!(
                    pixels[0]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|color| *color == [0, 0, 0, 255])
                );
            }
            (true, 13) => assert_eq!(selected, Some(2)),
            _ => {}
        }
    }
    None
}

fn shrink(
    steps: &[Step],
    target: &FailurePair,
    mut replay: impl FnMut(&[Step]) -> Option<FailurePair>,
) -> Vec<Step> {
    let mut reduced = steps[..=target.index].to_vec();
    let mut index = 0;
    for _ in 0..2 * reduced.len() {
        if index + 1 >= reduced.len() {
            return reduced;
        }
        let mut trial = reduced.clone();
        trial.remove(index);
        if replay(&trial).is_some_and(|hit| hit.repeats(target, &trial)) {
            reduced = trial;
            index = 0;
        } else {
            index += 1;
        }
    }
    eprintln!("shrink trial budget exhausted");
    reduced
}

fn headless_gpu() -> rune::render::Gpu {
    use rune::render::Gpu;
    Gpu::try_new(Gpu::instance(), None)
        .expect("Rune parity needs a headless Vulkan or Metal adapter")
}

fn campaign(value: &str) -> (u64, usize) {
    match value {
        "" => (8, 16),
        "smoke" => (2, 64),
        "1" => (256, 64),
        _ => panic!("EON_VENUS_TEST_RUNE_DIFF_SOAK must be empty, smoke, or 1; got {value:?}"),
    }
}

fn artifacts(name: &str) -> PathBuf {
    let root = std::env::var_os("EON_VENUS_TEST_RUNE_DIFF_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let now = std::time::SystemTime::UNIX_EPOCH.elapsed().unwrap();
    root.join(format!("{name}-{}-{}", std::process::id(), now.as_nanos()))
}

#[test]
fn rune_differential_noop() {
    let noop = wgpu::Device::noop(&DeviceDescriptor::default());
    let (seeds, count) = campaign("");
    let mut unique = HashSet::new();
    for seed in 0..seeds {
        let steps = trace(seed, count);
        assert_eq!(steps, trace(seed, count));
        if seed < 2 {
            assert!(replay(None, Some(&noop), &steps, None, seed == 0).is_none());
        } else {
            let mut model = Model::new();
            for &step in &steps {
                model.step(step);
            }
        }
        assert!(unique.insert(steps));
    }
    let steps = trace(0, 2);
    let run = |steps: &[Step]| replay(None, Some(&noop), steps, Some(1), false);
    let first = run(&steps).unwrap();
    assert_eq!(first.signature(), (Some("clip"), false));
    assert_eq!(shrink(&steps, &first, run), vec![steps[1]]);
}

#[test]
#[ignore = "needs a real headless GPU and packaged fonts; see venus/docs/DEVELOPMENT.md"]
fn rune_differential_default_or_soak() {
    let gpu = headless_gpu();
    eprintln!("adapter: {:?}", gpu.adapter.get_info());
    let mode = std::env::var_os("EON_VENUS_TEST_RUNE_DIFF_SOAK").unwrap_or_default();
    let mode = mode.to_str().expect("invalid soak mode value");
    let (seeds, count) = campaign(mode);
    for seed in 0..seeds {
        let steps = trace(seed, count);
        let run = |steps: &[Step]| replay(Some(&gpu), None, steps, None, false);
        if let Some(first) = replay(Some(&gpu), None, &steps, None, seed == 0) {
            let reduced = shrink(&steps, &first, run);
            let confirmed = run(&reduced);
            let directory = artifacts(&format!("rune-diff-{seed}"));
            let (failure, reported) = match &confirmed {
                Some(hit) if hit.repeats(&first, &reduced) => (hit, reduced.as_slice()),
                _ => {
                    eprintln!("unstable reduced replay");
                    (&first, &steps[..=first.index])
                }
            };
            let report = failure.report(seed, first.index, reported, &directory);
            panic!("{report}");
        }
    }
    let pairs = seeds as usize * count;
    eprintln!("checked {pairs} frame pairs ({seeds} seeds × {count} steps)");
}

#[test]
#[ignore = "needs a real headless GPU and packaged fonts; see venus/docs/DEVELOPMENT.md"]
fn rune_differential_reports_injected_divergence() {
    let gpu = headless_gpu();
    let seed = 0;
    let steps = trace(seed, 16);
    assert!(replay(Some(&gpu), None, &steps, None, true).is_none());
    let run = |steps: &[Step]| replay(Some(&gpu), None, steps, Some(3), false);
    let first = run(&steps).unwrap();
    let reduced = shrink(&steps, &first, run);
    assert!(reduced.len() < first.index + 1);
    let hit = run(&reduced).unwrap();
    let (w, h) = hit.size;
    assert!(hit.repeats(&first, &reduced));
    assert_eq!(hit.signature(), (Some("clip"), true));
    assert_eq!(hit.state[1].clip, Some([0, 0, w - 1, h]));
    assert_eq!(
        hit.pixel.map(|(at, _, count)| (at, count)),
        Some(((w - 1, 0), h as usize))
    );
    let directory = artifacts("rune-diff-injected");
    let report = hit.report(seed, first.index, &reduced, &directory);
    drop((hit, first));
    let [a, b] = ["venus.png", "rune.png"]
        .map(|file| rune::text::pixel_compare::Image::read_png(&directory.join(file)));
    std::fs::remove_dir_all(directory).unwrap();
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!((a.width, a.height, b.width, b.height), (w, h, w, h));
    let [a, b] = [a.rgb.as_slice(), b.rgb.as_slice()].map(|rgb| rgb.as_chunks::<3>().0);
    assert_eq!((a.len(), b.len()), ((w * h) as usize, (w * h) as usize));
    for (index, (a, b)) in a.iter().zip(b).enumerate() {
        assert_eq!(a != b, (index + 1).is_multiple_of(w as usize));
    }
    for expected in [
        "seed=0 original=3 reduced=0",
        "state=Some(\"clip\")",
        "pixel=Some",
        "venus.png",
        "rune.png",
        "writes=[Ok(()), Ok(())]",
    ] {
        assert!(report.contains(expected), "{report}");
    }
    eprintln!("seed={seed} original=3 reduced={reduced:?}: one-pixel clip divergence");
}
