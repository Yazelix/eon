//! Private Rune host candidate (eon-yp6vg.7.7): the canonical Venus grid drawn by the hosted
//! renderer inside one Rune device and frame pass. Only the non-default `candidate-rune`
//! feature builds it, and only the `venus-rune-candidate` binary runs it.
//!
//! It draws canonical fixtures (`fixtures`), never a live Orbit attachment. Limits kept for
//! later children of eon-yp6vg.7:
//! - keyboard, pointer and IME are bound in the live entry (eon-yp6vg.7.2);
//! - no supervisor, ready-v1 or Present/EOF here: the supervised entry runs Venus's own
//!   `Application` on Rune (`application::rune_host`, eon-yp6vg.7.9);
//! - a new GPU device or target format is terminal, not rebuilt (eon-yp6vg.7.10);
//! - no Rune accessibility tree for the terminal (the VEN-C1 Rune-hosted gap);
//! - the fixture cursor stays visible and uses a static cursor.

pub(crate) mod fixtures;
pub(crate) mod header_pilot;

#[cfg(test)]
pub(crate) use fixtures::messages as fixture_messages;

use crate::{
    CellMetrics, ImageKey, ModelEffect, Renderer, Scene, SessionModel, WorkspaceScene,
    render::{FontSettings, FontSetup, HOSTED_LOOK, HostedLook, PresentOutcome, RenderError},
};
use orbit_protocol::session::{decode_server_message, encode_server_message};
use std::{
    collections::HashSet,
    error::Error,
    io::BufRead,
    sync::{Arc, Mutex},
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
type WorkspaceKey = (winit::dpi::PhysicalSize<u32>, CellMetrics, usize);

/// Which host draws the fixtures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// One Rune window; the grid draws at an effect slot in Rune's own pass.
    Rune,
    /// Venus's own windowed draw path, with the hosted renderer's fixed look: the matched
    /// parent the native pixel proof compares against.
    Venus,
}

/// Steps per cycle.
pub(crate) const STEP_COUNT: usize = fixtures::STEPS.len();

/// The Rune effect slot's tag and key.
const GRID_MARK: u32 = 1;
const GRID_KEY: &str = "grid";
/// Rune's frame clear, outside the slot. The slot paints its own backdrop.
const RUNE_CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Runs `host` until its window closes. Errors are render, GPU, geometry or admission
/// failures.
pub fn run(host: Host) -> Result {
    run_fixture(host, false)
}

/// Runs the private measured popup fixture.
pub fn run_header_pilot(host: Host) -> Result {
    run_fixture(host, true)
}

fn run_fixture(host: Host, pilot: bool) -> Result {
    validate_fixtures()?;
    // The Darwin package sets this list; Linux gets its fonts from FONTCONFIG_FILE.
    let mut fonts = FontSetup::with_directories(
        &FontSettings::default(),
        std::env::var_os("EON_VENUS_FONT_DIRS").as_deref(),
    )?;
    let (look, family) = if pilot {
        let path = std::env::var_os("EON_VENUS_TEST_SANS_FONT");
        (
            header_pilot::look()?,
            Some(header_pilot::fonts(&mut fonts, host == Host::Rune, path)?),
        )
    } else {
        (HOSTED_LOOK, None)
    };
    match host {
        Host::Rune => rune_host::run(fonts, look, family),
        Host::Venus => venus_host::run(fonts, look, pilot),
    }
}

/// Fails before a window opens when a fixture does not reach the model; afterwards every
/// step is known to apply.
fn validate_fixtures() -> Result {
    let mut entry = Entry::new()?;
    for _ in 1..STEP_COUNT * 2 {
        entry.try_advance()?;
    }
    Ok(())
}

/// The entry's state machine over the fixture steps, shared by both hosts.
pub(crate) struct Entry {
    cycle: u64,
    step: usize,
    model: SessionModel,
    /// Increments with every applied step: Venus rebuilds prepared content on a new one.
    pub(crate) generation: u64,
    /// Image effects in message order, for the renderer to apply before the next draw.
    pending: Vec<ModelEffect>,
    header_workspace: Option<(WorkspaceKey, WorkspaceScene)>,
}

