//! Eon's generated chrome colors. Runtime owns selection; Orbit owns terminal colors.

use crate::Color;
use std::sync::LazyLock;

/// Resolved appearance, independent of Runtime's automatic selection policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    Dark,
    Light,
}

/// Actual contrast of a declared draw pair, on the quantized colors it uses.
#[derive(Debug)]
pub struct PaletteContrast {
    pub foreground: &'static str,
    pub background: &'static str,
    pub foreground_color: Color,
    pub background_color: Color,
    pub ratio: f64,
    pub target: f64,
    /// The source color is adapted to its backing before drawing.
    pub adapted: bool,
}

macro_rules! palette_fields {
    ($($(#[$doc:meta])* $field:ident: $role:ident = $stop:ident;)+) => {
        /// Immutable generated colors. Read the shared `Palette::builtin` handle.
        #[derive(Debug)]
        pub struct Palette {
            $($(#[$doc])* pub $field: Color,)+
            mode: PaletteMode,
        }
        #[derive(Clone, Copy, Debug)]
        enum Role { $($role,)+ }
        impl Role {
            fn stop(self) -> Stop {
                match self { $(Self::$role => Stop::$stop,)+ }
            }
            fn name(self) -> &'static str {
                match self { $(Self::$role => stringify!($field),)+ }
            }
            fn color(self, palette: &Palette) -> Color {
                match self { $(Self::$role => palette.$field,)+ }
            }
        }
        impl Palette {
            fn from_stops(mode: PaletteMode, colors: &[Color]) -> Self {
                Self { mode, $($field: colors[Stop::$stop as usize],)+ }
            }
            #[cfg(test)]
            fn snapshot(&self) -> Vec<(&'static str, Color)> {
                vec![$((stringify!($field), self.$field),)+]
            }
        }
    };
}

palette_fields! {
    /// Opaque Eon Bar, card margins and idle controls.
    bar_background: BarBackground = BarBackground;
    /// Window/card background before a terminal supplies its first frame.
    terminal_fallback: TerminalFallback = BarBackground;
    /// Neutral card, popup, sidebar, dialog and non-critical widget outline.
    card_outline: CardOutline = CardOutline;
    /// Hovered, pressed or keyboard-selected rows, pane titles and controls.
    chrome_highlight: ChromeHighlight = ChromeHighlight;
    /// Fill under a hovered bar control.
    control_hover: ControlHover = ControlHover;
    /// Resting modifier-key capsule in the hints bar.
    hint_keycap: HintKeycap = ControlHover;
    /// Opaque tooltip and keycap-popup surface.
    tooltip_background: TooltipBackground = TooltipBackground;
    /// Native dialog surface.
    dialog_background: DialogBackground = DialogBackground;
    /// Track behind the dialog scroll thumb.
    scrollbar_track: ScrollbarTrack = ScrollbarTrack;
    /// Resting tab and bar-widget capsule fill.
    tab_idle: TabIdle = ControlHover;
    /// Hovered tab or session-label capsule fill.
    tab_hover: TabHover = ChromeHighlight;
    /// Selected tab capsule fill.
    tab_selected: TabSelected = TabSelected;
    /// Pressed session-label capsule fill.
    session_pressed: SessionPressed = TooltipBackground;
    /// Full-emphasis titles and labels in panes, tabs, sidebar, popups and dialogs.
    primary_text: PrimaryText = PrimaryText;
    /// Other pane titles, bar readings, hints, sidebar details and dialog metadata.
    secondary_text: SecondaryText = SecondaryText;
    /// Detail line of a dimmed sidebar row.
    tertiary_text: TertiaryText = TertiaryText;
    /// Resting tab/heading numbers and IDs in other pane titles.
    pane_id: PaneId = PaneId;
    /// Title of a pane whose terminal is not live.
    dead_pane_text: DeadPaneText = DeadPaneText;
    /// Unselected tab labels and hint keys.
    tab_text: TabText = TabText;
    /// Version and clock text, and the Eon bar icon tint.
    version_text: VersionText = VersionText;
    /// Modifier-key legend in a hints-bar keycap.
    hint_modifier_text: HintModifierText = SecondaryText;
    /// Headings and key legends in a keycap popup.
    hint_popup_text: HintPopupText = TabText;
    /// Shortcut legends in the native Shortcuts dialog.
    shortcut_key_text: ShortcutKeyText = SecondaryText;
    /// Dialog scroll thumb and source for the adapted sidebar thumb.
    dialog_scroll_thumb: DialogScrollThumb = SecondaryText;
    /// Action labels beside shortcut legends.
    shortcut_action_text: ShortcutActionText = ShortcutActionText;
    /// Startup status text below the splash title.
    splash_text: SplashText = SplashText;
    /// Startup splash title.
    splash_title: SplashTitle = SplashTitle;
    /// Keyboard-focus outlines and pressed heading-button fill.
    accent: Accent = SplashTitle;
    /// Needs-input attention glyph.
    attention_needs_input: AttentionNeedsInput = AttentionNeedsInput;
    /// Error attention glyph.
    attention_error: AttentionError = AttentionError;
    /// Finished attention glyph.
    attention_finished: AttentionFinished = AttentionFinished;
    /// Quiet Idle dot and Unavailable ring.
    attention_idle: AttentionIdle = SecondaryText;
    /// Text in header and floating scrollback pills.
    scrollback_text: ScrollbackText = ScrollbackText;
    /// Scrollback pill fill in a pane header.
    scrollback_pill_header: ScrollbackPillHeader = TooltipBackground;
    /// Scrollback pill fill over terminal content.
    scrollback_pill_overlay: ScrollbackPillOverlay = ChromeHighlight;
    /// Outline around the hyperlink under the pointer.
    hyperlink_outline: HyperlinkOutline = HyperlinkOutline;
    /// Local notice and hyperlink-preview text.
    notice_text: NoticeText = NoticeText;
    /// Notice-band color before overlay opacity is applied.
    notice_background: NoticeBackground = NoticeBackground;
    /// Codex quota-provider icon tint.
    quota_icon_codex: QuotaIconCodex = QuotaIconCodex;
    /// Claude quota-provider icon tint.
    quota_icon_claude: QuotaIconClaude = QuotaIconClaude;
    /// Google quota-provider Gemini spark tint.
    quota_icon_google: QuotaIconGoogle = QuotaIconGoogle;
    /// Notice-level resource dots and first quota pace anchor.
    load_notice: LoadNotice = AttentionNeedsInput;
    /// High-level resource dots and middle quota pace anchor.
    load_high: LoadHigh = LoadHigh;
    /// Critical resource dots/outlines and final quota pace anchor.
    load_critical: LoadCritical = LoadCritical;
}
macro_rules! calibrated_stops {
    ($($name:ident => ($l:literal, $c:literal, $h:literal, $light:literal, $neutral:literal);)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Stop { $($name,)+ Count }
        // Exact OKLCH calibration preserves dark RGB8. Light/contrast resolution
        // walk these families along lightness, with chroma reduction for gamut fit.
        const RECIPE: [StopRecipe; Stop::Count as usize] = [$(StopRecipe {
            dark: Oklch { l: $l, c: $c, h: $h }, light: $light, neutral: $neutral,
        },)+];
    };
}
calibrated_stops! {
    BarBackground => (0.159392873739, 0.015734258463, 266.594059352138, 0.97, true);
    CardOutline => (0.515632609629, 0.039880919683, 248.606202447752, 0.53, true);
    ChromeHighlight => (0.327579052030, 0.034130637431, 252.182970949565, 0.86, true);
    ControlHover => (0.274017000008, 0.029701010674, 256.849663214620, 0.91, true);
    TooltipBackground => (0.393523334605, 0.042399529820, 248.950586159640, 0.77, true);
    DialogBackground => (0.220579488477, 0.024083433158, 259.291366816012, 0.94, true);
    ScrollbarTrack => (0.389498230486, 0.035699540970, 254.897800816524, 0.8, true);
    TabSelected => (0.481810478333, 0.035828316236, 250.048661655890, 0.63, true);
    PrimaryText => (0.964512633877, 0.007574619626, 241.663507259672, 0.18, true);
    SecondaryText => (0.746521641643, 0.026718321453, 255.591713617189, 0.35, true);
    TertiaryText => (0.702971513363, 0.026038417920, 259.816452679055, 0.4, true);
    PaneId => (0.701739236415, 0.031550043455, 257.779779617373, 0.42, true);
    DeadPaneText => (0.736451261167, 0.112449854187, 20.121102875439, 0.4, false);
    TabText => (0.853555032937, 0.019937436612, 252.897509363475, 0.3, true);
    VersionText => (0.780074029064, 0.077327478744, 217.372116030700, 0.4, false);
    ShortcutActionText => (0.909793871409, 0.013275710673, 251.566376540453, 0.25, true);
    SplashText => (0.838608528321, 0.020567912318, 267.261700583464, 0.3, true);
    SplashTitle => (0.852008660447, 0.118731142477, 163.602593955580, 0.35, false);
    AttentionNeedsInput => (0.853870414095, 0.141014997874, 80.115165404558, 0.43, false);
    AttentionError => (0.711607470663, 0.181221269924, 22.838885625987, 0.43, false);
    AttentionFinished => (0.736885323140, 0.167055509886, 143.647964318390, 0.43, false);
    ScrollbackText => (0.823914521614, 0.057538153844, 235.575986198708, 0.35, false);
    HyperlinkOutline => (0.766420356986, 0.111344238876, 259.884973167887, 0.4, false);
    NoticeText => (0.836779895544, 0.130034348055, 80.710158153247, 0.4, false);
    NoticeBackground => (0.234223970319, 0.021760661227, 82.323047940345, 0.95, false);
    QuotaIconCodex => (0.999999993474, 0.000000000000, 0.000000000000, 0.15, false);
    QuotaIconClaude => (0.672419625905, 0.130840179137, 38.755907245756, 0.4, false);
    QuotaIconGoogle => (0.658687209572, 0.138921358493, 250.246720859154, 0.4, false);
    LoadHigh => (0.770159108622, 0.156375850831, 54.819708389055, 0.43, false);
    LoadCritical => (0.670782258360, 0.216543682367, 25.192635240386, 0.43, false);
}

#[derive(Clone, Copy)]
struct Oklch {
    l: f64,
    c: f64,
    h: f64,
}
#[derive(Clone, Copy)]
struct StopRecipe {
    dark: Oklch,
    light: f64,
    neutral: bool,
}
#[derive(Clone, Copy)]
struct Inputs {
    base_hue: f64,
    base_chroma: f64,
    accent: Color,
}
const INPUTS: Inputs = Inputs {
    base_hue: 255.0,
    base_chroma: 0.03,
    accent: Color {
        r: 126,
        g: 231,
        b: 185,
    },
};
pub(super) static DARK: LazyLock<Palette> = LazyLock::new(|| {
    generate(INPUTS, PaletteMode::Dark, PAIRS).expect("invalid Eon dark palette recipe")
});
static LIGHT: LazyLock<Palette> = LazyLock::new(|| {
    generate(INPUTS, PaletteMode::Light, PAIRS).expect("invalid Eon light palette recipe")
});

#[derive(Clone, Copy, PartialEq, Eq)]
enum Adaptation {
    Raw,
    Surface,
    BarPill,
}
#[derive(Clone, Copy)]
struct Pair {
    foreground: Role,
    background: Role,
    target: f64,
    adaptation: Adaptation,
}
macro_rules! pairs {
    ($($kind:ident, $target:literal, $fg:ident => [$($bg:ident),+];)+) => {
        &[$($(Pair { foreground: Role::$fg, background: Role::$bg,
            target: $target, adaptation: Adaptation::$kind },)+)+]
    };
}
// Badge rows measure the existing adapter, not the unadapted source token.
const PAIRS: &[Pair] = pairs! {
    Raw, 4.5, PrimaryText => [BarBackground, TerminalFallback, TabIdle, TabHover, TabSelected, SessionPressed, ChromeHighlight, ControlHover, TooltipBackground, DialogBackground];
    Raw, 4.5, SecondaryText => [BarBackground, TerminalFallback, TabIdle, ChromeHighlight, ControlHover, DialogBackground];
    Raw, 4.5, TertiaryText => [TerminalFallback, ChromeHighlight, DialogBackground];
    Raw, 4.5, PaneId => [BarBackground, TerminalFallback, TabIdle, TabHover];
    Raw, 4.5, DeadPaneText => [BarBackground, TerminalFallback, ChromeHighlight];
    Raw, 4.5, TabText => [BarBackground, TabIdle, TabHover];
    Raw, 4.5, VersionText => [BarBackground, ControlHover, ChromeHighlight];
    Raw, 4.5, HintModifierText => [HintKeycap, ChromeHighlight];
    Raw, 4.5, HintPopupText => [TooltipBackground];
    Raw, 4.5, ShortcutKeyText => [DialogBackground, ControlHover];
    Raw, 4.5, ShortcutActionText => [DialogBackground, ControlHover];
    Raw, 4.5, SplashTitle => [TerminalFallback];
    Raw, 4.5, SplashText => [TerminalFallback];
    Raw, 4.5, ScrollbackText => [ScrollbackPillHeader, ScrollbackPillOverlay];
    Raw, 4.5, NoticeText => [NoticeBackground];
    Raw, 3.0, CardOutline => [BarBackground, TabIdle, TerminalFallback, DialogBackground];
    Raw, 3.0, Accent => [BarBackground, TerminalFallback, TabIdle, TabHover, TabSelected, SessionPressed, ChromeHighlight, ControlHover, DialogBackground];
    Raw, 3.0, DialogScrollThumb => [ScrollbarTrack, DialogBackground, TerminalFallback];
    Raw, 3.0, HyperlinkOutline => [TerminalFallback];
    Raw, 3.0, QuotaIconCodex => [TabIdle];
    Raw, 3.0, QuotaIconClaude => [TabIdle];
    Raw, 3.0, QuotaIconGoogle => [TabIdle];
    Raw, 3.0, TabSelected => [BarBackground];
    BarPill, 3.0, AttentionNeedsInput => [BarBackground, TabIdle, TabHover, TabSelected, SessionPressed];
    Surface, 3.0, AttentionNeedsInput => [TerminalFallback, ChromeHighlight, DialogBackground];
    BarPill, 3.0, AttentionError => [BarBackground, TabIdle, TabHover, TabSelected, SessionPressed];
    Surface, 3.0, AttentionError => [TerminalFallback, ChromeHighlight, DialogBackground];
    BarPill, 3.0, AttentionFinished => [BarBackground, TabIdle, TabHover, TabSelected, SessionPressed];
    Surface, 3.0, AttentionFinished => [TerminalFallback, ChromeHighlight, DialogBackground];
    BarPill, 3.0, AttentionIdle => [BarBackground, TabIdle, TabHover, TabSelected, SessionPressed];
    Surface, 3.0, AttentionIdle => [TerminalFallback, ChromeHighlight, DialogBackground];
    Raw, 4.5, LoadNotice => [TabIdle];
    Raw, 4.5, LoadHigh => [TabIdle];
    Raw, 4.5, LoadCritical => [TabIdle];
    Raw, 3.0, LoadNotice => [BarBackground];
    Raw, 3.0, LoadHigh => [BarBackground];
    Raw, 3.0, LoadCritical => [BarBackground];
};

impl Palette {
    pub fn builtin(mode: PaletteMode) -> &'static Self {
        match mode {
            PaletteMode::Dark => &DARK,
            PaletteMode::Light => &LIGHT,
        }
    }

    /// Window pace thresholds retain their product meaning; load uses the same colors.
    pub fn quota_pace(&self) -> [(f32, Color); 3] {
        [
            (0.05, self.load_notice),
            (0.10, self.load_high),
            (0.25, self.load_critical),
        ]
    }

    /// Fixed palette pairs, including explicitly adapted attention colors.
    /// Does not certify arbitrary terminal colors, wallpaper or animation alpha.
    pub fn contrasts(&self) -> impl Iterator<Item = PaletteContrast> + '_ {
        PAIRS.iter().map(|pair| pair.measure(self))
    }

    fn validate(&self) -> Result<(), String> {
        for pair in self.contrasts() {
            if pair.ratio < pair.target {
                return Err(format!(
                    "{} on {}: {} < {}",
                    pair.foreground, pair.background, pair.ratio, pair.target
                ));
            }
        }
        let rank = |color| contrast_ratio(color, self.bar_background);
        if !(rank(self.primary_text) > rank(self.secondary_text)
            && rank(self.secondary_text) >= rank(self.tertiary_text)
            && rank(self.secondary_text) >= rank(self.pane_id))
        {
            return Err("text emphasis order collapsed".into());
        }
        for color in [
            self.primary_text,
            self.secondary_text,
            self.tertiary_text,
            self.pane_id,
            self.hint_popup_text,
            self.hint_modifier_text,
            self.shortcut_key_text,
            self.dialog_scroll_thumb,
        ] {
            if color == self.accent || color == self.scrollback_text {
                return Err("text role lost its distinctness".into());
            }
        }
        let order = [self.tab_idle, self.tab_hover, self.tab_selected].map(relative_luminance);
        if !order.windows(2).all(|p| match self.mode {
            PaletteMode::Dark => p[0] < p[1],
            PaletteMode::Light => p[0] > p[1],
        }) {
            return Err("tab state order collapsed".into());
        }
        self.validate_pace()
    }

    fn validate_pace(&self) -> Result<(), String> {
        for pair in self.quota_pace().windows(2) {
            for color in interpolants(pair[0].1, pair[1].1) {
                if contrast_ratio(color, self.tab_idle) < 4.5
                    || contrast_ratio(color, self.bar_background) < 3.0
                {
                    return Err(format!(
                        "quota pace interpolant {color:?} below contrast floor"
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Pair {
    fn measure(self, palette: &Palette) -> PaletteContrast {
        let background = self.background.color(palette);
        let source = self.foreground.color(palette);
        let foreground = match self.adaptation {
            Adaptation::Raw => source,
            Adaptation::Surface => blend_to_contrast(source, background, self.target),
            Adaptation::BarPill => blend_to_contrast(
                blend_to_contrast(source, palette.bar_background, self.target),
                background,
                self.target,
            ),
        };
        // Notices are 0.96-opacity overlays. Both render encodings and desktop
        // endpoints bound their composited luminance; do not test an opaque fiction.
        let background = if matches!(self.foreground, Role::NoticeText) {
            [false, true]
                .into_iter()
                .flat_map(|srgb| {
                    [0, 255].map(move |under| composite(background, 0.96, under, srgb))
                })
                .min_by(|a, b| {
                    contrast_ratio(foreground, *a).total_cmp(&contrast_ratio(foreground, *b))
                })
                .unwrap()
        } else {
            background
        };
        let (background, background_name) = if self.adaptation == Adaptation::BarPill
            && contrast_ratio(foreground, palette.bar_background)
                < contrast_ratio(foreground, background)
        {
            (palette.bar_background, Role::BarBackground.name())
        } else {
            (background, self.background.name())
        };
        PaletteContrast {
            foreground: self.foreground.name(),
            background: background_name,
            foreground_color: foreground,
            background_color: background,
            ratio: contrast_ratio(foreground, background),
            target: self.target,
            adapted: self.adaptation != Adaptation::Raw,
        }
    }
}

fn generate(inputs: Inputs, mode: PaletteMode, pairs: &[Pair]) -> Result<Palette, String> {
    if !inputs.base_hue.is_finite()
        || !(0.0..360.0).contains(&inputs.base_hue)
        || !inputs.base_chroma.is_finite()
        || !(0.0..=0.4).contains(&inputs.base_chroma)
    {
        return Err("invalid palette inputs".into());
    }
    let accent = Oklch::from_rgb(inputs.accent);
    let seeds: Vec<_> = RECIPE
        .iter()
        .enumerate()
        .map(|(index, recipe)| {
            let mut color = recipe.dark;
            if recipe.neutral {
                color.h += inputs.base_hue - INPUTS.base_hue;
                color.c *= inputs.base_chroma / INPUTS.base_chroma;
            }
            if index == Stop::SplashTitle as usize {
                color = accent;
            }
            if mode == PaletteMode::Light {
                color.l = recipe.light;
            }
            color
        })
        .collect();
    let mut colors: Vec<_> = seeds.iter().map(|seed| seed.rgb()).collect();
    for (index, seed) in seeds.into_iter().enumerate() {
        let constraints: Vec<_> = pairs
            .iter()
            .filter(|pair| {
                pair.adaptation == Adaptation::Raw && pair.foreground.stop() as usize == index
            })
            .collect();
        if constraints.is_empty() {
            continue;
        }
        let valid = |color| {
            let mut candidate = colors.clone();
            candidate[index] = color;
            let palette = Palette::from_stops(mode, &candidate);
            constraints
                .iter()
                .all(|pair| pair.measure(&palette).ratio >= pair.target)
        };
        if valid(colors[index]) {
            continue;
        }
        // Search a bounded, fine lightness ramp in nearest-first order; validate
        // after gamut mapping and RGB8 quantization, never on ideal OKLCH values.
        let mut nearest = None;
        for step in 0..=4096 {
            let l = f64::from(step) / 4096.0;
            if nearest.is_some_and(|(_, distance)| (l - seed.l).abs() >= distance) {
                continue;
            }
            let color = Oklch { l, ..seed }.rgb();
            if valid(color) {
                nearest = Some((color, (l - seed.l).abs()));
            }
        }
        colors[index] = nearest
            .ok_or_else(|| format!("no color satisfies {:?}", constraints[0].foreground))?
            .0;
    }
    let palette = Palette::from_stops(mode, &colors);
    palette.validate()?;
    Ok(palette)
}

/// Match the sRGB interpolation used by quota pace drawing, including f32 rounding.
pub(super) fn interpolate(start: Color, end: Color, share: f32) -> Color {
    let mix = |a, b| (f32::from(a) + (f32::from(b) - f32::from(a)) * share).round() as u8;
    Color {
        r: mix(start.r, end.r),
        g: mix(start.g, end.g),
        b: mix(start.b, end.b),
    }
}

// Each channel is monotone. Locate every rounding transition in positive f32 bit
// order, so even simultaneous/nearby transitions are covered exactly as drawn.
fn interpolants(start: Color, end: Color) -> impl Iterator<Item = Color> {
    let mut points = vec![0, 1.0f32.to_bits()];
    for (a, b) in [(start.r, end.r), (start.g, end.g), (start.b, end.b)] {
        for step in 1..=a.abs_diff(b) {
            let target = if a < b { a + step } else { a - step };
            let (mut low, mut high) = (0, 1.0f32.to_bits());
            while low < high {
                let middle = low + (high - low) / 2;
                let channel = (f32::from(a)
                    + (f32::from(b) - f32::from(a)) * f32::from_bits(middle))
                .round() as u8;
                let reached = if a < b {
                    channel >= target
                } else {
                    channel <= target
                };
                if reached {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            points.push(low);
        }
    }
    points
        .into_iter()
        .map(move |bits| interpolate(start, end, f32::from_bits(bits)))
}

pub(super) fn linear_channel(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
pub(super) fn encoded_channel(value: f64) -> f64 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
pub(super) fn relative_luminance(color: Color) -> f64 {
    0.2126 * linear_channel(color.r)
        + 0.7152 * linear_channel(color.g)
        + 0.0722 * linear_channel(color.b)
}
/// WCAG 2.2 sRGB contrast, unrounded (1 through 21). Inputs are actual drawn colors.
pub fn contrast_ratio(left: Color, right: Color) -> f64 {
    let (a, b) = (relative_luminance(left), relative_luminance(right));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn composite(color: Color, alpha: f64, under: u8, srgb: bool) -> Color {
    let channel = |value| {
        let (value, under) = if srgb {
            (linear_channel(value), linear_channel(under))
        } else {
            (f64::from(value) / 255.0, f64::from(under) / 255.0)
        };
        let value = value * alpha + under * (1.0 - alpha);
        (255.0 * if srgb { encoded_channel(value) } else { value }).round() as u8
    };
    Color {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
    }
}

impl Oklch {
    fn from_rgb(color: Color) -> Self {
        let [r, g, b] = [color.r, color.g, color.b].map(linear_channel);
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
        let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
        let c = a.hypot(b);
        Self {
            l: 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            c: if c < 1e-6 { 0.0 } else { c },
            h: if c < 1e-6 {
                0.0
            } else {
                b.atan2(a).to_degrees().rem_euclid(360.0)
            },
        }
    }
    fn linear_rgb(self) -> [f64; 3] {
        let a = self.c * self.h.to_radians().cos();
        let b = self.c * self.h.to_radians().sin();
        let l = (self.l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
        let m = (self.l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
        let s = (self.l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
        [
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
        ]
    }
    fn gamut_mapped(self) -> Self {
        let fits = |color: Self| {
            color
                .linear_rgb()
                .iter()
                .all(|v| (-1e-7..=1.0000001).contains(v))
        };
        if fits(self) {
            return self;
        }
        let (mut low, mut high) = (0.0, self.c);
        for _ in 0..32 {
            let c = (low + high) / 2.0;
            if fits(Self { c, ..self }) {
                low = c;
            } else {
                high = c;
            }
        }
        Self { c: low, ..self }
    }
    fn rgb(self) -> Color {
        let [r, g, b] = self
            .gamut_mapped()
            .linear_rgb()
            .map(|v| (255.0 * encoded_channel(v.clamp(0.0, 1.0))).round() as u8);
        Color { r, g, b }
    }
}

/// `color` blended toward black or white, whichever contrasts more with `against`, by the least
/// that reaches `ratio` against it; black or white itself when no blend does.
pub(super) fn blend_to_contrast(color: Color, against: Color, ratio: f64) -> Color {
    let black = Color { r: 0, g: 0, b: 0 };
    let white = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    let target = if contrast_ratio(against, black) >= contrast_ratio(against, white) {
        black
    } else {
        white
    };
    (0..=255)
        .map(|amount| {
            let blend = |source: u8, target: u8| {
                ((u16::from(source) * (255 - amount) + u16::from(target) * amount + 127) / 255)
                    as u8
            };
            Color {
                r: blend(color.r, target.r),
                g: blend(color.g, target.g),
                b: blend(color.b, target.b),
            }
        })
        .find(|blended| contrast_ratio(against, *blended) >= ratio)
        .unwrap_or(target)
}

/// Prefer the nearest tint/shade that contrasts with both surfaces. A middle tone can work
/// even when neither black nor white does; return None only when no integer RGB colour works.
pub(super) fn blend_to_contrast_pair(
    color: Color,
    surfaces: [Color; 2],
    ratio: f64,
) -> Option<Color> {
    let valid = |color| {
        surfaces
            .into_iter()
            .all(|surface| contrast_ratio(color, surface) >= ratio)
    };
    if valid(color) {
        return Some(color);
    }
    let [a, b] = surfaces.map(relative_luminance);
    let (low, high) = (a.min(b), a.max(b));
    let intervals = [
        (0.0, (low + 0.05) / ratio - 0.05),
        (ratio * (low + 0.05) - 0.05, (high + 0.05) / ratio - 0.05),
        (ratio * (high + 0.05) - 0.05, 1.0),
    ]
    .into_iter()
    .filter(|(low, high)| low <= high)
    .collect::<Vec<_>>();
    if intervals.is_empty() {
        return None;
    }
    for amount in 1..=255u16 {
        for target in [0, 255] {
            let blend =
                |source| ((u16::from(source) * (255 - amount) + target * amount + 127) / 255) as u8;
            let candidate = Color {
                r: blend(color.r),
                g: blend(color.g),
                b: blend(color.b),
            };
            if valid(candidate) {
                return Some(candidate);
            }
        }
    }
    for channel in 0..=255 {
        let grey = Color {
            r: channel,
            g: channel,
            b: channel,
        };
        if valid(grey) {
            return Some(grey);
        }
    }
    // Very narrow feasible intervals may contain a coloured RGB value but no grey or tint.
    let levels: [f64; 256] = std::array::from_fn(|channel| linear_channel(channel as u8));
    for (low, high) in intervals {
        for (r, red) in levels.iter().enumerate() {
            for (g, green) in levels.iter().enumerate() {
                let partial = 0.2126 * red + 0.7152 * green;
                let first = levels.partition_point(|blue| partial + 0.0722 * blue < low);
                for (b, blue) in levels.iter().enumerate().skip(first).take(2) {
                    if partial + 0.0722 * blue <= high {
                        let candidate = Color {
                            r: r as u8,
                            g: g as u8,
                            b: b as u8,
                        };
                        if valid(candidate) {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    #[test]
    fn generated_palettes_meet_all_declared_contrasts() {
        for mode in [PaletteMode::Dark, PaletteMode::Light] {
            let palette = Palette::builtin(mode);
            assert!(palette.validate().is_ok(), "{:?}", palette.validate());
            for pair in palette.contrasts() {
                assert!(pair.ratio >= pair.target, "{mode:?}: {pair:?}");
            }
        }
    }

    #[test]
    fn hue_and_chroma_inputs_change_neutrals_without_losing_contrast() {
        for mode in [PaletteMode::Dark, PaletteMode::Light] {
            let shifted = generate(
                Inputs {
                    base_hue: 270.0,
                    base_chroma: 0.02,
                    ..INPUTS
                },
                mode,
                PAIRS,
            )
            .unwrap();
            assert_ne!(
                shifted.bar_background,
                Palette::builtin(mode).bar_background
            );
            assert!(shifted.validate().is_ok());
        }
    }

    #[test]
    fn dark_preserves_original_rgb_except_required_outline_correction() {
        let dark = Palette::builtin(PaletteMode::Dark);
        let expected = [
            ("bar_background", (10, 13, 20)),
            ("terminal_fallback", (10, 13, 20)),
            ("card_outline", (86, 106, 126)),
            ("chrome_highlight", (40, 54, 70)),
            ("control_hover", (30, 40, 54)),
            ("hint_keycap", (30, 40, 54)),
            ("tooltip_background", (52, 72, 92)),
            ("dialog_background", (20, 27, 38)),
            ("scrollbar_track", (56, 70, 88)),
            ("tab_idle", (30, 40, 54)),
            ("tab_hover", (40, 54, 70)),
            ("tab_selected", (79, 96, 114)),
            ("session_pressed", (52, 72, 92)),
            ("primary_text", (239, 244, 248)),
            ("secondary_text", (162, 174, 190)),
            ("tertiary_text", (150, 160, 176)),
            ("pane_id", (147, 160, 179)),
            ("dead_pane_text", (232, 140, 140)),
            ("tab_text", (198, 208, 220)),
            ("version_text", (124, 196, 216)),
            ("hint_modifier_text", (162, 174, 190)),
            ("hint_popup_text", (198, 208, 220)),
            ("shortcut_key_text", (162, 174, 190)),
            ("dialog_scroll_thumb", (162, 174, 190)),
            ("shortcut_action_text", (219, 226, 234)),
            ("splash_text", (196, 202, 216)),
            ("splash_title", (126, 231, 185)),
            ("accent", (126, 231, 185)),
            ("attention_needs_input", (255, 196, 87)),
            ("attention_error", (255, 107, 107)),
            ("attention_finished", (96, 196, 96)),
            ("attention_idle", (162, 174, 190)),
            ("scrollback_text", (162, 204, 230)),
            ("scrollback_pill_header", (52, 72, 92)),
            ("scrollback_pill_overlay", (40, 54, 70)),
            ("hyperlink_outline", (137, 180, 250)),
            ("notice_text", (245, 192, 94)),
            ("notice_background", (35, 29, 18)),
            ("quota_icon_codex", (255, 255, 255)),
            ("quota_icon_claude", (217, 119, 87)),
            ("quota_icon_google", (71, 150, 227)),
            ("load_notice", (255, 196, 87)),
            ("load_high", (255, 150, 70)),
            ("load_critical", (255, 75, 75)),
        ];
        let actual: Vec<_> = dark
            .snapshot()
            .into_iter()
            .map(|(name, c)| (name, (c.r, c.g, c.b)))
            .collect();
        assert_eq!(actual.len(), expected.len());
        for ((name, after), (expected_name, before)) in actual.into_iter().zip(expected) {
            assert_eq!(name, expected_name);
            if name == "card_outline" {
                let rgb = |(r, g, b)| Color { r, g, b };
                assert!(contrast_ratio(rgb(before), dark.tab_idle) < 3.0);
                assert!(contrast_ratio(rgb(after), dark.tab_idle) >= 3.0);
                assert_eq!(after, (94, 115, 135));
            } else {
                assert_eq!(after, before, "{name}");
            }
        }
        let light = Palette::builtin(PaletteMode::Light);
        for ((name, dark), (_, light)) in dark.snapshot().into_iter().zip(light.snapshot()) {
            eprintln!(
                "{name}: dark=#{:02x}{:02x}{:02x} light=#{:02x}{:02x}{:02x}",
                dark.r, dark.g, dark.b, light.r, light.g, light.b
            );
        }
        assert_eq!(
            dark.quota_pace(),
            [
                (
                    0.05,
                    Color {
                        r: 255,
                        g: 196,
                        b: 87
                    }
                ),
                (
                    0.10,
                    Color {
                        r: 255,
                        g: 150,
                        b: 70
                    }
                ),
                (
                    0.25,
                    Color {
                        r: 255,
                        g: 75,
                        b: 75
                    }
                ),
            ]
        );
    }

    #[test]
    fn damaged_palettes_and_unreachable_constraints_fail() {
        for mode in [PaletteMode::Dark, PaletteMode::Light] {
            let mut palette = generate(INPUTS, mode, PAIRS).unwrap();
            palette.primary_text = palette.bar_background;
            let failure = palette.validate().unwrap_err();
            assert!(failure.contains("primary_text"), "{failure}");
            assert!(failure.contains("bar_background"), "{failure}");
            let impossible = [Pair {
                foreground: Role::PrimaryText,
                background: Role::BarBackground,
                target: 22.0,
                adaptation: Adaptation::Raw,
            }];
            assert!(generate(INPUTS, mode, &impossible).is_err());
        }
        for (hue, chroma) in [(f64::NAN, 0.03), (255.0, f64::INFINITY), (255.0, -1.0)] {
            assert!(
                generate(
                    Inputs {
                        base_hue: hue,
                        base_chroma: chroma,
                        ..INPUTS
                    },
                    PaletteMode::Dark,
                    PAIRS
                )
                .is_err()
            );
        }
        let corrected = generate(
            Inputs {
                accent: Color { r: 0, g: 0, b: 0 },
                ..INPUTS
            },
            PaletteMode::Dark,
            PAIRS,
        )
        .unwrap();
        assert_ne!(corrected.accent, Color { r: 0, g: 0, b: 0 });
        assert!(corrected.validate().is_ok());
    }

    #[test]
    fn color_math_matches_known_values_and_maps_gamut() {
        let black = Color { r: 0, g: 0, b: 0 };
        let white = Color {
            r: 255,
            g: 255,
            b: 255,
        };
        assert_eq!(contrast_ratio(black, white), 21.0);
        assert_eq!(contrast_ratio(black, black), 1.0);
        for (_, color) in Palette::builtin(PaletteMode::Dark).snapshot() {
            assert_eq!(Oklch::from_rgb(color).rgb(), color);
            assert_eq!(contrast_ratio(color, white), contrast_ratio(white, color));
        }
        for l in [0.0, 0.01, 0.5, 0.99, 1.0] {
            for h in [0.0, 90.0, 180.0, 270.0] {
                let mapped = Oklch { l, c: 0.4, h }.gamut_mapped();
                assert!(
                    mapped
                        .linear_rgb()
                        .iter()
                        .all(|v| (-1e-7..=1.0000001).contains(v))
                );
            }
        }
        assert_eq!(Oklch::from_rgb(white).h, 0.0);
        assert!((linear_channel(10) - (10.0 / 255.0) / 12.92).abs() < 1e-15);
    }

    #[test]
    fn pace_inventory_includes_rounding_boundaries_and_sampled_draw_colors() {
        let start = Color { r: 0, g: 255, b: 0 };
        let end = Color {
            r: 255,
            g: 0,
            b: 255,
        };
        assert!(interpolants(start, end).any(|c| c
            == Color {
                r: 128,
                g: 128,
                b: 128
            }));
        for mode in [PaletteMode::Dark, PaletteMode::Light] {
            let pace = Palette::builtin(mode).quota_pace();
            for pair in pace.windows(2) {
                let rgb = |color: Color| (color.r, color.g, color.b);
                let reachable: std::collections::BTreeSet<_> =
                    interpolants(pair[0].1, pair[1].1).map(rgb).collect();
                for step in 0..=65536 {
                    let color = interpolate(pair[0].1, pair[1].1, step as f32 / 65536.0);
                    assert!(
                        reachable.contains(&rgb(color)),
                        "{mode:?}: {step}: {color:?}"
                    );
                }
            }
        }
    }
}
