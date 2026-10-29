//! The macOS calls the scenario makes in-process: its grants, keys sent to
//! Venus, the window list, the backdrop and the displays' modes. Elsewhere
//! each call fails, and run and scenario refuse to run off Apple Silicon
//! before any step reaches one.

#[cfg(not(target_os = "macos"))]
pub use elsewhere::{
    backdrop, displays, grants, in_mirror_set, mirror, press, switch, virtual_display, windows,
};
#[cfg(target_os = "macos")]
pub use native::{
    backdrop, displays, grants, in_mirror_set, mirror, press, switch, virtual_display, windows,
};

pub const VIRTUAL_DISPLAY_NAME: &str = "mac-proof HiDPI";

pub struct Window {
    pub id: u32,
    pub bounds: [f64; 4],
}

#[cfg(target_os = "macos")]
mod native {
    use super::{VIRTUAL_DISPLAY_NAME, Window};
    use crate::retina::{Display, Mode};
    use objc2::{
        MainThreadMarker, msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, Sel},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
        NSBackingStoreType, NSColor, NSRunningApplication, NSScreen, NSWindow,
        NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_core_foundation::{
        CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
        CGSize,
    };
    use objc2_core_graphics::{
        CGBeginDisplayConfiguration, CGCancelDisplayConfiguration, CGCompleteDisplayConfiguration,
        CGConfigureDisplayMirrorOfDisplay, CGConfigureDisplayWithDisplayMode, CGConfigureOption,
        CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyAllDisplayModes, CGDisplayCopyDisplayMode,
        CGDisplayIsBuiltin, CGDisplayIsInMirrorSet, CGDisplayIsMain, CGDisplayMirrorsDisplay,
        CGDisplayMode, CGError, CGEvent, CGEventFlags, CGGetActiveDisplayList, CGMainDisplayID,
        CGPreflightPostEventAccess, CGPreflightScreenCaptureAccess,
        CGRectMakeWithDictionaryRepresentation, CGWindowLevelForKey, CGWindowLevelKey,
        CGWindowListCopyWindowInfo, CGWindowListOption, kCGDisplayShowDuplicateLowResolutionModes,
        kCGNullDirectDisplay, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber,
        kCGWindowOwnerPID,
    };
    use std::{
        ffi::{CStr, c_char, c_void},
        io::{self, Write},
        ptr, thread,
        time::Duration,
    };

