//! Looks the app can wear, from View > Theme: the system's light or dark,
//! or one of two moods (a black-and-white LED screen, a black-and-orange
//! piece of hardware). Only colors change; where things are stays put.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

use eframe::egui::{self, Color32, Stroke, Visuals};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    /// Light or dark, following the computer.
    System,
    Dark,
    Light,
    /// White on black, like a tiny LED screen.
    Led,
    /// Black with orange lights, like a drum machine.
    Hardware,
}

impl Theme {
    pub const ALL: [Theme; 5] = [
        Theme::System,
        Theme::Dark,
        Theme::Light,
        Theme::Led,
        Theme::Hardware,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Theme::System => "Follow the computer",
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Led => "LED",
            Theme::Hardware => "Hardware",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Theme::System => "Light or dark, like the rest of your computer",
            Theme::Dark => "Dark grays",
            Theme::Light => "A soft off-white with rich colors",
            Theme::Led => "White on black, like a small LED screen",
            Theme::Hardware => "Black with orange lights, like a drum machine",
        }
    }

    /// The word it's remembered by.
    pub fn key(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Led => "led",
            Theme::Hardware => "hardware",
        }
    }

    pub fn from_key(key: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// The theme in use, for colors drawn outside egui's visuals (the steps).
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Whether the light look is showing (Light, or the computer's light).
static LIGHT: AtomicBool = AtomicBool::new(false);

/// Notes whether the light look is showing; set every frame from egui's
/// visuals, since "follow the computer" can switch while the app runs.
pub fn set_light(light: bool) {
    LIGHT.store(light, Relaxed);
}

fn light() -> bool {
    LIGHT.load(Relaxed) && matches!(current(), Theme::System | Theme::Light)
}

/// `color` as a mark (a line, a ring, a lit bar) on the ground: darker on
/// the light look, so it stands out at least 3:1 there too.
pub fn mark(color: Color32) -> Color32 {
    if light() {
        deep(color)
    } else {
        color
    }
}

/// `color` deeper and richer, for the light look: darker but more
/// saturated, so it stays a clear color instead of going muddy the way
/// mixing it with black does.
pub fn deep(color: Color32) -> Color32 {
    let mut c = egui::ecolor::Hsva::from(color);
    c.s = (c.s * 1.3).min(1.0);
    c.v *= 0.86;
    Color32::from(c)
}

/// A knob's body (`hovered` a little lighter or darker), its outline, and
/// its pointer.
pub fn knob(hovered: bool) -> (Color32, Stroke, Color32) {
    if light() {
        (
            Color32::from_gray(if hovered { 214 } else { 226 }),
            Stroke::new(1.0_f32, Color32::from_gray(150)),
            Color32::from_gray(40),
        )
    } else {
        (
            Color32::from_gray(if hovered { 70 } else { 55 }),
            Stroke::NONE,
            Color32::WHITE,
        )
    }
}

/// The scope's screen: its ground, grid, trace and label colors.
pub fn scope() -> [Color32; 4] {
    match current() {
        Theme::Led => [
            Color32::BLACK,
            Color32::from_gray(38),
            Color32::from_gray(235),
            Color32::from_gray(140),
        ],
        Theme::Hardware => [
            Color32::from_gray(10),
            Color32::from_gray(36),
            Color32::from_rgb(255, 106, 61),
            Color32::from_rgb(170, 80, 50),
        ],
        _ if light() => [
            screen_fill(),
            Color32::from_gray(224),
            Color32::from_rgb(20, 135, 70),
            Color32::from_gray(100),
        ],
        _ => [
            Color32::from_rgb(12, 20, 16),
            Color32::from_rgb(28, 48, 36),
            Color32::from_rgb(90, 255, 140),
            Color32::from_rgb(90, 160, 110),
        ],
    }
}

/// The spectrum's screen: its ground, the fill under the line, and the
/// line.
pub fn spectrum() -> [Color32; 3] {
    match current() {
        Theme::Led | Theme::Hardware => {
            let [ground, _, line, _] = scope();
            [ground, line.gamma_multiply(0.35).to_opaque(), line]
        }
        _ if light() => [
            screen_fill(),
            Color32::from_rgb(170, 200, 232),
            Color32::from_rgb(30, 100, 170),
        ],
        _ => [
            Color32::from_rgb(12, 16, 24),
            Color32::from_rgb(40, 90, 140),
            Color32::from_rgb(120, 190, 255),
        ],
    }
}

/// The master meter's ground and outline.
pub fn meter() -> (Color32, Color32) {
    if light() {
        (Color32::from_gray(222), Color32::from_gray(190))
    } else {
        (Color32::from_gray(40), Color32::from_gray(70))
    }
}

pub fn current() -> Theme {
    Theme::ALL[CURRENT.load(Relaxed) as usize % Theme::ALL.len()]
}

/// Puts `theme` on.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    CURRENT.store(theme as u8, Relaxed);
    let (dark, light) = match theme {
        Theme::Led => (led(), Visuals::light()),
        Theme::Hardware => (hardware(), Visuals::light()),
        _ => (flat_dark(), flat_light()),
    };
    let (mut dark, mut light) = (dark, light);
    // Sliders fill up to their handle, like the track sliders.
    dark.slider_trailing_fill = true;
    light.slider_trailing_fill = true;
    ctx.set_visuals_of(egui::Theme::Dark, dark);
    ctx.set_visuals_of(egui::Theme::Light, light);
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        _ => egui::ThemePreference::Dark,
    });
}

