use super::{CellMetrics, DEFAULT_METRICS, FALLBACK_CAP_HEIGHT_EM, RenderError};
use glyphon::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight,
    cosmic_text::{Fallback, PlatformFallback},
    fontdb,
};
use std::sync::OnceLock;
use unicode_script::Script;

/// Startup typography. Validation is shared by launch parsing and font resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSettings {
    pub family: Option<String>,
    pub fallbacks: Vec<String>,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            family: None,
            fallbacks: Vec::new(),
            size: DEFAULT_METRICS.font_size,
            line_height: DEFAULT_METRICS.height / DEFAULT_METRICS.font_size,
        }
    }
}

impl FontSettings {
    pub fn validate(&self) -> Result<(), RenderError> {
        if !self.size.is_finite()
            || !(6.0..=96.0).contains(&self.size)
            || !self.line_height.is_finite()
            || !(1.0..=3.0).contains(&self.line_height)
            || self.fallbacks.len() > 8
            || self.family.iter().chain(&self.fallbacks).any(|name| {
                name.is_empty()
                    || name.len() > 128
                    || name.trim() != name
                    || name.chars().any(char::is_control)
            })
        {
            return Err(RenderError("invalid terminal typography: font size must be 6..96, line height 1..3, with at most eight fallback families and nonempty trimmed family names up to 128 bytes without controls".into()));
        }
        Ok(())
    }
}

struct OrderedFallback {
    names: Vec<&'static str>,
    common: Vec<&'static str>,
    // Script is a u8 in the exact selected unicode-script release.
    scripts: [OnceLock<Vec<&'static str>>; 256],
}

impl Fallback for OrderedFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }
    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        self.scripts[script as usize].get_or_init(|| {
            self.names
                .iter()
                .copied()
                .chain(
                    PlatformFallback
                        .script_fallback(script, locale)
                        .iter()
                        .copied(),
                )
                .collect()
        })
    }
}

/// Resolved fonts are admitted before native window creation and moved into its renderer.
pub struct FontSetup {
    pub(super) font_system: FontSystem,
    logical: CellMetrics,
    pub(super) family: Option<&'static str>,
    pub(super) configured: bool,
}

impl FontSetup {
    pub fn new(settings: &FontSettings) -> Result<Self, RenderError> {
        Self::resolve(settings, FontSystem::new())
    }