    #[link(name = "System")]
    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut AnyObject;
        fn dispatch_release(object: *mut AnyObject);
    }

    fn virtual_class(name: &CStr, selectors: &[Sel]) -> Result<&'static AnyClass, String> {
        let class = AnyClass::get(name)
            .ok_or_else(|| format!("virtual display: no class {}", name.to_string_lossy()))?;
        if class.class_method(sel!(alloc)).is_none() {
            return Err(format!(
                "virtual display: {} has no +alloc",
                class.name().to_string_lossy()
            ));
        }
        for &selector in selectors.iter().chain(&[sel!(retain), sel!(release)]) {
            if class.instance_method(selector).is_none() {
                return Err(format!(
                    "virtual display: {} has no -{selector}",
                    class.name().to_string_lossy()
                ));
            }
        }
        Ok(class)
    }

    /// Keeps a HiDPI virtual display alive until stdin closes.
    pub fn virtual_display() -> Result<(), String> {
        let descriptor_class = virtual_class(
            c"CGVirtualDisplayDescriptor",
            &[
                sel!(init),
                sel!(setName:),
                sel!(setVendorID:),
                sel!(setProductID:),
                sel!(setSerialNum:),
                sel!(setMaxPixelsWide:),
                sel!(setMaxPixelsHigh:),
                sel!(setSizeInMillimeters:),
                sel!(setWhitePoint:),
                sel!(setRedPrimary:),
                sel!(setGreenPrimary:),
                sel!(setBluePrimary:),
                sel!(setQueue:),
            ],
        )?;
        let display_class = virtual_class(
            c"CGVirtualDisplay",
            &[
                sel!(initWithDescriptor:),
                sel!(applySettings:),
                sel!(displayID),
            ],
        )?;
        let settings_class = virtual_class(
            c"CGVirtualDisplaySettings",
            &[sel!(init), sel!(setHiDPI:), sel!(setModes:)],
        )?;
        let mode_class = virtual_class(
            c"CGVirtualDisplayMode",
            &[sel!(initWithWidth:height:refreshRate:)],
        )?;
        let serial_number = descriptor_class
            .instance_method(sel!(setSerialNumber:))
            .is_some();
        let rotation = settings_class.instance_method(sel!(setRotation:)).is_some();
        let array_class = AnyClass::get(c"NSArray").ok_or("virtual display: no class NSArray")?;
        if array_class.class_method(sel!(arrayWithObject:)).is_none() {
            return Err("virtual display: NSArray has no +arrayWithObject:".to_owned());
        }
        for selector in [sel!(retain), sel!(release)] {
            if array_class.instance_method(selector).is_none() {
                return Err(format!("virtual display: NSArray has no -{selector}"));
            }
        }
        let allocate = |class: &AnyClass| -> Result<Allocated<AnyObject>, String> {
            // SAFETY: +alloc was checked before any message was sent.
            let object: Allocated<AnyObject> = unsafe { msg_send![class, alloc] };
            if Allocated::as_ptr(&object).is_null() {
                return Err(format!(
                    "virtual display: {} alloc returned nil",
                    class.name().to_string_lossy()
                ));
            }
            Ok(object)
        };
        // SAFETY: every selector was checked above. The signatures follow the
        // CoreGraphics classes: dimensions and hiDPI are unsigned int, geometry
        // is CGSize/CGPoint by value, and applySettings: returns BOOL.
        unsafe {
            let descriptor: Option<Retained<AnyObject>> =
                msg_send![allocate(descriptor_class)?, init];
            let descriptor = descriptor
                .ok_or("virtual display: CGVirtualDisplayDescriptor init returned nil")?;
            let name = CFString::from_str(VIRTUAL_DISPLAY_NAME);
            let name: &AnyObject = name.as_ref();
            let _: () = msg_send![&descriptor, setName: name];
            let _: () = msg_send![&descriptor, setVendorID: 0x454fu32];
            let _: () = msg_send![&descriptor, setProductID: 1u32];
            let _: () = msg_send![&descriptor, setSerialNum: 1u32];
            if serial_number {
                let _: () = msg_send![&descriptor, setSerialNumber: 1u32];
            }
            let _: () = msg_send![&descriptor, setMaxPixelsWide: 2048u32];
            let _: () = msg_send![&descriptor, setMaxPixelsHigh: 1536u32];
            let _: () = msg_send![&descriptor, setSizeInMillimeters: CGSize::new(25.4 * 2048.0 / 220.0, 25.4 * 1536.0 / 220.0)];
            let _: () = msg_send![&descriptor, setWhitePoint: CGPoint::new(0.3125, 0.3291)];
            let _: () = msg_send![&descriptor, setRedPrimary: CGPoint::new(0.6797, 0.3203)];
            let _: () = msg_send![&descriptor, setGreenPrimary: CGPoint::new(0.2559, 0.6983)];
            let _: () = msg_send![&descriptor, setBluePrimary: CGPoint::new(0.1494, 0.0557)];
            let queue = dispatch_queue_create(c"mac-proof.virtual-display".as_ptr(), ptr::null());
            if queue.is_null() {
                return Err("virtual display: dispatch_queue_create returned nil".to_owned());
            }
            let _: () = msg_send![&descriptor, setQueue: queue];
            dispatch_release(queue);
            let display: Option<Retained<AnyObject>> =
                msg_send![allocate(display_class)?, initWithDescriptor: &*descriptor];
            let display = display
                .ok_or("virtual display: CGVirtualDisplay initWithDescriptor: returned nil")?;
            let settings: Option<Retained<AnyObject>> = msg_send![allocate(settings_class)?, init];
            let settings =
                settings.ok_or("virtual display: CGVirtualDisplaySettings init returned nil")?;
            let _: () = msg_send![&settings, setHiDPI: 1u32];
            if rotation {
                let _: () = msg_send![&settings, setRotation: 0u32];
            }
            let mode: Option<Retained<AnyObject>> = msg_send![allocate(mode_class)?, initWithWidth: 2048u32, height: 1536u32, refreshRate: 60.0f64];
            let mode = mode.ok_or("virtual display: CGVirtualDisplayMode initWithWidth:height:refreshRate: returned nil")?;
            let modes: Retained<AnyObject> = msg_send![array_class, arrayWithObject: &*mode];
            let _: () = msg_send![&settings, setModes: &*modes];
            let applied: bool = msg_send![&display, applySettings: &*settings];
            if !applied {
                return Err(
                    "virtual display: CGVirtualDisplay applySettings: returned NO".to_owned(),
                );
            }
            let id: u32 = msg_send![&display, displayID];
            println!("display {id}");
            io::stdout()
                .flush()
                .map_err(|error| format!("virtual display stdout: {error}"))?;
            io::copy(&mut io::stdin().lock(), &mut io::sink())
                .map_err(|error| format!("virtual display stdin: {error}"))?;
            drop(display);
        }
        Ok(())
    }

    /// Shows a solid backdrop below normal windows until stdin closes.
    pub fn backdrop(rgb: [u8; 3]) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("backdrop needs the main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
            return Err("backdrop cannot use the Accessory activation policy".to_owned());
        }
        let screen = NSScreen::screens(mtm)
            .firstObject()
            .ok_or("backdrop has no main display")?;
        // SAFETY: AppKit runs on the main thread, with a valid screen frame
        // and the borderless, buffered window styles.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                screen.frame(),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the retained window owns its release, including after close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(true);
        window.setHasShadow(false);
        let [r, g, b] = rgb.map(|channel| f64::from(channel) / 255.0);
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
        window.setBackgroundColor(Some(&color));
        let level = CGWindowLevelForKey(CGWindowLevelKey::DesktopIconWindowLevelKey);
        window.setLevel(level as isize + 1);
        window.setIgnoresMouseEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.orderFrontRegardless();
        println!("shown");
        io::stdout()
            .flush()
            .map_err(|error| format!("backdrop stdout: {error}"))?;
        thread::spawn(|| {
            let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
            std::process::exit(0);
        });
        app.run();
        Err("backdrop run loop ended".to_owned())
    }

    /// Whether this process may capture other windows (Screen Recording)
    /// and post key events (Accessibility), without asking.
    pub fn grants() -> Result<(bool, bool), String> {
        Ok((
            CGPreflightScreenCaptureAccess(),
            CGPreflightPostEventAccess(),
        ))
    }

    /// Brings process `pid` to the front, then posts it each key, as a
    /// virtual key code and modifier flags, pressed and released.
    pub fn press(pid: u32, keys: &[(u16, u64)]) -> Result<(), String> {
        let pid = i32::try_from(pid).map_err(|error| format!("pid {pid}: {error}"))?;
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .ok_or_else(|| format!("no application with pid {pid}"))?;
        // Ignoring other applications counts before macOS 14; later systems
        // decide activation themselves.
        #[allow(deprecated)]
        let options = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
        if !app.activateWithOptions(options) {
            return Err(format!(
                "the application with pid {pid} cannot be activated"
            ));
        }
        thread::sleep(Duration::from_millis(500));
        for &(code, flags) in keys {
            for down in [true, false] {
                let event = CGEvent::new_keyboard_event(None, code, down)
                    .ok_or_else(|| format!("no key event for key code {code}"))?;
                CGEvent::set_flags(Some(&event), CGEventFlags::from_bits_retain(flags));
                CGEvent::post_to_pid(pid, Some(&event));
                thread::sleep(Duration::from_millis(50));
            }
        }
        thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    /// The ID and bounds of each on-screen window of process `pid` in its normal
    /// layer, and the size of the main display, all dimensions in points.
    pub fn windows(pid: u32) -> Result<(Vec<Window>, [f64; 2]), String> {
        let list =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)
                .ok_or("the window list is unavailable")?;
        // SAFETY: the window list is an array of dictionaries keyed by
        // strings (CGWindow.h), and the keys are constant strings.
        let (list, owner, layer, bounds_key, number_key) = unsafe {
            (
                CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(list),
                kCGWindowOwnerPID,
                kCGWindowLayer,
                kCGWindowBounds,
                kCGWindowNumber,
            )
        };
        let number = |window: &CFDictionary<CFString, CFType>, key: &CFString| {
            window
                .get(key)
                .and_then(|value| value.downcast::<CFNumber>().ok())
                .and_then(|number| number.as_i64())
        };
        let mut found = Vec::new();
        for window in list.iter() {
            if number(&window, owner) != Some(i64::from(pid)) || number(&window, layer) != Some(0) {
                continue;
            }
            let Some(id) = number(&window, number_key)
                .and_then(|number| u32::try_from(number).ok())
                .filter(|id| *id != 0)
            else {
                continue;
            };
            let Some(bounds) = window
                .get(bounds_key)
                .and_then(|value| value.downcast::<CFDictionary>().ok())
            else {
                continue;
            };
            let mut rect = CGRect::default();
            // SAFETY: `bounds` is an untyped dictionary, as the call takes,
            // and `rect` is a CGRect for the call to fill.
            if unsafe { CGRectMakeWithDictionaryRepresentation(Some(&bounds), &mut rect) } {
                found.push(Window {
                    id,
                    bounds: [
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ],
                });
            }
        }
        let display = CGDisplayBounds(CGMainDisplayID());
        Ok((found, [display.size.width, display.size.height]))
    }

    fn mode(mode: &CGDisplayMode) -> Mode {
        let mode = Some(mode);
        let size = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        Mode {
            width: size(CGDisplayMode::width(mode)),
            height: size(CGDisplayMode::height(mode)),
            pixel_width: size(CGDisplayMode::pixel_width(mode)),
            pixel_height: size(CGDisplayMode::pixel_height(mode)),
            refresh: CGDisplayMode::refresh_rate(mode),
            desktop: CGDisplayMode::is_usable_for_desktop_gui(mode),
        }
    }

    /// Every mode of `display`, with the HiDPI ones that macOS lists only
    /// on request.
    fn modes(display: CGDirectDisplayID) -> Result<Vec<CFRetained<CGDisplayMode>>, String> {
        // SAFETY: the key is a constant string.
        let key: &CFString = unsafe { kCGDisplayShowDuplicateLowResolutionModes };
        let options =
            CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
        // SAFETY: the options map a string key to a boolean, as
        // CGDirectDisplay.h asks.
        let list = unsafe { CGDisplayCopyAllDisplayModes(display, Some(options.as_opaque())) }
            .ok_or_else(|| format!("display {display} lists no modes"))?;
        // SAFETY: the list holds display modes (CGDirectDisplay.h).
        let list = unsafe { CFRetained::cast_unchecked::<CFArray<CGDisplayMode>>(list) };
        Ok(list.iter().collect())
    }

    /// Each active display, its current mode and every mode it offers.
    pub fn displays() -> Result<Vec<Display>, String> {
        let mut ids = [0; 16];
        let mut count = 0;
        // SAFETY: `ids` has room for the 16 displays asked for, and `count`
        // is there for the call to fill.
        let error = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
        if error != CGError::Success {
            return Err(format!(
                "the display list is unavailable: CGError {}",
                error.0
            ));
        }
        ids.iter()
            .take(count as usize)
            .map(|&id| {
                let current = CGDisplayCopyDisplayMode(id)
                    .ok_or_else(|| format!("display {id} has no current mode"))?;
                Ok(Display {
                    id,
                    main: CGDisplayIsMain(id),
                    builtin: CGDisplayIsBuiltin(id),
                    current: mode(&current),
                    modes: modes(id)?.iter().map(|each| mode(each)).collect(),
                })
            })
            .collect()
    }

    /// Sets `display` to `mode` for as long as this process runs, so that
    /// macOS restores it when the process ends however it ends.
    fn configure(display: CGDirectDisplayID, mode: &CGDisplayMode) -> Result<(), String> {
        let failed = |call: &str, error: CGError| format!("{call}: CGError {}", error.0);
        let mut config = ptr::null_mut();
        // SAFETY: `config` is there for the call to fill, and each later
        // call takes the configuration it filled in, once completed or
        // cancelled.
        unsafe {
            let error = CGBeginDisplayConfiguration(&mut config);
            if error != CGError::Success {
                return Err(failed("CGBeginDisplayConfiguration", error));
            }
            let error = CGConfigureDisplayWithDisplayMode(config, display, Some(mode), None);
            if error != CGError::Success {
                CGCancelDisplayConfiguration(config);
                return Err(failed("CGConfigureDisplayWithDisplayMode", error));
            }
            let error = CGCompleteDisplayConfiguration(config, CGConfigureOption::ForAppOnly);
            if error != CGError::Success {
                return Err(failed("CGCompleteDisplayConfiguration", error));
            }
        }
        Ok(())
    }

    /// A display switched to another mode, which `restore` or dropping it
    /// switches back.
    pub struct Switched {
        display: CGDirectDisplayID,
        original: Option<CFRetained<CGDisplayMode>>,
    }

    impl Switched {
        pub fn restore(&mut self) -> Result<(), String> {
            if let Some(original) = &self.original {
                configure(self.display, original)?;
            }
            // Kept after a failure, so that dropping it tries again.
            self.original = None;
            Ok(())
        }
    }

    impl Drop for Switched {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }

    /// Switches `display` to its mode equal to `to`.
    pub fn switch(display: CGDirectDisplayID, to: Mode) -> Result<Switched, String> {
        let original = CGDisplayCopyDisplayMode(display)
            .ok_or_else(|| format!("display {display} has no current mode"))?;
        let target = modes(display)?
            .into_iter()
            .find(|each| mode(each) == to)
            .ok_or_else(|| format!("display {display} no longer offers {to}"))?;
        configure(display, &target)?;
        Ok(Switched {
            display,
            original: Some(original),
        })
    }

    /// Whether `display` already belongs to a mirror set that the step must leave alone.
    pub fn in_mirror_set(display: CGDirectDisplayID) -> Result<bool, String> {
        Ok(CGDisplayIsInMirrorSet(display)
            || CGDisplayMirrorsDisplay(display) != kCGNullDirectDisplay)
    }

    fn configure_mirror(
        main: CGDirectDisplayID,
        master: CGDirectDisplayID,
        mode: Option<&CGDisplayMode>,
    ) -> Result<(), String> {
        let failed = |call: &str, error: CGError| format!("{call}: CGError {}", error.0);
        let mut config = ptr::null_mut();
        // SAFETY: config is filled by Begin, then used until completed or
        // cancelled. Only the main display and the virtual master are changed.
        unsafe {
            let error = CGBeginDisplayConfiguration(&mut config);
            if error != CGError::Success {
                return Err(failed("CGBeginDisplayConfiguration", error));
            }
            if let Some(mode) = mode {
                let error = CGConfigureDisplayWithDisplayMode(config, master, Some(mode), None);
                if error != CGError::Success {
                    CGCancelDisplayConfiguration(config);
                    return Err(failed("CGConfigureDisplayWithDisplayMode", error));
                }
            }
            let error = CGConfigureDisplayMirrorOfDisplay(config, main, master);
            if error != CGError::Success {
                CGCancelDisplayConfiguration(config);
                return Err(failed("CGConfigureDisplayMirrorOfDisplay", error));
            }
            let error = CGCompleteDisplayConfiguration(config, CGConfigureOption::ForAppOnly);
            if error != CGError::Success {
                return Err(failed("CGCompleteDisplayConfiguration", error));
            }
        }
        Ok(())
    }

    /// A main display mirrored to a virtual one, which `restore` or dropping it unmirrors.
    pub struct Mirrored {
        main: Option<CGDirectDisplayID>,
    }

    impl Mirrored {
        pub fn restore(&mut self) -> Result<(), String> {
            if let Some(main) = self.main {
                configure_mirror(main, kCGNullDirectDisplay, None)?;
            }
            // Kept after a failure, so that dropping it tries again.
            self.main = None;
            Ok(())
        }
    }

    impl Drop for Mirrored {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }

    /// Switches the virtual display to `to` and makes it the main display's mirror master.
    pub fn mirror(
        main: CGDirectDisplayID,
        display: CGDirectDisplayID,
        to: Mode,
    ) -> Result<Mirrored, String> {
        if in_mirror_set(main)? {
            return Err(format!(
                "main display {main} already belongs to a mirror set"
            ));
        }
        let target = modes(display)?
            .into_iter()
            .find(|each| mode(each) == to)
            .ok_or_else(|| format!("display {display} no longer offers {to}"))?;
        let mirrored = Mirrored { main: Some(main) };
        configure_mirror(main, display, Some(&target))?;
        Ok(mirrored)
    }
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use super::Window;
    use crate::retina::{Display, Mode};

    const MACOS: &str = "macOS only";

    pub fn backdrop(_rgb: [u8; 3]) -> Result<(), String> {
        Err(MACOS.to_owned())
    }

    pub fn virtual_display() -> Result<(), String> {
        Err(MACOS.to_owned())
    }

    pub fn grants() -> Result<(bool, bool), String> {
        Err(MACOS.to_owned())
    }

    pub fn press(_pid: u32, _keys: &[(u16, u64)]) -> Result<(), String> {
        Err(MACOS.to_owned())
    }

    pub fn windows(_pid: u32) -> Result<(Vec<Window>, [f64; 2]), String> {
        Err(MACOS.to_owned())
    }

    pub fn displays() -> Result<Vec<Display>, String> {
        Err(MACOS.to_owned())
    }

    pub struct Switched;

    impl Switched {
        pub fn restore(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    pub fn switch(_display: u32, _to: Mode) -> Result<Switched, String> {
        Err(MACOS.to_owned())
    }

    pub fn in_mirror_set(_display: u32) -> Result<bool, String> {
        Err(MACOS.to_owned())
    }

    pub struct Mirrored;

    impl Mirrored {
        pub fn restore(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    pub fn mirror(_main: u32, _display: u32, _to: Mode) -> Result<Mirrored, String> {
        Err(MACOS.to_owned())
    }
}