impl Entry {
    pub(crate) fn new() -> Result<Self> {
        let mut entry = Self {
            cycle: 0,
            step: 0,
            model: SessionModel::new(),
            generation: 0,
            pending: Vec::new(),
            header_workspace: None,
        };
        entry.apply_step()?;
        Ok(entry)
    }

    /// The next step; after the last, the first again as a fresh attachment.
    pub(crate) fn advance(&mut self) {
        self.try_advance()
            .expect("the fixtures were validated before the window opened");
    }

    fn try_advance(&mut self) -> Result {
        self.step += 1;
        if self.step == STEP_COUNT {
            self.step = 0;
            self.cycle += 1;
            self.model = SessionModel::new();
        }
        self.apply_step()
    }

    fn apply_step(&mut self) -> Result {
        for message in fixtures::messages(self.step) {
            let message = decode_server_message(&encode_server_message(&message)?)?;
            if let Some(effect) = self.model.apply(message)? {
                self.pending.push(effect);
            }
        }
        self.generation += 1;
        Ok(())
    }

    pub(crate) fn scene(&self) -> &Scene {
        self.model
            .scene()
            .expect("every fixture step ends with a frame")
    }

    fn title_step(&self) -> usize {
        match self.step {
            0 | 1 => 0,
            2 => 2,
            _ => 3,
        }
    }

    fn prepare_header_workspace(&mut self, renderer: &mut Renderer) {
        let title_step = self.title_step();
        let key = (renderer.size(), renderer.metrics(), title_step);
        if self
            .header_workspace
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key)
        {
            self.header_workspace = Some((key, fixtures::header_workspace(renderer, title_step)));
        }
    }

    fn prepare_hosted_header(
        &mut self,
        renderer: &mut Renderer,
    ) -> std::result::Result<(), RenderError> {
        self.sync_images(renderer);
        self.prepare_header_workspace(renderer);
        let (_, workspace) = self.header_workspace.as_ref().unwrap();
        renderer.prepare_hosted_workspace(self.scene(), workspace, self.generation)
    }

    /// (cycle, step, name) of the current step.
    #[cfg(test)]
    pub(crate) fn position(&self) -> (u64, usize, &'static str) {
        (self.cycle, self.step, fixtures::STEPS[self.step])
    }

    /// Applies the pending image effects, then frees what the current attachment and frame
    /// no longer keep, as the application does before a redraw (VEN-C31). An upload the
    /// renderer refuses leaves its marker and the renderer working.
    pub(crate) fn sync_images(&mut self, renderer: &mut Renderer) {
        for effect in std::mem::take(&mut self.pending) {
            let referenced = self.sweep(renderer);
            match effect {
                ModelEffect::Resource(resource) => {
                    if let Err(error) = renderer.upload_image(resource, &referenced) {
                        eprintln!("venus-rune-candidate: image not shown: {error}");
                    }
                }
                ModelEffect::Retired(key) => renderer.retire_image(key),
                ModelEffect::Clipboard(_) => {}
            }
        }
        self.sweep(renderer);
    }

    fn sweep(&self, renderer: &mut Renderer) -> HashSet<ImageKey> {
        let attachment = self.model.image_attachment();
        let referenced: HashSet<ImageKey> = attachment
            .into_iter()
            .flat_map(|attachment| {
                self.scene()
                    .graphics
                    .placements
                    .iter()
                    .filter(move |_| self.scene().graphics.attachment == attachment)
                    .map(move |placement| ImageKey {
                        attachment,
                        resource: placement.resource,
                    })
            })
            .collect();
        renderer.sweep_images(
            attachment.into_iter().collect(),
            HashSet::new(),
            &referenced,
        );
        referenced
    }
}