    pub(super) fn resolve(
        settings: &FontSettings,
        mut font_system: FontSystem,
    ) -> Result<Self, RenderError> {
        settings.validate()?;
        for (index, name) in settings
            .family
            .iter()
            .chain(&settings.fallbacks)
            .enumerate()
        {
            let id = font_system
                .db()
                .query(&fontdb::Query {
                    families: &[Family::Name(name)],
                    ..Default::default()
                })
                .ok_or_else(|| {
                    RenderError(format!("terminal font family is unavailable: {name}"))
                })?;
            if index == 0
                && settings.family.is_some()
                && !font_system.db().face(id).unwrap().monospaced
            {
                return Err(RenderError(format!(
                    "terminal primary font is not monospace: {name}"
                )));
            }
            if font_system.get_font(id, Weight::NORMAL).is_none() {
                return Err(RenderError(format!(
                    "cannot load terminal font family: {name}"
                )));
            }
        }
        let family = if settings.family.is_some() || !settings.fallbacks.is_empty() {
            if font_system
                .db()
                .query(&fontdb::Query {
                    families: &[Family::SansSerif],
                    ..Default::default()
                })
                .is_none()
            {
                // Resolve UI text through the original platform fallback before
                // terminal-only fallbacks can select a symbol face for ASCII.
                let mut buffer =
                    Buffer::new(&mut font_system, Metrics::new(settings.size, settings.size));
                buffer.set_text(
                    " ",
                    &Attrs::new().family(Family::SansSerif),
                    Shaping::Basic,
                    None,
                );
                buffer.shape_until_scroll(&mut font_system, false);
                if let Some(id) = buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs)
                    .next()
                    .map(|glyph| glyph.font_id)
                {
                    let name = font_system.db().face(id).unwrap().families[0].0.clone();
                    font_system.db_mut().set_sans_serif_family(name);
                }
            }
            let primary = if let Some(name) = &settings.family {
                name.clone()
            } else {
                let mut buffer =
                    Buffer::new(&mut font_system, Metrics::new(settings.size, settings.size));
                buffer.set_text(
                    " ",
                    &Attrs::new().family(Family::Monospace),
                    Shaping::Advanced,
                    None,
                );
                buffer.shape_until_scroll(&mut font_system, false);
                let id = buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs)
                    .next()
                    .ok_or_else(|| RenderError("no usable monospace font is installed".into()))?
                    .font_id;
                font_system.db().face(id).unwrap().families[0].0.clone()
            };
            // ponytail: cosmic-text requires static family names; at most nine bounded
            // startup names live for this process. Revisit with upstream runtime lists/live reload.
            let primary: &'static str = Box::leak(primary.into_boxed_str());
            let names: Vec<&'static str> = settings
                .fallbacks
                .iter()
                .map(|name| &*Box::leak(name.clone().into_boxed_str()))
                .collect();
            let common = names
                .iter()
                .copied()
                .chain(PlatformFallback.common_fallback().iter().copied())
                .collect();
            let (locale, db) = font_system.into_locale_and_db();
            font_system = FontSystem::new_with_locale_and_db_and_fallback(
                locale,
                db,
                OrderedFallback {
                    names,
                    common,
                    scripts: std::array::from_fn(|_| OnceLock::new()),
                },
            );
            Some(primary)
        } else {
            None
        };
        let width = if let Some(family) = family {
            let mut buffer =
                Buffer::new(&mut font_system, Metrics::new(settings.size, settings.size));
            buffer.set_text(
                " ",
                &Attrs::new().family(Family::Name(family)),
                Shaping::Advanced,
                None,
            );
            buffer
                .line_layout(&mut font_system, 0)
                .and_then(|lines| lines.first())
                .map(|line| line.w)
                .filter(|width| width.is_finite() && *width > 0.0)
                .ok_or_else(|| {
                    RenderError("terminal primary font has no usable cell advance".into())
                })?
        } else {
            DEFAULT_METRICS.width * settings.size / DEFAULT_METRICS.font_size
        };
        let title_cap_top_em = {
            let mut buffer =
                Buffer::new(&mut font_system, Metrics::new(settings.size, settings.size));
            buffer.set_text(
                "H",
                &Attrs::new().family(Family::SansSerif),
                Shaping::Basic,
                None,
            );
            buffer.shape_until_scroll(&mut font_system, false);
            let face = buffer
                .layout_runs()
                .flat_map(|run| run.glyphs)
                .next()
                .and_then(|glyph| font_system.get_font(glyph.font_id, Weight::NORMAL));
            face.map(|face| {
                let metrics = face.metrics();
                let units = f32::from(metrics.units_per_em).max(1.0);
                let cap = metrics
                    .cap_height
                    .map(|cap| cap / units)
                    .filter(|cap| cap.is_finite() && *cap > 0.0)
                    // shortcut: faces without cap metadata use 0.7 em; use their metric when supplied.
                    .unwrap_or(FALLBACK_CAP_HEIGHT_EM);
                (metrics.ascent + metrics.descent) / (2.0 * units) - cap
            })
        };
        Ok(Self {
            font_system,
            logical: CellMetrics {
                scale: 1.0,
                width,
                height: settings.size * settings.line_height,
                font_size: settings.size,
                title_cap_top_em,
                padding: DEFAULT_METRICS.padding,
            },
            family,
            configured: settings != &FontSettings::default(),
        })
    }

    #[must_use]
    pub fn metrics(&self, scale: f64) -> CellMetrics {
        self.logical.scaled(scale)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{cells::NERD_FONT_FAMILY, shaping};
    use super::*;

    #[test]
    fn eon_bar_title_metrics_match_shaped_capitals() {
        for line_height in [1.125, 2.0] {
            let mut fonts = FontSetup::new(&FontSettings {
                line_height,
                ..FontSettings::default()
            })
            .unwrap();
            for scale in [1.0, 1.3] {
                let metrics = fonts.metrics(scale);
                let title = crate::scene::chrome_title_metrics(metrics);
                let mut buffer = Buffer::new(
                    &mut fonts.font_system,
                    Metrics::new(title.font_size, title.line_height),
                );
                buffer.set_text(
                    "H",
                    &Attrs::new().family(Family::SansSerif),
                    Shaping::Basic,
                    None,
                );
                buffer.shape_until_scroll(&mut fonts.font_system, false);
                let run = buffer.layout_runs().next().unwrap();
                let id = run.glyphs[0].font_id;
                let face = fonts.font_system.get_font(id, Weight::NORMAL).unwrap();
                let face_metrics = face.metrics();
                let cap = face_metrics
                    .cap_height
                    .map_or(FALLBACK_CAP_HEIGHT_EM, |cap| {
                        cap / f32::from(face_metrics.units_per_em)
                    });
                let cap_top = run.line_y - cap * title.font_size;
                assert!((cap_top - title.cap_top).abs() < 0.001);
                println!(
                    "title face={:?}, cap_height={:?}, units={}, scale={scale}, line_height={line_height}, capital top={cap_top}",
                    fonts.font_system.db().face(id).unwrap().families,
                    face_metrics.cap_height,
                    face_metrics.units_per_em
                );
            }
        }
    }

    #[test]
    fn ordered_terminal_fallbacks_preserve_readable_generic_ui_text() {
        let original = FontSystem::new();
        let mut db = fontdb::Database::new();
        for name in ["DejaVu Sans", "DejaVu Sans Mono", NERD_FONT_FAMILY] {
            let face = original
                .db()
                .face(
                    original
                        .db()
                        .query(&fontdb::Query {
                            families: &[Family::Name(name)],
                            ..Default::default()
                        })
                        .expect("the accepted font environment supplies the tested face"),
                )
                .unwrap()
                .clone();
            db.push_face_info(face);
        }
        // Model a platform generic name that is not installed, as with Open Sans.
        db.set_sans_serif_family("Unavailable UI Sans");
        let settings = FontSettings {
            family: Some("DejaVu Sans Mono".into()),
            fallbacks: vec![NERD_FONT_FAMILY.into()],
            ..FontSettings::default()
        };
        let mut fonts = FontSetup::resolve(
            &settings,
            FontSystem::new_with_locale_and_db("en-US".into(), db),
        )
        .unwrap();
        for text in ["Project", "+", "?"] {
            let mut buffer = Buffer::new(&mut fonts.font_system, Metrics::new(16.0, 18.0));
            buffer.set_text(
                text,
                &Attrs::new().family(Family::SansSerif),
                shaping(text),
                None,
            );
            buffer.shape_until_scroll(&mut fonts.font_system, false);
            let glyphs: Vec<_> = buffer.layout_runs().flat_map(|run| run.glyphs).collect();
            assert_eq!(glyphs.len(), text.len());
            assert!(
                glyphs.iter().all(|glyph| glyph.glyph_id != 0),
                "missing UI glyphs in {text:?}"
            );
        }
    }
}