/// Dark visuals with these grounds, widget fills and highlight.
fn shades(
    ground: Color32,
    panel: Color32,
    widget: Color32,
    hover: Color32,
    text: Color32,
) -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = ground;
    v.faint_bg_color = widget;
    v.code_bg_color = ground;
    let w = &mut v.widgets;
    // Text color by state, so a selected button can still have dark text.
    for state in [&mut w.noninteractive, &mut w.inactive] {
        state.fg_stroke.color = text;
    }
    w.hovered.fg_stroke.color = Color32::WHITE;
    w.active.fg_stroke.color = Color32::WHITE;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.inactive.bg_fill = widget;
    w.inactive.weak_bg_fill = widget;
    w.hovered.bg_fill = hover;
    w.hovered.weak_bg_fill = hover;
    w.active.bg_fill = hover;
    w.active.weak_bg_fill = hover;
    v
}

/// The everyday dark look: flat grays, no outlines on buttons.
fn flat_dark() -> Visuals {
    let mut v = shades(
        Color32::from_rgb(21, 22, 24),
        Color32::from_rgb(42, 43, 46),
        Color32::from_rgb(55, 56, 61),
        Color32::from_rgb(69, 71, 77),
        Color32::from_rgb(214, 215, 218),
    );
    v.window_fill = Color32::from_rgb(47, 48, 52);
    v.faint_bg_color = Color32::from_rgb(38, 39, 41);
    v.selection.bg_fill = Color32::from_rgb(69, 71, 77);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    let w = &mut v.widgets;
    for state in [&mut w.inactive, &mut w.noninteractive] {
        state.bg_stroke = Stroke::NONE;
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(27, 28, 31));
    v
}

/// The everyday light look: a soft off-white, not paper white, with flat
/// gray buttons and a deep blue for what's picked (white text on it).
fn flat_light() -> Visuals {
    let mut v = Visuals::light();
    let text = Color32::from_gray(34);
    v.panel_fill = Color32::from_rgb(236, 236, 233);
    v.window_fill = Color32::from_rgb(242, 242, 239);
    v.extreme_bg_color = Color32::from_rgb(248, 248, 246);
    v.faint_bg_color = Color32::from_rgb(230, 230, 227);
    v.code_bg_color = v.faint_bg_color;
    v.selection.bg_fill = Color32::from_rgb(46, 94, 170);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(30, 90, 180);
    let w = &mut v.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive] {
        state.fg_stroke.color = text;
        state.bg_stroke = Stroke::NONE;
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(212));
    w.noninteractive.bg_fill = v.panel_fill;
    w.noninteractive.weak_bg_fill = v.panel_fill;
    w.inactive.bg_fill = Color32::from_gray(222);
    w.inactive.weak_bg_fill = Color32::from_gray(222);
    w.hovered.bg_fill = Color32::from_gray(208);
    w.hovered.weak_bg_fill = Color32::from_gray(208);
    w.hovered.fg_stroke.color = Color32::BLACK;
    w.active.bg_fill = Color32::from_gray(196);
    w.active.weak_bg_fill = Color32::from_gray(196);
    w.active.fg_stroke.color = Color32::BLACK;
    v
}

