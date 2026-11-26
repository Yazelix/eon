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
//! - the cursor stays visible, and the hosted renderer draws no cursor tail.

mod fixtures;

#[cfg(test)]
pub(crate) use fixtures::messages as fixture_messages;

use crate::{
    ImageKey, ModelEffect, Renderer, Scene, SessionModel,
    render::{FontSettings, FontSetup, HOSTED_LOOK, PresentOutcome},
};
use orbit_protocol::session::{decode_server_message, encode_server_message};
use std::{
    collections::HashSet,
    error::Error,
    io::BufRead,
    sync::{Arc, Mutex},
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

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
    validate_fixtures()?;
    // The Darwin package sets this list; Linux gets its fonts from FONTCONFIG_FILE.
    let fonts = FontSetup::with_directories(
        &FontSettings::default(),
        std::env::var_os("EON_VENUS_FONT_DIRS").as_deref(),
    )?;
    match host {
        Host::Rune => rune_host::run(fonts),
        Host::Venus => venus_host::run(fonts),
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
}

impl Entry {
    pub(crate) fn new() -> Result<Self> {
        let mut entry = Self {
            cycle: 0,
            step: 0,
            model: SessionModel::new(),
            generation: 0,
            pending: Vec::new(),
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
fn report_drawn((cycle, step): (u64, usize)) {
    println!("drawn {cycle} {step} {}", fixtures::STEPS[step]);
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
        pub(crate) drawn: Option<(u64, usize)>,
    }

    pub(crate) fn view(_: &State, _: &mut Cx<State>) -> El {
        grid_view()
    }

    pub(crate) fn run(fonts: FontSetup) -> Result {
        let failure = Failure::default();
        let entry = Entry::new()?;
        let mut app = App::new(AppSpec::new(
            "Eon (Rune candidate)",
            (1000.0, 640.0),
            || unreachable!("App::init supplies the state"),
            view,
        ))
        .args(Vec::new())
        .srgb_blend(false)
        .init(move || State { entry, drawn: None });
        let handle = app.handle();
        let mut printed = None;
        app = app
            .compositor(GridCompositor::new(
                fonts,
                failure.clone(),
                Some(handle.clone()),
            ))
            .on_frame_drawn(move |rt: &rune::Runtime<State>| {
                let state = &rt.state;
                if let Some(drawn) = state.drawn
                    && printed != Some(drawn)
                {
                    printed = Some(drawn);
                    report_drawn(drawn);
                }
            });
        let advancing = handle.clone();
        read_advances(move || advancing.update(|state: &mut State, _| state.entry.advance()));
        app.try_run()?;
        failure.into_result()
    }

    /// Draws the hosted Venus renderer at the grid slot, inside Rune's frame pass.
    pub(crate) struct GridCompositor {
        fonts: Option<FontSetup>,
        renderer: Option<(Renderer, FrameTarget)>,
        prepared: Option<SlotGeometry>,
        failure: Failure,
        quit: Option<AppHandle<State>>,
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
                renderer: None,
                prepared: None,
                failure,
                quit,
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
        fn prepare(&mut self, f: &mut Frame<'_, State>) -> Option<SlotGeometry> {
            let (geometry, target) =
                match preflight(f, self.renderer.as_ref().map(|(_, target)| *target)) {
                    Ok(preflight) => preflight,
                    Err(message) => {
                        self.fail(message);
                        return None;
                    }
                };
            if self.renderer.is_none() {
                let fonts = self.fonts.take()?;
                match hosted_renderer(f, geometry, fonts) {
                    Ok(renderer) => self.renderer = Some((renderer, target)),
                    Err(error) => {
                        self.fail(error);
                        return None;
                    }
                }
            }
            let (renderer, _) = self.renderer.as_mut()?;
            if self.prepared != Some(geometry) {
                resize_to(renderer, geometry);
            }
            let entry = &mut f.rt.state.entry;
            entry.sync_images(renderer);
            if let Err(error) =
                renderer.prepare_hosted(entry.scene(), (None, 0.0), true, entry.generation, "", "")
            {
                self.fail(error.to_string());
                return None;
            }
            self.prepared = Some(geometry);
            Some(geometry)
        }
    }

    impl Compositor<State> for GridCompositor {
        fn render(&mut self, f: &mut Frame<'_, State>) {
            let prepared = self.prepare(f);
            let renderer =
                prepared.and_then(|_| self.renderer.as_ref().map(|(renderer, _)| renderer));
            let drawn = draw_slot(f, renderer.zip(prepared), Fit::Exact);
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
            if let Some((renderer, _)) = self.renderer.as_mut() {
                renderer.end_frame_hosted();
            }
            if self.drew {
                let entry = &f.rt.state.entry;
                f.rt.state.drawn = Some((entry.cycle, entry.step));
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
    ) -> std::result::Result<Renderer, String> {
        Renderer::hosted(
            f.device.clone(),
            f.queue.clone(),
            f.format,
            geometry.size(),
            f64::from(geometry.scale()),
            fonts,
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
    }

    /// Runs Rune's frame pass, drawing `prepared`'s renderer at the grid slot when the slot
    /// sits where it was prepared, within `fit`, in the prepared target. With nothing prepared
    /// the frame still draws Rune's clear, so the surface is committed.
    pub(crate) fn draw_slot<S: 'static>(
        f: &mut Frame<'_, S>,
        prepared: Option<(&Renderer, SlotGeometry)>,
        fit: Fit,
    ) -> Drawn {
        f.ui.prepare(
            f.device,
            f.queue,
            &mut f.rt.text.atlas,
            &f.rt.instances,
            f.size,
        );
        let target_format = f.format;
        let mut refusal = None;
        let mut moved = None;
        let mut calls = 0;
        let mut slot = None;
        f.ui.draw_with(f.encoder, f.target, RUNE_CLEAR, &mut |pass,
                                                              site: &EffectSite<
            '_,
        >| {
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
        });
        Drawn {
            hook_calls: calls,
            slot,
            refusal,
            moved,
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
        drawn: Option<(u64, usize)>,
        failure: Failure,
    }

    /// Wakes the loop for one advance.
    struct Advance;

    pub(super) fn run(fonts: FontSetup) -> Result {
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
        };
        event_loop.run_app(&mut parent)?;
        failure.into_result()
    }

    impl ApplicationHandler<Advance> for Parent {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let attributes = Window::default_attributes()
                .with_title("Eon (Venus parent)")
                .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 640.0));
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
            let look = HOSTED_LOOK;
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
                Ok(renderer) => self.renderer = Some(renderer),
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
            let outcome = renderer.render(
                Some(self.entry.scene()),
                None,
                0.0,
                None,
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
                    let position = (self.entry.cycle, self.entry.step);
                    if self.drawn != Some(position) {
                        self.drawn = Some(position);
                        report_drawn(position);
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

#[cfg(test)]
mod tests;