/// How close to where it was prepared a slot must draw for the prepared grid to draw in it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fit {
    /// Exactly there: the fixtures' window never resizes, so any move is a fault.
    Exact,
    /// Within a device pixel ([`SlotGeometry::same_slot`]): the Venus host, whose window the
    /// compositor resizes.
    Near,
}

/// A slot's device-pixel rectangle and scale, compared after rounding: Rune snaps slot rects
/// to the pixel grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotGeometry {
    pub(crate) rect: [i64; 4],
    pub(crate) scale_bits: u32,
}

impl SlotGeometry {
    pub(crate) fn new(rect: [f32; 4], scale: f32) -> Self {
        Self {
            rect: rect.map(|value| value.round() as i64),
            scale_bits: scale.to_bits(),
        }
    }

    pub(crate) fn size(self) -> (u32, u32) {
        (self.rect[2].max(1) as u32, self.rect[3].max(1) as u32)
    }

    pub(crate) fn scale(self) -> f32 {
        f32::from_bits(self.scale_bits)
    }

    /// Whether a slot drawn at `self` is the one prepared at `prepared`: the same scale, and
    /// each edge within one device pixel, since the layout rect in points and the drawn rect in
    /// device pixels round separately.
    pub(crate) fn same_slot(self, prepared: SlotGeometry) -> bool {
        self.scale_bits == prepared.scale_bits
            && self
                .rect
                .iter()
                .zip(prepared.rect)
                .all(|(at, prepared)| at.abs_diff(prepared) <= 1)
    }

    /// Where a grid prepared at `self` draws in a slot that drew at `at`, as a viewport in a
    /// pass whose top-left is `origin` and whose size is `target`. It keeps the prepared size,
    /// so cells stay pixel-exact; a slot within a pixel of it leaves at most a strip of clear.
    /// Under `Fit::Near` a grid a pixel past the target's far edge shifts back inside. None when
    /// the slot has moved further than `fit` allows, or the prepared grid would leave the target.
    pub(crate) fn viewport(
        self,
        at: SlotGeometry,
        fit: Fit,
        origin: [f32; 2],
        target: [u32; 2],
    ) -> Option<[f32; 4]> {
        let near = match fit {
            Fit::Exact => at == self,
            Fit::Near => at.same_slot(self),
        };
        let (width, height) = self.size();
        let size = [width as f32, height as f32];
        let mut corner = [at.rect[0] as f32 - origin[0], at.rect[1] as f32 - origin[1]];
        if let Fit::Near = fit {
            // The prepared size and the drawn rect round separately, so a grid at the pass's far
            // edge can come out a pixel past it on every frame; it shifts back by that pixel
            // instead of counting as a move. A real resize still leaves it outside.
            for ((corner, size), end) in corner.iter_mut().zip(size).zip(target) {
                *corner = corner.min(end as f32 - size).max(*corner - 1.0);
            }
        }
        let [left, top] = corner;
        let inside = left >= 0.0
            && top >= 0.0
            && left + size[0] <= target[0] as f32
            && top + size[1] <= target[1] as f32;
        (near && inside).then_some([left, top, size[0], size[1]])
    }

    /// The refusal a slot that keeps drawing away from where it was prepared ends with.
    pub(crate) fn moved_from(self, prepared: SlotGeometry) -> String {
        format!("the slot drew at {self:?} but was prepared at {prepared:?}")
    }
}

/// The first failure, carried out of the event loop to the exit status.
#[derive(Clone, Default)]
pub(crate) struct Failure(Arc<Mutex<Option<String>>>);

impl Failure {
    pub(crate) fn set(&self, message: impl Into<String>) {
        let mut slot = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if slot.is_none() {
            *slot = Some(message.into());
        }
    }

    pub(crate) fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn into_result(self) -> Result {
        match self.get() {
            Some(message) => Err(message.into()),
            None => Ok(()),
        }
    }
}

