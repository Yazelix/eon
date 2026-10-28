//! The retina step's data: each display's modes as macOS lists them,
//! `system_profiler`'s display report and the scale Venus logs, read here
//! without the Mac so Linux tests it.

use serde_json::{Value, json};
use std::{fmt, process::Command, time::Instant};

use crate::step::{Failure, Log, check, execute_filtered};

/// Removes device identifiers, including display serials, at every depth.
pub fn redact(mut value: Value) -> Value {
    match &mut value {
        Value::Object(object) => {
            object.retain(|key, _| {
                let key = key.to_ascii_lowercase();
                !["serial", "uuid", "udid"]
                    .iter()
                    .any(|part| key.contains(part))
            });
            for value in object.values_mut() {
                *value = redact(value.take());
            }
        }
        Value::Array(array) => {
            for value in array {
                *value = redact(value.take());
            }
        }
        _ => {}
    }
    value
}

/// Never writes the unredacted profiler output to evidence.
pub fn system_profiler(log: &mut Log, types: &[&str], deadline: Instant) -> Result<Value, Failure> {
    let (status, bytes) = execute_filtered(
        log,
        Command::new("system_profiler").arg("-json").args(types),
        deadline,
        true,
        true,
        |bytes| {
            let value = serde_json::from_slice(&bytes)
                .map_err(|error| Failure::Check(format!("system_profiler JSON: {error}")))?;
            Ok(redact(value).to_string().into_bytes())
        },
    )?;
    check(status.success(), || {
        format!("system_profiler exited with {status}")
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Failure::Check(format!("system_profiler JSON: {error}")))
}

/// A display mode, in points and pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub refresh: f64,
    /// Whether macOS offers it for the desktop.
    pub desktop: bool,
}

impl Mode {
    /// Pixels per point, which is the backing scale a window on the display
    /// gets in this mode.
    pub fn scale(&self) -> f64 {
        match self.width {
            0 => 0.0,
            width => f64::from(self.pixel_width) / f64::from(width),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}x{} points at {}x{} pixels (scale {}, {} Hz{})",
            self.width,
            self.height,
            self.pixel_width,
            self.pixel_height,
            self.scale(),
            self.refresh,
            if self.desktop {
                ""
            } else {
                ", not for the desktop"
            }
        )
    }
}

pub struct Display {
    pub id: u32,
    pub main: bool,
    pub builtin: bool,
    pub current: Mode,
    /// Every mode, HiDPI ones included.
    pub modes: Vec<Mode>,
}

