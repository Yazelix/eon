//! One pointer on the private display; generated bindings own the Wayland wire.
use super::*;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{wl_callback, wl_pointer, wl_registry},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct Registry {
    manager: Option<ZwlrVirtualPointerManagerV1>,
    synced: bool,
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
            && interface == "zwlr_virtual_pointer_manager_v1"
        {
            state.manager = Some(registry.bind(name, version.min(2), qh, ()));
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
delegate_noop!(Registry: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Registry: ignore ZwlrVirtualPointerV1);

pub(super) struct Pointer {
    connection: Connection,
    pointer: ZwlrVirtualPointerV1,
}
impl Pointer {
    pub fn new(launch: &Headless, columns: u64, width: u64, height: u64) -> Result<Self> {
        let socket = std::os::unix::net::UnixStream::connect(
            Path::new(&launch.env["XDG_RUNTIME_DIR"]).join(&launch.env["WAYLAND_DISPLAY"]),
        )
        .map_err(error)?;
        let connection = Connection::from_socket(socket).map_err(error)?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut state = Registry::default();
        roundtrip(&connection, &mut queue, &mut state)?;
        let pointer = state
            .manager
            .as_ref()
            .ok_or("private Sway lacks virtual pointer protocol")?
            .create_virtual_pointer(None, &qh, ());
        pointer.motion_absolute(
            milliseconds(),
            (width - width / columns / 2) as u32,
            (height / 2) as u32,
            width as u32,
            height as u32,
        );
        pointer.frame();
        connection.flush().map_err(error)?;
        roundtrip(&connection, &mut queue, &mut state)?;
        Ok(Self {
            connection,
            pointer,
        })
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
        self.connection.flush().map_err(error)
    }
    pub fn stop(&self) -> Result {
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        self.pointer
            .axis_stop(milliseconds(), wl_pointer::Axis::VerticalScroll);
        self.pointer.frame();
        self.connection.flush().map_err(error)
    }
}
impl Drop for Pointer {
    fn drop(&mut self) {
        self.pointer.destroy();
        let _ = self.connection.flush();
    }
}
fn milliseconds() -> u32 {
    (environment::monotonic_ns() / 1_000_000) as u32
}

fn roundtrip(
    connection: &Connection,
    queue: &mut EventQueue<Registry>,
    state: &mut Registry,
) -> Result {
    use std::os::fd::{AsFd, AsRawFd};
    state.synced = false;
    connection.display().sync(&queue.handle(), ());
    connection.flush().map_err(error)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        suite::interrupt::check()?;
        queue.dispatch_pending(state).map_err(error)?;
        if state.synced {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("private pointer handshake exceeded five seconds".into());
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fd = libc::pollfd {
            fd: connection.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one initialized pollfd for this live connection.
        let ready = unsafe { libc::poll(&mut fd, 1, 100) };
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error(e));
        }
        if ready > 0 {
            guard.read().map_err(error)?;
        }
    }
}