/// Reads stdin on its own thread: each line advances one step. Closed stdin stops it, and
/// the entry stays where it is.
fn read_advances(mut advance: impl FnMut() + Send + 'static) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if line.is_err() {
                break;
            }
            advance();
        }
    });
}

/// The line each host prints once a step's frame is presented.
type Receipt = (u64, usize, Option<SlotGeometry>);

fn report_drawn((cycle, step, geometry): Receipt, follow_ups: u64) {
    let name = fixtures::STEPS[step];
    if let Some(geometry) = geometry {
        let (width, height) = geometry.size();
        let scale = geometry.scale();
        println!("drawn {cycle} {step} {name} {width} {height} {scale} {follow_ups}");
    } else {
        println!("drawn {cycle} {step} {name}");
    }
}

pub(crate) mod rune_host {
    use super::*;
    use rune::{
        App, AppHandle, AppSpec, Compositor, Cx, El, Frame, compose::DeviceKey, effect, el,
        render::EffectSite,
    };

    pub(crate) type FrameTarget = (DeviceKey, wgpu::TextureFormat);

    /// Rune's app state: the entry, and the step the compositor last drew.
    pub(crate) struct State {
        pub(crate) entry: Entry,
        pub(crate) drawn: Option<Receipt>,
        pub(crate) follow_ups: u64,
        pub(crate) header: Option<(&'static str, Option<header_pilot::Title>)>,
        renderer: Option<Renderer>,
        early_workspace: Option<std::result::Result<(), RenderError>>,
    }

    impl State {
        pub(crate) fn new(entry: Entry, family: Option<&'static str>) -> Self {
            Self {
                entry,
                drawn: None,
                follow_ups: 0,
                header: family.map(|name| (name, None)),
                renderer: None,
                early_workspace: None,
            }
        }

        pub(crate) fn advance(&mut self) {
            let previous = self.entry.title_step();
            self.entry.advance();
            self.early_workspace = None;
            // Fit against the last drawable before Rune builds the next view.
            if previous != self.entry.title_step()
                && let (Some((_, title)), Some(renderer)) = (&mut self.header, &mut self.renderer)
            {
                let prepared = self.entry.prepare_hosted_header(renderer);
                if prepared.is_ok() {
                    *title = renderer.header_paint.title.clone();
                }
                self.early_workspace = Some(prepared);
            }
        }
    }

    pub(crate) fn view(state: &State, _: &mut Cx<State>) -> El {
        let grid = grid_view();
        match &state.header {
            Some((family, title)) => grid
                .child(header_pilot::marker())
                .children(title.iter().map(|title| title.view(family))),
            None => grid,
        }
    }

    pub(crate) fn run(fonts: FontSetup, look: HostedLook, family: Option<&'static str>) -> Result {
        let failure = Failure::default();
        let entry = Entry::new()?;
        let mut app = App::new(AppSpec::new(
            "Eon (Rune candidate)",
            if family.is_some() {
                (1280.0, 800.0)
            } else {
                (1000.0, 640.0)
            },
            || unreachable!("App::init supplies the state"),
            view,
        ))
        .args(Vec::new())
        .srgb_blend(false)
        .init(move || State::new(entry, family));
        if family.is_some() {
            app = app.window_options(rune::WindowOptions {
                transparent: look.background_opacity < 1.0,
                min_size: Some((0.0, 0.0)),
                ..Default::default()
            });
        }
        let handle = app.handle();
        let mut printed = None;
        let mut compositor = GridCompositor::new(fonts, failure.clone(), Some(handle.clone()));
        compositor.look = look;
        app = app
            .compositor(compositor)
            .on_frame_drawn(move |rt: &rune::Runtime<State>| {
                let state = &rt.state;
                if let Some(drawn) = state.drawn
                    && printed != Some(drawn)
                {
                    printed = Some(drawn);
                    report_drawn(drawn, state.follow_ups);
                }
            });
        let advancing = handle.clone();
        read_advances(move || advancing.update(|state: &mut State, _| state.advance()));
        app.try_run()?;
        failure.into_result()
    }

