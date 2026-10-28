//! The macOS calls the scenario makes in-process: its grants, keys sent to
//! Venus and the window list. Elsewhere each call fails, and run and
//! scenario refuse to run off Apple Silicon before any step reaches one.

#[cfg(not(target_os = "macos"))]
pub use elsewhere::{grants, press, windows};
#[cfg(target_os = "macos")]
pub use native::{grants, press, windows};

#[cfg(target_os = "macos")]
mod native {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use objc2_core_foundation::{
        CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
    };
    use objc2_core_graphics::{
        CGDisplayBounds, CGEvent, CGEventFlags, CGMainDisplayID, CGPreflightPostEventAccess,
        CGPreflightScreenCaptureAccess, CGRectMakeWithDictionaryRepresentation,
        CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowBounds,
        kCGWindowLayer, kCGWindowOwnerPID,
    };
    use std::{thread, time::Duration};

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
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
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
}
