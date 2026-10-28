//! The macOS calls the scenario makes in-process: its grants, keys sent to
//! Venus, the window list and the displays' modes. Elsewhere each call
//! fails, and run and scenario refuse to run off Apple Silicon before any
//! step reaches one.

#[cfg(not(target_os = "macos"))]
pub use elsewhere::{displays, grants, press, switch, windows};
#[cfg(target_os = "macos")]
pub use native::{displays, grants, press, switch, windows};

#[cfg(target_os = "macos")]
mod native {
    use crate::retina::{Display, Mode};
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use objc2_core_foundation::{
        CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
    };
    use objc2_core_graphics::{
        CGBeginDisplayConfiguration, CGCancelDisplayConfiguration, CGCompleteDisplayConfiguration,
        CGConfigureDisplayWithDisplayMode, CGConfigureOption, CGDirectDisplayID, CGDisplayBounds,
        CGDisplayCopyAllDisplayModes, CGDisplayCopyDisplayMode, CGDisplayIsBuiltin,
        CGDisplayIsMain, CGDisplayMode, CGError, CGEvent, CGEventFlags, CGGetActiveDisplayList,
        CGMainDisplayID, CGPreflightPostEventAccess, CGPreflightScreenCaptureAccess,
        CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
        kCGDisplayShowDuplicateLowResolutionModes, kCGNullWindowID, kCGWindowBounds,
        kCGWindowLayer, kCGWindowOwnerPID,
    };
    use std::{ptr, thread, time::Duration};

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

    /// The bounds of each on-screen window of process `pid` in its normal
    /// layer, and the size of the main display, all in points as x, y,
    /// width and height.
    pub fn windows(pid: u32) -> Result<(Vec<[f64; 4]>, [f64; 2]), String> {
        let list =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)
                .ok_or("the window list is unavailable")?;
        // SAFETY: the window list is an array of dictionaries keyed by
        // strings (CGWindow.h), and the keys are constant strings.
        let (list, owner, layer, bounds_key) = unsafe {
            (
                CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(list),
                kCGWindowOwnerPID,
                kCGWindowLayer,
                kCGWindowBounds,
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
                found.push([
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ]);
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
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use crate::retina::{Display, Mode};

    const MACOS: &str = "macOS only";

    pub fn grants() -> Result<(bool, bool), String> {
        Err(MACOS.to_owned())
    }

    pub fn press(_pid: u32, _keys: &[(u16, u64)]) -> Result<(), String> {
        Err(MACOS.to_owned())
    }

    pub fn windows(_pid: u32) -> Result<(Vec<[f64; 4]>, [f64; 2]), String> {
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
}