    /// Draws the hosted Venus renderer at the grid slot, inside Rune's frame pass.
    pub(crate) struct GridCompositor {
        fonts: Option<FontSetup>,
        target: Option<FrameTarget>,
        prepared: Option<SlotGeometry>,
        failure: Failure,
        quit: Option<AppHandle<State>>,
        pub(crate) look: HostedLook,
        instances: Vec<rune::render::Instance>,
        /// Effect-slot hook calls in the last frame, for the proofs.
        pub(crate) hook_calls: u32,
        /// Whether the last frame's hook drew the grid.
        pub(crate) drew: bool,
    }

    impl GridCompositor {
        pub(crate) fn new(
            fonts: FontSetup,
            failure: Failure,
            quit: Option<AppHandle<State>>,
        ) -> Self {
            Self {
                fonts: Some(fonts),
                target: None,
                prepared: None,
                failure,
                quit,
                look: HOSTED_LOOK,
                instances: Vec::new(),
                hook_calls: 0,
                drew: false,
            }
        }

        fn fail(&mut self, message: String) {
            self.failure.set(message);
            if let Some(quit) = &self.quit {
                quit.quit();
            }
        }

        /// Admits the frame's device and format, and prepares the scene at the slot's
        /// geometry before any pass is open.
        pub(crate) fn prepare(&mut self, f: &mut Frame<'_, State>) -> Option<SlotGeometry> {
            let pilot = f.rt.state.header.is_some();
            if pilot && !f.format.is_srgb() {
                self.fail("the measured header pilot requires an sRGB target".into());
                return None;
            }
            if pilot && self.look.background_opacity < 1.0 && !f.rt.transparent {
                self.fail("the measured header pilot requires a transparent surface".into());
                return None;
            }
            let (geometry, target) = match preflight(f, self.target) {
                Ok(preflight) => preflight,
                Err(message) => {
                    self.fail(message);
                    return None;
                }
            };
            if f.rt.state.renderer.is_none() {
                match hosted_renderer(f, geometry, self.fonts.take()?, self.look) {
                    Ok(mut renderer) => {
                        if pilot {
                            renderer.enable_header_pilot(true);
                        }
                        f.rt.state.renderer = Some(renderer);
                        self.target = Some(target);
                    }
                    Err(message) => {
                        self.fail(message);
                        return None;
                    }
                }
            }
            let state = &mut f.rt.state;
            let renderer = state.renderer.as_mut()?;
            if self.prepared != Some(geometry) {
                resize_to(renderer, geometry);
                state.early_workspace = None;
            }
            let entry = &mut state.entry;
            let result = if pilot {
                state
                    .early_workspace
                    .take()
                    .unwrap_or_else(|| entry.prepare_hosted_header(renderer))
            } else {
                entry.sync_images(renderer);
                renderer.prepare_hosted(
                    entry.scene(),
                    (None, 0.0),
                    (true, f.dt as f32),
                    entry.generation,
                    "",
                    "",
                )
            };
            if let Err(error) = result {
                self.fail(error.to_string());
                return None;
            }
            self.prepared = Some(geometry);
            Some(geometry)
        }
    }

