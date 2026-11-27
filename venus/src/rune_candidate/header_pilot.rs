use super::{FontSetup, HOSTED_LOOK, HostedLook, Result};
use crate::{Color, PanelHeading, SceneRect};
use rune::{
    Align, El, Family, TextStyle,
    color::srgb_decode,
    effect, el,
    render::{Instance, KIND_EFFECT, KIND_TRIANGLE},
};
use std::{io::Read, sync::Arc};

const OUTLINE: u32 = 2;
pub(crate) const TITLE: u32 = 3;

pub(crate) fn fonts(
    fonts: &mut FontSetup,
    register: bool,
    path: Option<std::ffi::OsString>,
) -> Result<&'static str> {
    let mut bytes = Vec::new();
    const LIMIT: usize = 64 * 1024 * 1024;
    std::fs::File::open(path.ok_or("missing EON_VENUS_TEST_SANS_FONT")?)?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > LIMIT {
        return Err("invalid packaged header font length".into());
    }
    let bytes = Arc::<[u8]>::from(bytes);
    let family = fonts.set_packaged_header_font(Arc::clone(&bytes))?;
    if family != "DejaVu Sans" || (register && !rune::text::add_font(bytes).contains(&family)) {
        return Err("Rune did not register the packaged header family".into());
    }
    Ok("DejaVu Sans")
}

pub(super) fn look() -> Result<HostedLook> {
    match std::env::var("EON_VENUS_TEST_HEADER_LOOK").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("notch") => Ok(HostedLook {
            background_opacity: 0.8,
            pane_frames: true,
            panel_heading: PanelHeading::Notch,
            border_width: 2,
            focus_border_width: 2,
            ..HOSTED_LOOK
        }),
        Ok("connected") => Ok(HOSTED_LOOK),
        _ => Err("expected EON_VENUS_TEST_HEADER_LOOK=notch|connected".into()),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Title {
    pub(crate) source: String,
    pub(crate) origin: [f32; 2],
    pub(crate) clip: SceneRect,
    pub(crate) scale: f32,
    pub(crate) size: f32,
    pub(crate) weight: f32,
    pub(crate) line_height: f32,
    pub(crate) color: Color,
}

impl Title {
    pub(crate) fn style(&self, family: &'static str) -> TextStyle {
        TextStyle::new(self.size / self.scale, self.weight)
            .lh(self.line_height / self.scale)
            .family(Family::Named(family))
    }

    pub(crate) fn view(&self, family: &'static str) -> El {
        let Self {
            source,
            origin: [x, y],
            clip,
            scale,
            line_height,
            color,
            ..
        } = self;
        let width = clip.width / scale;
        let height = clip.height / scale;
        el().absolute(clip.left / scale, clip.top / scale)
            .w(width)
            .h(height)
            .clip_rounded(0.0)
            .child(
                rune::text(source.clone())
                    .key("popup-title")
                    .mark(TITLE)
                    .nowrap()
                    .absolute((x - clip.left) / scale, (y - clip.top) / scale)
                    .w((clip.right() - x) / scale)
                    .h(line_height / scale)
                    .align(Align::Start)
                    .align_y(Align::Start)
                    .color(linear(*color))
                    .font(self.style(family)),
            )
    }
}

#[derive(Default)]
pub(crate) struct HeaderPaint {
    pub(crate) title: Option<Title>,
    pub(crate) outline: Vec<(SceneRect, Color, f32)>,
    pub(crate) later_text: usize,
    pub(crate) overlap: bool,
}

impl HeaderPaint {
    pub(crate) fn overlaps(&self, bounds: SceneRect, check_title: bool) -> bool {
        self.outline
            .iter()
            .any(|(rect, _, _)| rect.intersection(bounds).is_some())
            || (check_title
                && self
                    .title
                    .as_ref()
                    .is_some_and(|title| title.clip.intersection(bounds).is_some()))
    }

    pub(crate) fn overlaps_points(&self, points: impl Iterator<Item = [f32; 2]>) -> bool {
        let (lo, hi) = points.fold(
            ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
            |(lo, hi), [x, y]| ([lo[0].min(x), lo[1].min(y)], [hi[0].max(x), hi[1].max(y)]),
        );
        self.overlaps(
            SceneRect {
                left: lo[0],
                top: lo[1],
                width: hi[0] - lo[0],
                height: hi[1] - lo[1],
            },
            false,
        )
    }
}

pub(super) fn marker() -> El {
    effect(OUTLINE).absolute(0.0, 0.0).w_full().h_full()
}

pub(crate) fn splice<S: 'static>(
    rt: &rune::Runtime<S>,
    paint: Option<&HeaderPaint>,
    out: &mut Vec<Instance>,
) -> bool {
    out.clear();
    if rt.marks.len() != rt.instances.len() {
        return false;
    }
    for (instance, &mark) in rt.instances.iter().zip(&rt.marks) {
        if instance.params[1] == KIND_EFFECT && instance.border[..2] == [OUTLINE as f32, 0.0] {
            if let Some(paint) = paint {
                for &(rect, color, alpha) in &paint.outline {
                    let (x, y, r, b) = (rect.left, rect.top, rect.right(), rect.bottom());
                    let triangle = |radii, uv| Instance {
                        rect: [x, y, rect.width, rect.height],
                        radii,
                        uv,
                        fill: linear(color).premul(alpha),
                        border: [0.0; 4],
                        params: [0.0, KIND_TRIANGLE, 0.0, 1.0],
                        clip: instance.clip,
                        fade: [0; 4],
                    };
                    out.push(triangle([x, y, x, b], [r, b, 0.0, 0.0]));
                    out.push(triangle([x, y, r, b], [r, y, 0.0, 0.0]));
                }
            }
        } else if mark != TITLE || paint.and_then(|paint| paint.title.as_ref()).is_some() {
            out.push(*instance);
        }
    }
    true
}

fn linear(color: Color) -> rune::Color {
    let [r, g, b] = [color.r, color.g, color.b].map(|byte| srgb_decode(f32::from(byte) / 255.0));
    rune::Color { r, g, b, a: 1.0 }
}
