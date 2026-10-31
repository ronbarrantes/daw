//! Smoke tests for the UI: they draw the whole app headless, find buttons
//! by their labels (through egui's accessibility tree) and click them like
//! a person would, so a crash in a menu or a view fails a test instead of a
//! release. They check that things don't break, not every detail.

use eframe::egui::{self, accesskit, Pos2, Rect, Vec2};

use crate::project::Project;
use crate::App;

/// The Air's screen.
const SCREEN: Vec2 = Vec2::new(1366.0, 768.0);

struct Harness {
    app: App,
    ctx: egui::Context,
    events: Vec<egui::Event>,
    /// Every labelled widget on screen last frame, and where it was.
    widgets: Vec<(String, accesskit::Role, Rect)>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut h = Harness {
            app: App::new(Project::demo(), None, 120.0),
            ctx,
            events: Vec::new(),
            widgets: Vec::new(),
        };
        h.frames(2);
        h
    }

    /// Draws one frame with the input gathered since the last.
    fn frame(&mut self) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        let app = &mut self.app;
        let out = self.ctx.run(input, |ctx| app.draw(ctx));
        if let Some(tree) = out.platform_output.accesskit_update {
            self.widgets = tree
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    let name = node.name()?.to_string();
                    let b = node.bounds()?;
                    let rect = Rect::from_min_max(
                        Pos2::new(b.x0 as f32, b.y0 as f32),
                        Pos2::new(b.x1 as f32, b.y1 as f32),
                    );
                    Some((name, node.role(), rect))
                })
                .collect();
        }
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// Where the widget labelled `label` is, if it's on screen (scrolled
    /// out of view doesn't count). Buttons win over plain text.
    fn find(&self, label: &str) -> Option<Pos2> {
        let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
        let shown = || {
            self.widgets
                .iter()
                .filter(move |(name, _, rect)| name == label && screen.contains(rect.center()))
        };
        shown()
            .find(|(_, role, _)| *role != accesskit::Role::Label)
            .or_else(|| shown().next())
            .map(|(_, _, rect)| rect.center())
    }

    fn press(&mut self, label: &str, button: egui::PointerButton) {
        let Some(pos) = self.find(label) else {
            let mut names: Vec<&str> = self.widgets.iter().map(|w| w.0.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            panic!("no \"{label}\" on screen; there's {names:?}");
        };
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        self.events.push(egui::Event::PointerMoved(pos));
        self.events.push(click(true));
        self.frame();
        self.events.push(click(false));
        self.frames(2);
    }

    fn click(&mut self, label: &str) {
        self.press(label, egui::PointerButton::Primary);
    }

    fn right_click(&mut self, label: &str) {
        self.press(label, egui::PointerButton::Secondary);
    }

    /// Presses or lets go of a key on the computer keyboard.
    fn key(&mut self, key: egui::Key, pressed: bool) {
        self.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        });
        self.frame();
    }

    fn track_names(&self) -> Vec<&str> {
        self.app
            .project
            .tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect()
    }
}

#[test]
fn deleting_a_track_from_its_menu() {
    let mut ui = Harness::new();
    assert_eq!(ui.track_names()[4], "Lead");
    // Lead has Pad after it: this crashed in 0.47.
    ui.right_click("Lead");
    ui.click("Delete track");
    assert!(!ui.track_names().contains(&"Lead"));
    assert_eq!(ui.track_names().len(), 5);
    ui.frames(3);
}

#[test]
fn duplicating_and_moving_tracks_from_their_menu() {
    let mut ui = Harness::new();
    ui.right_click("Kick");
    ui.click("Duplicate track");
    assert_eq!(&ui.track_names()[..2], ["Kick", "Kick 2"]);
    ui.right_click("Kick 2");
    ui.click("Move down");
    assert_eq!(&ui.track_names()[..3], ["Kick", "Snare", "Kick 2"]);
    ui.right_click("Kick 2");
    ui.click("Move up");
    assert_eq!(&ui.track_names()[..2], ["Kick", "Kick 2"]);
}

#[test]
fn longer_patterns_and_their_bars() {
    let mut ui = Harness::new();
    ui.click("2");
    assert_eq!(ui.app.project.pattern_bars(0), 2);
    ui.click("Bar 2");
    assert_eq!(ui.app.edit_bar, 1);
    ui.click("1");
    ui.frames(2);
    assert_eq!(ui.app.edit_bar, 0, "back in range when the bar is gone");
}

#[test]
fn menus_and_views_open() {
    let mut ui = Harness::new();
    ui.click("View");
    ui.click("Tracks only");
    ui.click("View");
    ui.click("Show everything  (F9)");
    ui.click("Edit");
    ui.click("Copy pattern");
    ui.click("Edit");
    ui.click("Paste pattern");
    ui.click("Help");
    ui.click("Diagnostics");
    assert!(
        ui.find("Copy all").is_some(),
        "the Diagnostics window opened"
    );
    ui.click("Arrange");
    ui.frames(3);
    ui.click("Jam");
    ui.frames(3);
}

#[test]
fn keys_1_and_2_bend_the_held_chord_like_a_wheel() {
    use egui::Key;
    let mut ui = Harness::new();
    ui.click("Lead");
    ui.app.bend_keys = false;
    ui.key(Key::A, true);
    ui.key(Key::Num1, true);
    ui.frames(30);
    assert_eq!(ui.app.bend, 0.0, "off: 1 does nothing");
    ui.key(Key::Num1, false);
    ui.key(Key::A, false);
    ui.app.bend_keys = true;
    // A chord on the keys, then the wheel down.
    for k in [Key::A, Key::D, Key::G] {
        ui.key(k, true);
    }
    ui.key(Key::Num1, true);
    ui.frames(30);
    assert_eq!(ui.app.held_keys.len(), 3);
    assert_eq!(ui.app.bend, -ui.app.bend_range, "all the way down");
    // Let go: it springs back to the notes.
    ui.key(Key::Num1, false);
    ui.frames(30);
    assert_eq!(ui.app.bend, 0.0);
    ui.key(Key::Num2, true);
    ui.frames(30);
    assert_eq!(ui.app.bend, ui.app.bend_range, "and up");
}

#[test]
fn the_instrument_panel_slides_shut_and_open() {
    let mut ui = Harness::new();
    assert!(ui.app.view.panel);
    ui.click("Hide the instrument");
    ui.frames(30); // let it slide
    assert!(!ui.app.view.panel);
    ui.click("Show the instrument");
    ui.frames(30);
    assert!(ui.app.view.panel);
    assert!(ui.find("Tap").is_some(), "its controls are back");
}

#[test]
fn mix_adds_pan_and_sends_to_the_rows() {
    let mut ui = Harness::new();
    // The drawer has a Mix section too; shut it so "Mix" is the toggle.
    ui.click("Hide the instrument");
    ui.frames(2);
    assert!(ui.find("Pan").is_none());
    ui.click("Mix");
    ui.frames(2);
    assert!(ui.app.show_mix);
    assert!(ui.find("Pan").is_some(), "each row has a pan slider");
    assert!(ui.find("Volume").is_some(), "and its volume");
}