    impl Compositor<State> for GridCompositor {
        fn render(&mut self, f: &mut Frame<'_, State>) {
            f.rt.state.drawn = None;
            let prepared = self.prepare(f);
            let mut follow_up = None;
            let submission = if let Some((_, mounted)) = &f.rt.state.header {
                let paint = prepared
                    .and(f.rt.state.renderer.as_ref())
                    .map(|r| &r.header_paint);
                if let Some(paint) = paint.filter(|paint| &paint.title != mounted) {
                    follow_up = Some(paint.title.clone());
                }
                if !header_pilot::splice(
                    f.rt,
                    paint.filter(|_| follow_up.is_none()),
                    &mut self.instances,
                ) {
                    (self.hook_calls, self.drew) = (0, false);
                    self.fail("the measured header pilot frame is missing instance marks".into());
                    return;
                }
                Some((self.instances.as_slice(), [0.0; 4]))
            } else {
                None
            };
            let mut renderer = f.rt.state.renderer.take();
            let drawn = draw_instances(f, renderer.as_ref().zip(prepared), Fit::Exact, submission);
            self.hook_calls = drawn.hook_calls;
            self.drew = drawn.slot.is_some();
            // The fixtures' window never resizes, so a moved slot stays a refusal here.
            if let Some(refusal) = drawn.refusal.or_else(|| {
                prepared
                    .zip(drawn.moved)
                    .map(|(prepared, at)| at.moved_from(prepared))
            }) {
                self.fail(refusal);
            }
            if let Some(renderer) = renderer.as_mut() {
                renderer.end_frame_hosted();
            }
            f.rt.state.renderer = renderer;
            if let Some(next) = follow_up {
                if f.rt.update_if(|state, _| {
                    state.header.as_mut().unwrap().1 = next;
                    state.follow_ups += 1;
                    true
                }) && let Some(handle) = &self.quit
                {
                    handle.wake();
                }
            } else if self.drew {
                let entry = &f.rt.state.entry;
                let geometry = prepared.filter(|_| f.rt.state.header.is_some());
                f.rt.state.drawn = Some((entry.cycle, entry.step, geometry));
            }
        }
    }

    /// The grid slot's view: one effect slot filling the window. Shared with the supervised
    /// candidate host (eon-yp6vg.7.9).
    pub(crate) fn grid_view() -> El {
        // The grid follows the drawable immediately, including in animated commits.
        el().w_full()
            .h_full()
            .instant_changes()
            .child(effect(GRID_MARK).key(GRID_KEY).w_full().h_full())
    }

    /// Check the frame before making or preparing GPU resources.
    pub(crate) fn preflight<S: 'static>(
        f: &Frame<'_, S>,
        admitted: Option<FrameTarget>,
    ) -> std::result::Result<(SlotGeometry, FrameTarget), String> {
        let target = (f.device_key(), f.format);
        if admitted.is_some_and(|admitted| admitted != target) {
            return Err(format!(
                "the frame moved to another GPU device or target format ({:?})",
                f.format
            ));
        }
        if f.srgb_blend() || f.rt.srgb_blend() {
            return Err("the Venus grid requires a linear-blended frame".into());
        }
        let Some((rect, _)) = f.rt.presented(GRID_KEY) else {
            return Err("the grid slot is not in the frame".into());
        };
        let scale = f.rt.env.scale;
        let geometry = SlotGeometry::new(rect.map(|value| value * scale), scale);
        Ok((geometry, target))
    }