/// The displays as JSON, for the evidence folder.
pub fn report(displays: &[Display]) -> Value {
    let mode = |mode: &Mode| {
        json!({
            "width": mode.width,
            "height": mode.height,
            "pixel_width": mode.pixel_width,
            "pixel_height": mode.pixel_height,
            "scale": mode.scale(),
            "refresh": mode.refresh,
            "desktop": mode.desktop,
        })
    };
    Value::Array(
        displays
            .iter()
            .map(|display| {
                json!({
                    "id": display.id,
                    "main": display.main,
                    "builtin": display.builtin,
                    "current": mode(&display.current),
                    "modes": display.modes.iter().map(mode).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// The HiDPI desktop mode to switch from `current` to: the one with
/// `current`'s pixels if there is one, else the most points, and
/// `current`'s refresh rate among equals.
pub fn hidpi(current: &Mode, modes: &[Mode]) -> Option<Mode> {
    modes
        .iter()
        .filter(|mode| mode.desktop && mode.scale() >= 2.0)
        .max_by_key(|mode| {
            (
                (mode.pixel_width, mode.pixel_height)
                    == (current.pixel_width, current.pixel_height),
                u64::from(mode.width) * u64::from(mode.height),
                mode.refresh == current.refresh,
            )
        })
        .copied()
}

/// A display in `system_profiler -json SPDisplaysDataType`.
#[derive(Debug, PartialEq)]
pub struct Profiled {
    pub name: String,
    pub main: bool,
    /// Width and height in pixels.
    pub pixels: (u32, u32),
    /// Width and height in points, which macOS calls its resolution.
    pub points: (u32, u32),
}

impl Profiled {
    pub fn scale(&self) -> f64 {
        match self.points.0 {
            0 => 0.0,
            width => f64::from(self.pixels.0) / f64::from(width),
        }
    }
}

/// Every display of each graphics processor in `report`.
pub fn profiled(report: &Value) -> Result<Vec<Profiled>, String> {
    let size = |display: &Value, key: &str| -> Result<(u32, u32), String> {
        let text = display[key].as_str().unwrap_or_default();
        // "2560 x 1600", or "1440 x 900 @ 60.00Hz".
        let mut numbers = text
            .split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(str::parse);
        match (numbers.next(), numbers.next()) {
            (Some(Ok(width)), Some(Ok(height))) => Ok((width, height)),
            _ => Err(format!("{key} {:?} is not a size", display[key])),
        }
    };
    let processors = report["SPDisplaysDataType"]
        .as_array()
        .ok_or("no SPDisplaysDataType list")?;
    processors
        .iter()
        .flat_map(|processor| {
            processor["spdisplays_ndrvs"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .map(|display| {
            Ok(Profiled {
                name: display["_name"].as_str().unwrap_or_default().to_owned(),
                main: display["spdisplays_main"] == "spdisplays_yes",
                pixels: size(display, "_spdisplays_pixels")?,
                points: size(display, "_spdisplays_resolution")?,
            })
        })
        .collect()
}

/// What Venus logs at window creation and on each scale change.
const VENUS_SCALE: &str = "venus: window scale factor ";

/// Each scale factor Venus logged in `log`, oldest first, from whole lines
/// only, since Venus may still be writing the last one.
pub fn venus_scales(log: &str) -> Vec<f64> {
    log.split_inclusive('\n')
        .filter_map(|line| {
            let line = line.strip_suffix('\n')?;
            line.split_once(VENUS_SCALE)?.1.trim().parse().ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiler_redaction_removes_nested_hardware_and_display_identifiers() {
        let report = json!({
            "SPHardwareDataType": [{"machine_model": "Mac14,2", "serial_number": "secret",
                "platform_UUID": "secret", "provisioning_UDID": "secret"}],
            "SPDisplaysDataType": [{"spdisplays_ndrvs": [{"_name": "Color LCD",
                "_spdisplays_display-serial-number": "secret", "_spdisplays_display-serial-number2": "secret",
                "nested": [{"Hardware UUID": "secret", "DisplaySerialNumber": "secret", "width": 1920}]}]}]
        });
        let redacted = redact(report);
        assert_eq!(
            redacted,
            json!({
                "SPHardwareDataType": [{"machine_model": "Mac14,2"}],
                "SPDisplaysDataType": [{"spdisplays_ndrvs": [{"_name": "Color LCD", "nested": [{"width": 1920}]}]}]
            })
        );
        assert_eq!(redact(redacted.clone()), redacted);
    }

    fn mode(width: u32, height: u32, pixel_width: u32, pixel_height: u32) -> Mode {
        Mode {
            width,
            height,
            pixel_width,
            pixel_height,
            refresh: 60.0,
            desktop: true,
        }
    }

    #[test]
    fn the_retina_step_reads_scales_and_picks_a_hidpi_mode() {
        let current = mode(1920, 1080, 1920, 1080);
        assert_eq!(current.scale(), 1.0);
        let native = mode(960, 540, 1920, 1080);
        let larger = mode(1280, 720, 2560, 1440);
        let hidden = Mode {
            desktop: false,
            ..mode(1920, 1080, 3840, 2160)
        };
        let slower = Mode {
            refresh: 30.0,
            ..native
        };
        assert_eq!(
            hidpi(&current, &[current, slower, larger, native, hidden]),
            Some(native)
        );
        assert_eq!(hidpi(&current, &[current, larger, hidden]), Some(larger));
        assert_eq!(hidpi(&current, &[current, hidden]), None);
        assert_eq!(
            native.to_string(),
            "960x540 points at 1920x1080 pixels (scale 2, 60 Hz)"
        );

        // An M2 MacBook Air on macOS 14.6, as published in
        // github.com/leighklotz/llamafiles at 345bed7, examples/mac-system-profiler-gpu.txt.
        let report: Value = serde_json::from_str(
            r#"{
  "SPDisplaysDataType" : [
    {
      "_name" : "Apple M2",
      "spdisplays_mtlgpufamilysupport" : "spdisplays_metal3",
      "spdisplays_ndrvs" : [
        {
          "_name" : "Color LCD",
          "_spdisplays_display-product-id" : "a052",
          "_spdisplays_display-serial-number" : "fd626d62",
          "_spdisplays_display-vendor-id" : "610",
          "_spdisplays_display-week" : "0",
          "_spdisplays_display-year" : "0",
          "_spdisplays_displayID" : "1",
          "_spdisplays_pixels" : "3420 x 2224",
          "_spdisplays_resolution" : "1710 x 1112 @ 60.00Hz",
          "spdisplays_ambient_brightness" : "spdisplays_yes",
          "spdisplays_connection_type" : "spdisplays_internal",
          "spdisplays_display_type" : "spdisplays_built-in-liquid-retina",
          "spdisplays_main" : "spdisplays_yes",
          "spdisplays_mirror" : "spdisplays_off",
          "spdisplays_online" : "spdisplays_yes",
          "spdisplays_pixelresolution" : "spdisplays_2560x1664Retina"
        }
      ],
      "spdisplays_vendor" : "sppci_vendor_Apple",
      "sppci_bus" : "spdisplays_builtin",
      "sppci_cores" : "10",
      "sppci_device_type" : "spdisplays_gpu",
      "sppci_model" : "Apple M2"
    }
  ]
}"#,
        )
        .unwrap();
        let displays = profiled(&report).unwrap();
        assert_eq!(
            displays,
            [Profiled {
                name: "Color LCD".to_owned(),
                main: true,
                pixels: (3420, 2224),
                points: (1710, 1112),
            }]
        );
        assert_eq!(displays[0].scale(), 2.0);
        assert!(
            profiled(&json!({"SPDisplaysDataType": [{"spdisplays_ndrvs": [{"_name": "x"}]}]}))
                .is_err()
        );

        assert_eq!(
            venus_scales(
                "venus: window scale factor 1\n\
                 eon: other\n\
                 [12:00] venus: window scale factor 2\n\
                 venus: window scale factor x\n\
                 venus: window scale factor 1"
            ),
            [1.0, 2.0]
        );
    }
}
