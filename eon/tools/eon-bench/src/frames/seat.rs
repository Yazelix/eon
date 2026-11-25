//! A virtual pointer, keyboard and input method on the private display;
//! generated bindings own the Wayland wire.
use super::*;
use rustix::event::PollFlags;
use std::{io::ErrorKind::WouldBlock, os::fd::AsFd};
use wayland_client::backend::WaylandError;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{wl_callback, wl_pointer, wl_registry, wl_seat::WlSeat},
};
use wayland_protocols_misc::{
    zwp_input_method_v2::client::{
        zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
        zwp_input_method_v2::{self, ZwpInputMethodV2},
    },
    zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    },
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// The compositor resolves the includes; evdev key codes follow.
const KEYMAP: &str = "xkb_keymap {\n xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n xkb_types { include \"complete\" };\n xkb_compat { include \"complete\" };\n xkb_symbols { include \"pc+us+inet(evdev)\" };\n};\n\0";

#[derive(Default)]
struct Registry {
    pointers: Option<ZwlrVirtualPointerManagerV1>,
    keyboards: Option<ZwpVirtualKeyboardManagerV1>,
    methods: Option<ZwpInputMethodManagerV2>,
    seat: Option<WlSeat>,
    synced: bool,
    pending: bool,
    active: bool,
    done: u32,
    unavailable: bool,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Registry {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointers = Some(registry.bind(name, version.min(2), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboards = Some(registry.bind(name, 1, qh, ()));
                }
                "zwp_input_method_manager_v2" => {
                    state.methods = Some(registry.bind(name, 1, qh, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for Registry {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.synced = true;
    }
}
impl Dispatch<ZwpInputMethodV2, ()> for Registry {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.pending = true,
            zwp_input_method_v2::Event::Deactivate => state.pending = false,
            zwp_input_method_v2::Event::Done => {
                state.active = state.pending;
                state.done = state.done.wrapping_add(1);
            }
            zwp_input_method_v2::Event::Unavailable => state.unavailable = true,
            _ => {}
        }
    }
}
delegate_noop!(Registry: ignore WlSeat);
delegate_noop!(Registry: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Registry: ignore ZwlrVirtualPointerV1);
delegate_noop!(Registry: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Registry: ignore ZwpVirtualKeyboardV1);
delegate_noop!(Registry: ignore ZwpInputMethodManagerV2);

/// One client connection to the private compositor, shared by every device.
struct Display {
    connection: Connection,
    queue: EventQueue<Registry>,
    state: Registry,
}
impl Display {
    fn connect(launch: &Headless) -> Result<Self> {
        let socket = std::os::unix::net::UnixStream::connect(
            Path::new(&launch.env["XDG_RUNTIME_DIR"]).join(&launch.env["WAYLAND_DISPLAY"]),
        )
        .map_err(error)?;
        let connection = Connection::from_socket(socket).map_err(error)?;
        let queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut display = Self {
            connection,
            queue,
            state: Registry::default(),
        };
        display.roundtrip()?;
        Ok(display)
    }
    /// Waits up to `wait` for `flags` on the socket; false on timeout or EINTR.
    fn poll(&self, flags: PollFlags, wait: Duration) -> Result<bool> {
        let mut fd = [rustix::event::PollFd::from_borrowed_fd(
            self.connection.as_fd(),
            flags,
        )];
        let timeout = rustix::event::Timespec::try_from(wait).map_err(error)?;
        match rustix::event::poll(&mut fd, Some(&timeout)) {
            Ok(n) => Ok(n > 0),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(e) => Err(error(std::io::Error::from(e))),
        }
    }
    /// Flushes, waiting up to five seconds while the compositor drains a full socket.
    fn flush(&self) -> Result {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.connection.flush() {
                Err(WaylandError::Io(e)) if e.kind() == WouldBlock && Instant::now() < deadline => {
                    self.poll(PollFlags::OUT, Duration::from_millis(100))?
                }
                other => return other.map_err(error),
            };
        }
    }
    /// Dispatches whatever arrives within `wait`.
    fn dispatch(&mut self, wait: Duration) -> Result {
        suite::interrupt::check()?;
        self.flush()?;
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(error)?;
        match self.queue.prepare_read() {
            Some(guard) if self.poll(PollFlags::IN, wait)? => guard.read().map_err(error)?,
            _ => return Ok(()),
        };
        self.queue
            .dispatch_pending(&mut self.state)
            .map(drop)
            .map_err(error)
    }
    fn roundtrip(&mut self) -> Result {
        self.state.synced = false;
        self.connection.display().sync(&self.queue.handle(), ());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.state.synced {
            if Instant::now() >= deadline {
                return Err("private Wayland roundtrip exceeded five seconds".into());
            }
            self.dispatch(Duration::from_millis(100))?;
        }
        Ok(())
    }
    fn seat(&self) -> Result<&WlSeat> {
        self.state
            .seat
            .as_ref()
            .ok_or("private compositor lacks a seat".into())
    }
}

pub(super) struct Pointer {
    display: Display,
    pointer: ZwlrVirtualPointerV1,
}
impl Pointer {
    pub fn new(launch: &Headless, columns: u64, width: u64, height: u64) -> Result<Self> {
        let mut display = Display::connect(launch)?;
        let pointer = display
            .state
            .pointers
            .as_ref()
            .ok_or("private Sway lacks virtual pointer protocol")?
            .create_virtual_pointer(None, &display.queue.handle(), ());
        pointer.motion_absolute(
            milliseconds(),
            (width - width / columns / 2) as u32,
            (height / 2) as u32,
            width as u32,
            height as u32,
        );
        pointer.frame();
        display.roundtrip()?;
        Ok(Self { display, pointer })
    }
    pub fn scroll(&self, pixels: bool) -> Result {
        self.pointer.axis_source(if pixels {
            wl_pointer::AxisSource::Finger
        } else {
            wl_pointer::AxisSource::Wheel
        });
        if pixels {
            self.pointer
                .axis(milliseconds(), wl_pointer::Axis::VerticalScroll, -4.0);
        } else {
            self.pointer
                .axis_discrete(milliseconds(), wl_pointer::Axis::VerticalScroll, -15.0, -1);
        }
        self.pointer.frame();
        self.display.flush()
    }
    pub fn stop(&self) -> Result {
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        self.pointer
            .axis_stop(milliseconds(), wl_pointer::Axis::VerticalScroll);
        self.pointer.frame();
        self.display.flush()
    }
}
impl Drop for Pointer {
    fn drop(&mut self) {
        self.pointer.destroy();
        let _ = self.display.flush();
    }
}

/// A `us` keyboard. Its keys do not move the seat's modifier state, so
/// modifiers are sent as their own depressed mask.
pub(super) struct Keyboard {
    display: Display,
    keyboard: ZwpVirtualKeyboardV1,
}
impl Keyboard {
    pub fn new(launch: &Headless) -> Result<Self> {
        let mut display = Display::connect(launch)?;
        let keyboard = display
            .state
            .keyboards
            .as_ref()
            .ok_or("private compositor lacks virtual keyboard protocol")?
            .create_virtual_keyboard(display.seat()?, &display.queue.handle(), ());
        let keymap = rustix::fs::memfd_create("eon-bench-keymap", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(error)?;
        rustix::io::write(&keymap, KEYMAP.as_bytes()).map_err(error)?;
        keyboard.keymap(1, keymap.as_fd(), KEYMAP.len() as u32);
        display.roundtrip()?;
        Ok(Self { display, keyboard })
    }
    pub fn key(&self, key: u32, pressed: bool) -> Result {
        self.keyboard.key(milliseconds(), key, u32::from(pressed));
        self.display.flush()
    }
    pub fn modifiers(&self, depressed: u32) -> Result {
        self.keyboard.modifiers(depressed, 0, 0, 0);
        self.display.flush()
    }
    pub fn roundtrip(&mut self) -> Result {
        self.display.roundtrip()
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        self.keyboard.destroy();
        let _ = self.display.flush();
    }
}

/// A bounded input method that answers with empty commits, as P0's busy
/// agent pane's echo did.
pub(super) struct Echo {
    display: Display,
    method: ZwpInputMethodV2,
}
impl Echo {
    pub fn new(launch: &Headless) -> Result<Self> {
        let mut display = Display::connect(launch)?;
        let method = display
            .state
            .methods
            .as_ref()
            .ok_or("private compositor lacks input method protocol")?
            .get_input_method(display.seat()?, &display.queue.handle(), ());
        display.roundtrip()?;
        Ok(Self { display, method })
    }
    pub fn active(&mut self) -> Result<bool> {
        self.display.dispatch(Duration::ZERO)?;
        if self.display.state.unavailable {
            return Err("another input method holds the private seat".into());
        }
        Ok(self.display.state.active)
    }
    /// Sends `count` empty commits, each on the next `done` or after
    /// `interval`, whichever comes first.
    pub fn burst(&mut self, count: u32, interval: Duration) -> Result {
        for _ in 0..count {
            let seen = self.display.state.done;
            let until = Instant::now() + interval;
            while self.display.state.done == seen {
                let Some(wait) = until.checked_duration_since(Instant::now()) else {
                    break;
                };
                self.display.dispatch(wait)?;
            }
            self.method.commit(self.display.state.done);
        }
        self.display.roundtrip()
    }
}
impl Drop for Echo {
    fn drop(&mut self) {
        self.method.destroy();
        let _ = self.display.flush();
    }
}

fn milliseconds() -> u32 {
    (environment::monotonic_ns() / 1_000_000) as u32
}