    /// A hosted renderer on the frame's device and format, at the slot's geometry.
    pub(crate) fn hosted_renderer<S: 'static>(
        f: &Frame<'_, S>,
        geometry: SlotGeometry,
        fonts: FontSetup,
        look: HostedLook,
    ) -> std::result::Result<Renderer, String> {
        Renderer::hosted_with_look(
            f.device.clone(),
            f.queue.clone(),
            f.format,
            geometry.size(),
            f64::from(geometry.scale()),
            fonts,
            look,
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn resize_to(renderer: &mut Renderer, geometry: SlotGeometry) {
        let (width, height) = geometry.size();
        renderer.resize(
            winit::dpi::PhysicalSize::new(width, height),
            f64::from(geometry.scale()),
        );
    }

    /// What one frame's slot pass did.
    pub(crate) struct Drawn {
        /// Effect-slot hook calls in the frame, for the proofs.
        pub(crate) hook_calls: u32,
        /// Where the hook drew the grid, if it drew one.
        pub(crate) slot: Option<SlotGeometry>,
        /// Why the prepared grid could not draw.
        pub(crate) refusal: Option<String>,
        /// Where the slot drew when it had moved since it was prepared further than the pass's
        /// `Fit` allows (a resize landing between the two); the grid did not draw.
        pub(crate) moved: Option<SlotGeometry>,
        #[cfg(test)]
        pub(crate) clip: Option<[f32; 4]>,
    }

    /// Runs Rune's frame pass, drawing `prepared`'s renderer at the grid slot when the slot
    /// sits where it was prepared, within `fit`, in the prepared target. With nothing prepared
    /// the frame still draws Rune's clear, so the surface is committed.
    pub(crate) fn draw_slot<S: 'static>(
        f: &mut Frame<'_, S>,
        prepared: Option<(&Renderer, SlotGeometry)>,
        fit: Fit,
    ) -> Drawn {
        draw_instances(f, prepared, fit, None)
    }

    fn draw_instances<S: 'static>(
        f: &mut Frame<'_, S>,
        prepared: Option<(&Renderer, SlotGeometry)>,
        fit: Fit,
        submission: Option<(&[rune::render::Instance], [f32; 4])>,
    ) -> Drawn {
        let (instances, clear) = submission.unwrap_or((&f.rt.instances, RUNE_CLEAR));
        f.ui.prepare(f.device, f.queue, &mut f.rt.text.atlas, instances, f.size);
        let target_format = f.format;
        let mut refusal = None;
        let mut moved = None;
        let mut calls = 0;
        let mut slot = None;
        #[cfg(test)]
        let mut clip = None;
        f.ui.draw_with(
            f.encoder,
            f.target,
            clear,
            &mut |pass, site: &EffectSite<'_>| {
                calls += 1;
                let Some((renderer, prepared)) = prepared else {
                    return;
                };
                if site.mark != GRID_MARK {
                    return;
                }
                let at = SlotGeometry::new(site.instance.rect, site.scale);
                if site.format != target_format || site.in_layer || site.srgb_blend {
                    refusal = Some(format!(
                        "incompatible Venus grid pass ({:?}, in a layer: {}, sRGB blend: {})",
                        site.format, site.in_layer, site.srgb_blend
                    ));
                    return;
                }
                let Some([left, top, width, height]) =
                    prepared.viewport(at, fit, site.origin, site.size)
                else {
                    moved = Some(at);
                    return;
                };
                pass.set_viewport(left, top, width, height, 0.0, 1.0);
                match renderer.draw_hosted(pass) {
                    Ok(()) => slot = Some(at),
                    Err(error) => refusal = Some(error.to_string()),
                }
                #[cfg(test)]
                if slot == Some(at) {
                    clip = Some(site.instance.clip);
                }
            },
        );
        Drawn {
            hook_calls: calls,
            slot,
            refusal,
            moved,
            #[cfg(test)]
            clip,
        }
    }
}