fn led() -> Visuals {
    let white = Color32::from_gray(235);
    let mut v = shades(
        Color32::BLACK,
        Color32::BLACK,
        Color32::from_gray(26),
        Color32::from_gray(60),
        white,
    );
    // A lit gray, not white: some selected buttons keep white text.
    v.selection.bg_fill = Color32::from_gray(95);
    v.selection.stroke = Stroke::new(1.0_f32, white);
    v.hyperlink_color = white;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(80));
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(110));
    v
}

fn hardware() -> Visuals {
    let orange = Color32::from_rgb(255, 106, 61);
    let mut v = shades(
        Color32::from_gray(8),
        Color32::from_gray(30),
        Color32::from_gray(46),
        Color32::from_gray(64),
        Color32::from_gray(225),
    );
    v.selection.bg_fill = orange;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::BLACK);
    v.hyperlink_color = orange;
    v
}

/// Each track's color, by its place in the list: what its steps, notes
/// and name light up in.
pub fn track_color(i: usize) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(217, 119, 74),
        Color32::from_rgb(217, 164, 65),
        Color32::from_rgb(184, 191, 92),
        Color32::from_rgb(95, 179, 161),
        Color32::from_rgb(224, 122, 143),
        Color32::from_rgb(127, 163, 230),
        Color32::from_rgb(166, 138, 216),
        Color32::from_rgb(111, 192, 216),
    ];
    match current() {
        Theme::Led => Color32::from_gray(235),
        Theme::Hardware => Color32::from_rgb(255, 106, 61),
        // Darker on the light look, so a lit step stands out at least 3:1
        // from an empty one.
        _ if light() => deep(COLORS[i % COLORS.len()]),
        _ => COLORS[i % COLORS.len()],
    }
}

/// An empty step: every other group of four a shade apart, so the beats
/// read; `dark` for a piano roll's sharp lanes.
pub fn step_empty(step: usize, dark: bool) -> Color32 {
    let first = (step / 4).is_multiple_of(2);
    let gray = match (current(), light()) {
        (Theme::Led, _) => [40, 32, 28, 22],
        (Theme::Hardware, _) => [58, 50, 46, 40],
        (_, true) => [222, 208, 200, 190],
        _ => [66, 58, 52, 46],
    };
    let at = match (dark, first) {
        (false, true) => 0,
        (false, false) => 1,
        (true, true) => 2,
        (true, false) => 3,
    };
    Color32::from_gray(gray[at])
}

/// Another track's note, shown faintly behind the piano roll's own.
pub fn other_note() -> Color32 {
    if light() {
        Color32::from_gray(176)
    } else {
        Color32::from_rgb(55, 75, 105)
    }
}

/// Where a lit step's color starts from at the softest velocity.
pub fn step_base() -> Color32 {
    if light() {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(40)
    }
}

/// The background of the drawings (envelope, filter, LFO).
pub fn screen_fill() -> Color32 {
    match current() {
        Theme::Led => Color32::BLACK,
        Theme::Hardware => Color32::from_gray(10),
        _ if light() => Color32::from_rgb(248, 248, 246),
        _ => Color32::from_gray(30),
    }
}

/// The piano roll's keys: the white keys' fill, the black keys', and the
/// note names' color (on the white part). `hovered` lights them a little.
pub fn piano_keys(hovered: bool) -> (Color32, Color32, Color32) {
    let lift = if hovered { 14 } else { 0 };
    if light() {
        (
            Color32::from_gray(226 - lift),
            Color32::from_gray(50 + lift),
            Color32::from_gray(60),
        )
    } else {
        (
            Color32::from_gray(196 + lift),
            Color32::from_gray(26 + lift),
            Color32::from_gray(50),
        )
    }
}

/// A lit drum step.
pub fn drum_on() -> Color32 {
    match current() {
        Theme::Led => Color32::from_gray(235),
        Theme::Hardware => Color32::from_rgb(255, 106, 61),
        _ => Color32::from_rgb(255, 140, 40),
    }
}

/// A lit note.
pub fn note_on() -> Color32 {
    match current() {
        Theme::Led => Color32::from_gray(235),
        Theme::Hardware => Color32::from_gray(235),
        _ => mark(Color32::from_rgb(80, 180, 255)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_is_remembered_by_its_own_word() {
        for t in Theme::ALL {
            assert_eq!(Theme::from_key(t.key()), Some(t));
        }
        assert_eq!(Theme::from_key("nope"), None);
    }
}