mod venus_host {
    use super::*;
    #[cfg(target_os = "linux")]
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{Window, WindowId},
    };

    struct Parent {
        fonts: Option<FontSetup>,
        entry: Entry,
        window: Option<Arc<Window>>,
        renderer: Option<Renderer>,
        drawn: Option<Receipt>,
        failure: Failure,
        pilot: bool,
        look: HostedLook,
    }

    /// Wakes the loop for one advance.
    struct Advance;

    pub(super) fn run(fonts: FontSetup, look: HostedLook, pilot: bool) -> Result {
        let mut builder = EventLoop::<Advance>::with_user_event();
        // Linux: Wayland only (the binary has already refused without it). macOS: the
        // native AppKit loop, the builder's default.
        #[cfg(target_os = "linux")]
        builder.with_wayland();
        let event_loop = builder.build()?;
        let proxy: EventLoopProxy<Advance> = event_loop.create_proxy();
        read_advances(move || {
            let _ = proxy.send_event(Advance);
        });
        let failure = Failure::default();
        let mut parent = Parent {
            fonts: Some(fonts),
            entry: Entry::new()?,
            window: None,
            renderer: None,
            drawn: None,
            failure: failure.clone(),
            pilot,
            look,
        };
        event_loop.run_app(&mut parent)?;
        failure.into_result()
    }

    impl ApplicationHandler<Advance> for Parent {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let size = if self.pilot {
                (1280.0, 800.0)
            } else {
                (1000.0, 640.0)
            };
            let attributes = Window::default_attributes()
                .with_title("Eon (Venus parent)")
                .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
                .with_transparent(self.look.background_opacity < 1.0);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    self.failure.set(error.to_string());
                    event_loop.exit();
                    return;
                }
            };
            let Some(fonts) = self.fonts.take() else {
                return;
            };
            let look = self.look;
            match pollster::block_on(Renderer::new(
                Arc::clone(&window),
                event_loop,
                look.background_opacity,
                look.padding_color,
                look.cursor_tail,
                look.pane_frames,
                look.panel_heading,
                look.detect_local_hosts,
                look.focus_color,
                look.border_width,
                look.focus_border_width,
                look.animate_working_badge,
                None,
                fonts,
            )) {
                Ok(mut renderer) => {
                    if self.pilot {
                        renderer.enable_header_pilot(false);
                    }
                    self.renderer = Some(renderer);
                }
                Err(error) => {
                    self.failure.set(error.to_string());
                    event_loop.exit();
                    return;
                }
            }
            window.request_redraw();
            self.window = Some(window);
        }

        fn user_event(&mut self, _: &ActiveEventLoop, _: Advance) {
            self.entry.advance();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) {
                        renderer.resize(size, window.scale_factor());
                        window.request_redraw();
                    }
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    if let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) {
                        renderer.resize(window.inner_size(), scale_factor);
                        window.request_redraw();
                    }
                }
                WindowEvent::RedrawRequested => self.redraw(event_loop),
                _ => {}
            }
        }
    }

    impl Parent {
        fn redraw(&mut self, event_loop: &ActiveEventLoop) {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            self.entry.sync_images(renderer);
            if self.pilot {
                self.entry.prepare_header_workspace(renderer);
            }
            let workspace = self
                .entry
                .header_workspace
                .as_ref()
                .map(|(_, workspace)| workspace);
            let outcome = renderer.render(
                Some(self.entry.scene()),
                None,
                0.0,
                workspace,
                &[],
                None,
                crate::WorkspaceFocus::Terminal,
                "",
                true,
                "",
                self.entry.generation,
            );
            match outcome {
                Ok(PresentOutcome::Presented(_)) => {
                    let geometry = self.pilot.then(|| {
                        let size = renderer.size();
                        SlotGeometry::new(
                            [0.0, 0.0, size.width as f32, size.height as f32],
                            renderer.metrics().scale,
                        )
                    });
                    let position = (self.entry.cycle, self.entry.step, geometry);
                    if self.drawn != Some(position) {
                        self.drawn = Some(position);
                        report_drawn(position, 0);
                    }
                }
                Ok(_) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Err(error) => {
                    self.failure.set(error.to_string());
                    event_loop.exit();
                }
            }
        }
    }
}

/// Runs a compositor on a private target; only capture frames need a readback.
#[cfg(test)]
pub(crate) fn offscreen_frame<S: 'static>(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    ui: &mut rune::render::Renderer,
    rt: &mut rune::Runtime<S>,
    (size, target): ((u32, u32), &rune::render::Offscreen),
    (dt, readback): (f64, bool),
    render: impl FnOnce(&mut rune::Frame<'_, S>),
) -> Option<Vec<u8>> {
    let format = ui.format();
    let mut encoder = device.create_command_encoder(&Default::default());
    render(&mut rune::Frame {
        device,
        queue,
        encoder: &mut encoder,
        target: &target.view,
        format,
        size,
        dt,
        update_ms: 0.0,
        rt,
        ui,
    });
    if readback {
        target.copy(&mut encoder);
    }
    queue.submit([encoder.finish()]);
    readback.then(|| target.read(device))
}

#[cfg(test)]
mod tests;
