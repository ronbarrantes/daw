//! The Jam view's pattern editor: the bar picker, the track list with
//! its step rows, and the piano roll, plus the tools and helpers they
//! share (painting steps, dragging notes, the step menu).

use eframe::egui::{self, Color32, RichText, Sense, Vec2};

use crate::audio::Event;
use crate::project::{
    self, move_note, note_head, note_len, set_note_length, Row, Step, ACCENT, BAR_CHOICES, GHOST,
    LANES, MAX_OCTAVES, MAX_RATCHET, MAX_TRACKS, NORMAL, STEPS,
};
use crate::theme::track_color;
use crate::widgets::{
    disclosure_arrow, empty_fill, fold_toggle, level_bar, lit_fill, mini_slider, note_on, playhead,
    NOTE_NAMES,
};
use crate::{chords, drums, notes, App, DELAY_SEND, GROUP_VOLUME, PAN, REVERB_SEND, TRACK_VOLUME};

const SOLO: Color32 = Color32::from_rgb(230, 200, 60);
/// Notes from other tracks, shown faintly in the piano roll.
const OTHER_NOTE: Color32 = Color32::from_rgb(55, 75, 105);

/// Something to do to a track, chosen from its right-click menu.
enum TrackAction {
    Duplicate,
    /// Swap places with the track above (-1) or below (+1).
    Move(isize),
    Group,
    MoveTo(u32),
    Ungroup,
    Delete,
}

/// What clicks and drags do in the step rows and piano rolls, picked in
/// the tool bar: buttons for what Ctrl, Alt and right-click do, so a touch
/// screen can do it all.
#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    /// Click or drag to add notes (and turn steps on or off).
    Draw,
    /// Click to pick notes, drag to box-select (MIDI blocks).
    Select,
    /// Click or drag to remove notes.
    Erase,
}

impl Tool {
    /// The tool bar: `select` adds the Select tool where it applies.
    pub fn bar(ui: &mut egui::Ui, tool: &mut Tool, select: bool) {
        ui.label("Tool");
        let tools = [
            (Tool::Draw, "Draw", "Click or drag to add notes"),
            (
                Tool::Select,
                "Select",
                "Click notes to pick them, drag a box around several;\n\
                 then drag them, or use Delete, Ctrl+D and the arrows",
            ),
            (
                Tool::Erase,
                "Erase",
                "Click or drag over notes to remove them",
            ),
        ];
        // Without Select here, it works (and shows) as Draw.
        let current = if !select && *tool == Tool::Select {
            Tool::Draw
        } else {
            *tool
        };
        for (t, name, hint) in tools {
            if t == Tool::Select && !select {
                continue;
            }
            if ui
                .selectable_label(current == t, name)
                .on_hover_text(hint)
                .clicked()
            {
                *tool = t;
            }
        }
    }
}

/// Where the pattern being edited is playing: how many bars in (parts
/// shorter than that repeat) and the step.
#[derive(Clone, Copy)]
pub struct Playhead {
    pub bar: usize,
    pub step: usize,
}

/// "1 bar", "2 bars"...
/// A piano-roll key's width and a lane's height.
const KEY_WIDTH: f32 = 60.0;
const LANE_HEIGHT: f32 = 10.0;

/// A step's size, the gap between steps, and a track row's height.
const CELL: f32 = 18.0;
const GAP: f32 = 2.0;
const ROW: f32 = 20.0;

/// How wide the part of a row before its steps is, and the name in it.
const HEAD_WIDTH: f32 = 200.0;
const NAME_WIDTH: f32 = 56.0;

/// The space between one bar's steps and the next.
const BAR_GAP: f32 = 8.0;

/// A name as plain text that can be clicked, `width` wide, left-aligned
/// and cut short if it doesn't fit; named for screen readers and tests.
fn name_label(ui: &mut egui::Ui, name: &str, color: Color32, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let font = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout(name.to_string(), font, color, width);
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(rect).galley(pos, galley, color);
    resp
}

/// A small square M or S switch; lit (in `on_color` if given) when on.
fn small_toggle(
    ui: &mut egui::Ui,
    text: &str,
    on: bool,
    on_color: Option<Color32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 15.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let v = ui.visuals();
    let (fill, ink) = if on {
        (
            on_color.unwrap_or(Color32::from_gray(150)),
            Color32::from_gray(25),
        )
    } else if resp.hovered() {
        (v.widgets.hovered.weak_bg_fill, v.text_color())
    } else {
        (v.widgets.inactive.weak_bg_fill, v.weak_text_color())
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(9.0),
        ink,
    );
    resp
}

/// A plain square for a step or a piano-roll cell, `size` big.
fn cell(ui: &mut egui::Ui, size: Vec2, fill: Color32, stroke: egui::Stroke) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, fill);
    if stroke.width > 0.0 {
        painter.rect_stroke(rect, 2.0, stroke);
    }
    resp
}

fn bars_label(bars: usize) -> String {
    format!("{bars} bar{}", if bars == 1 { "" } else { "s" })
}

/// A drag across steps that turns them on or off, like a pencil or an
/// eraser. It stays on the track (and piano-roll row) it started on.
#[derive(Clone, Copy, PartialEq)]
pub struct Paint {
    track: usize,
    /// The piano-roll row, or `None` in the track's step row.
    lane: Option<usize>,
    /// Painting notes in, or erasing them.
    on: bool,
    /// Where the pointer was last frame, so a fast drag doesn't skip steps.
    pub last_x: Option<f32>,
}

impl Paint {
    /// Whether the pointer passed over `rect`'s columns since last frame.
    /// Only the columns count: the paint stays on its row.
    fn crosses(&self, pointer: Option<egui::Pos2>, rect: egui::Rect) -> bool {
        pointer.is_some_and(|pos| {
            let from = self.last_x.unwrap_or(pos.x);
            from.min(pos.x) <= rect.right() && from.max(pos.x) >= rect.left()
        })
    }
}

/// Where the pointer is while the main button is held, for painting.
fn held_pointer(ui: &egui::Ui) -> Option<egui::Pos2> {
    ui.input(|i| {
        i.pointer
            .primary_down()
            .then(|| i.pointer.interact_pos())
            .flatten()
    })
}

/// A piano-roll note being dragged, found by its lane and first step.
#[derive(Clone, Copy)]
pub enum NoteDrag {
    /// By its right edge, longer or shorter.
    Resize { lane: usize, head: usize },
    /// By the rest of it, to another pitch or step; `grab` is how many
    /// steps into the note it was picked up.
    Move {
        lane: usize,
        head: usize,
        grab: usize,
    },
}

/// How close to a note's right end, in points, a drag makes it longer or
/// shorter instead of moving it.
const RESIZE_EDGE: f32 = 9.0;

/// Removes the note at `step` of a lane, and the ties that held it on.
fn erase_note(lane: &mut project::Lane, step: usize) {
    lane[step] = None;
    for later in lane.iter_mut().skip(step + 1) {
        if !later.is_some_and(|s| s.tie) {
            break;
        }
        *later = None;
    }
}

/// Shift-click: a ghost note on an empty or normal step, normal on a ghost.
fn toggle_ghost(step: &mut Option<Step>) {
    *step = Some(match *step {
        Some(s) if s.velocity == GHOST => Step {
            velocity: NORMAL,
            ..s
        },
        Some(s) => Step {
            velocity: GHOST,
            ..s
        },
        None => Step {
            velocity: GHOST,
            ..Step::HIT
        },
    });
}

/// The right-click menu for one step. Returns true if anything changed.
fn step_menu(ui: &mut egui::Ui, step: &mut Step) -> bool {
    let mut changed = false;
    ui.set_min_width(200.0);
    ui.horizontal(|ui| {
        for (label, velocity) in [("Ghost", GHOST), ("Normal", NORMAL), ("Accent", ACCENT)] {
            if ui
                .selectable_label(step.velocity == velocity, label)
                .clicked()
            {
                step.velocity = velocity;
                changed = true;
            }
        }
    });
    let mut row = |ui: &mut egui::Ui, label: &str, hint: &str, widget: egui::Slider| {
        ui.label(label).on_hover_text(hint);
        changed |= ui.add(widget).changed();
    };
    egui::Grid::new("step").num_columns(2).show(ui, |ui| {
        row(
            ui,
            "Velocity",
            "How loud: low = ghost note, 127 = accent",
            egui::Slider::new(&mut step.velocity, 1..=127),
        );
        ui.end_row();
        row(
            ui,
            "Chance",
            "How often this step plays",
            egui::Slider::new(&mut step.chance, 1..=100).suffix("%"),
        );
        ui.end_row();
        row(
            ui,
            "Ratchet",
            "Repeat within the step, like a roll",
            egui::Slider::new(&mut step.ratchet, 1..=MAX_RATCHET).suffix("×"),
        );
        ui.end_row();
        row(
            ui,
            "Nudge",
            "Play a little early (−) or late (+), in % of a step",
            egui::Slider::new(&mut step.nudge, -50..=50).suffix("%"),
        );
        ui.end_row();
    });
    if ui.button("Reset").clicked() {
        *step = Step::HIT;
        changed = true;
    }
    changed
}

/// Small markers shown on a step: ratchet count, chance, nudge.
fn step_marks(s: &Step) -> String {
    let mut marks = String::new();
    if s.ratchet > 1 {
        marks.push_str(&format!("×{}", s.ratchet));
    }
    if s.chance < 100 {
        marks.push('?');
    }
    if s.nudge < 0 {
        marks.push('<');
    } else if s.nudge > 0 {
        marks.push('>');
    }
    marks
}

impl App {
    /// The pattern's length, and which of its bars the editor shows.
    /// The bars shown side by side: two at a time, the pair holding the
    /// bar being edited.
    pub fn shown_bars(&self, pattern: usize) -> Vec<usize> {
        let bars = self.project.pattern_bars(pattern);
        let first = (self.edit_bar.min(bars - 1) / 2) * 2;
        (first..(first + 2).min(bars)).collect()
    }

    /// The pattern's length: 1, 2, 4 or 8 bars. It sits next to the
    /// patterns when they're showing, and above the tracks when not.
    pub fn length_picker(&mut self, ui: &mut egui::Ui, pattern: usize) {
        ui.label("Bars").on_hover_text(
            "How many bars the pattern lasts. Each track can be shorter\n\
             (its button next to S); short parts repeat to fill it.",
        );
        for b in BAR_CHOICES {
            let same =
                (0..self.project.tracks.len()).all(|i| self.project.tracks[i].bars(pattern) == b);
            let hint = format!("Make every track in this pattern {}", bars_label(b));
            if ui
                .selectable_label(same, b.to_string())
                .on_hover_text(hint)
                .clicked()
            {
                self.project.set_pattern_bars(pattern, b);
                self.dirty = true;
            }
        }
    }

    /// The line over the tracks: bar numbers over their steps, Level over
    /// the meters, the Mix switch, and which bars show of a longer pattern.
    pub fn tracks_header(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<Playhead>) {
        let bars = self.project.pattern_bars(pattern);
        let shown = self.shown_bars(pattern);
        let weak = ui.visuals().weak_text_color();
        ui.horizontal(|ui| {
            let small = |t: String| RichText::new(t).size(10.0).color(weak);
            ui.allocate_ui_with_layout(
                Vec2::new(HEAD_WIDTH, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(HEAD_WIDTH);
                    if fold_toggle(ui, true, "Tracks", "Hide the tracks")
                        .on_hover_text("Shortcut: F6")
                        .clicked()
                    {
                        self.view.tracks = false;
                    }
                },
            );
            ui.spacing_mut().item_spacing.x = 0.0;
            for (n, &bar) in shown.iter().enumerate() {
                if n > 0 {
                    ui.add_space(BAR_GAP);
                }
                let width = STEPS as f32 * (CELL + GAP) - GAP;
                let playing = current.is_some_and(|c| c.bar % bars == bar);
                let text = small(format!("{}", bar + 1));
                let text = if playing {
                    text.color(ui.visuals().text_color())
                } else {
                    text
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 14.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(width);
                        ui.label(text);
                    },
                );
            }
            ui.add_space(6.0);
            ui.add_sized([40.0, 14.0], egui::Label::new(small("Level".into())));
            ui.spacing_mut().item_spacing.x = 6.0;
            if self.show_mix {
                ui.add_space(14.0);
                for name in ["Pan", "Reverb", "Delay"] {
                    ui.add_sized([48.0, 14.0], egui::Label::new(small(name.into())));
                }
            }
            ui.toggle_value(&mut self.show_mix, RichText::new("Mix").size(11.0))
                .on_hover_text(
                    "Show each track's pan and its reverb and delay sends\n\
                     as more columns on its row",
                );
            // A longer pattern shows two bars at a time; pick which two.
            if bars > 2 {
                ui.separator();
                for first in (0..bars).step_by(2) {
                    let on = shown.first() == Some(&first);
                    let label = RichText::new(format!("{}-{}", first + 1, first + 2)).size(11.0);
                    if ui
                        .selectable_label(on, label)
                        .on_hover_text("Show these two bars")
                        .clicked()
                    {
                        self.edit_bar = first;
                    }
                }
            }
        });
    }

    /// The track list: group headers, and tracks unless their group is collapsed.
    pub fn track_list(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<Playhead>) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let rows = self.project.rows();
        for &row in &rows {
            // A row's menu can delete, add or regroup tracks; the rows
            // after it are drawn next frame, when they're up to date.
            if self.project.rows() != rows {
                break;
            }
            match row {
                Row::Group(g) => self.group_row(ui, g, pattern, current),
                Row::Track(i) => {
                    let track = &self.project.tracks[i];
                    let group = track.group.and_then(|id| self.project.group(id));
                    if !group.is_some_and(|g| g.collapsed) {
                        self.track_row(ui, i, pattern, current);
                        // The selected melodic track's notes open right
                        // under its row.
                        let melodic = self.project.tracks.get(i).is_some_and(|t| t.is_melodic());
                        if i == self.selected && melodic {
                            self.piano_roll_or_bar(ui, pattern, current);
                        }
                    }
                }
            }
        }
    }

    /// A group header: expand arrow, name, volume, mute, solo, and every
    /// hit in the group combined, lined up with the tracks' steps.
    fn group_row(
        &mut self,
        ui: &mut egui::Ui,
        g: usize,
        pattern: usize,
        current: Option<Playhead>,
    ) {
        let id = self.project.groups[g].id;
        let shown = self.shown_bars(pattern);
        let lit: Vec<[bool; STEPS]> = shown
            .iter()
            .map(|&bar| {
                let mut lit = [false; STEPS];
                for t in self.project.tracks.iter().filter(|t| t.group == Some(id)) {
                    for (step, on) in lit.iter_mut().enumerate() {
                        *on |= t.hit_at(pattern, bar, step);
                    }
                }
                lit
            })
            .collect();
        let _ = current;
        let mut ungroup = false;
        ui.horizontal(|ui| {
            let selected = self.selected_group == Some(id);
            let group = &mut self.project.groups[g];
            let head = Vec2::new(HEAD_WIDTH, ROW);
            ui.allocate_ui_with_layout(
                head,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(HEAD_WIDTH);
                    let hint = if group.collapsed {
                        "Expand"
                    } else {
                        "Collapse"
                    };
                    let resp = disclosure_arrow(ui, group.collapsed).on_hover_text(hint);
                    if resp.clicked() {
                        group.collapsed = !group.collapsed;
                        self.dirty = true;
                    }
                    let color = if selected {
                        Color32::WHITE
                    } else {
                        ui.visuals().text_color()
                    };
                    let resp = name_label(ui, &group.name, color, NAME_WIDTH - 12.0)
                        .on_hover_text("Click for group settings · right-click to ungroup");
                    if resp.clicked() {
                        self.selected_group = Some(id);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Ungroup (keep the tracks)").clicked() {
                            ungroup = true;
                            ui.close_menu();
                        }
                    });
                    if mini_slider(ui, &GROUP_VOLUME, &mut group.volume, 56.0) {
                        self.dirty = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_toggle(ui, "S", group.solo, Some(SOLO))
                            .on_hover_text("Solo the whole group")
                            .clicked()
                        {
                            group.solo = !group.solo;
                            self.dirty = true;
                        }
                        if small_toggle(ui, "M", group.muted, None)
                            .on_hover_text("Mute the whole group")
                            .clicked()
                        {
                            group.muted = !group.muted;
                            self.dirty = true;
                        }
                    });
                },
            );
            ui.spacing_mut().item_spacing.x = GAP;
            // A read-only summary of the group's hits; click to expand.
            for (n, lit) in lit.iter().enumerate() {
                if n > 0 {
                    ui.add_space(BAR_GAP);
                }
                for (step, on) in lit.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(CELL, ROW), Sense::click());
                    let fill = if *on {
                        Color32::from_gray(150)
                    } else {
                        empty_fill(step, true)
                    };
                    ui.painter()
                        .rect_filled(rect.shrink2(Vec2::new(0.0, 6.0)), 1.0, fill);
                    if resp.clicked() && group.collapsed {
                        group.collapsed = false;
                        self.dirty = true;
                    }
                }
            }
        });
        if ungroup {
            self.project.ungroup(id);
            self.selected_group = None;
            self.dirty = true;
        }
    }

    /// One channel-rack row: name, mute, solo, and 16 steps.
    fn track_row(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        pattern: usize,
        current: Option<Playhead>,
    ) {
        let mut action = None;
        let shown = self.shown_bars(pattern);
        let pattern_bars = self.project.pattern_bars(pattern);
        let highlighted =
            (self.selected == i && self.selected_group.is_none()) || self.picked.contains(&i);
        let fill = if self.selected == i && self.selected_group.is_none() {
            ui.visuals().widgets.inactive.weak_bg_fill
        } else {
            Color32::TRANSPARENT
        };
        egui::Frame::none().fill(fill).show(ui, |ui| ui.horizontal(|ui| {
            let groups: Vec<(u32, String)> = self
                .project
                .groups
                .iter()
                .map(|g| (g.id, g.name.clone()))
                .collect();
            let color = track_color(i);
            let track = &mut self.project.tracks[i];
            let head = Vec2::new(HEAD_WIDTH, ROW);
            ui.allocate_ui_with_layout(head, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(HEAD_WIDTH);
                // Tracks in a group are indented under its header.
                let indent = if track.group.is_some() { 12.0 } else { 0.0 };
                ui.add_space(indent);
                let (dot, _) = ui.allocate_exact_size(Vec2::new(8.0, ROW), Sense::hover());
                ui.painter().rect_filled(
                    egui::Rect::from_center_size(dot.center(), Vec2::splat(8.0)),
                    1.0,
                    color,
                );
                let text_color = if highlighted {
                    color
                } else {
                    ui.visuals().text_color()
                };
                let hint = "Click to select · Ctrl/Cmd-click to pick several · right-click for more";
                let resp = name_label(ui, &track.name, text_color, NAME_WIDTH - indent)
                    .on_hover_text(format!("{}\n{hint}", track.name));
                if resp.clicked() {
                    if ui.input(|i| i.modifiers.command) {
                        if let Some(at) = self.picked.iter().position(|&p| p == i) {
                            self.picked.remove(at);
                        } else {
                            self.picked.push(i);
                        }
                    } else {
                        // Silent: switching tracks while jamming shouldn't make
                        // a sound. Tap in the drawer plays one. A synth's
                        // piano roll always opens.
                        if track.is_melodic() {
                            self.view.piano = true;
                        }
                        self.selected = i;
                        self.selected_group = None;
                        self.picked.clear();
                    }
                }
                resp.context_menu(|ui| {
                    // How many bars this track plays before it repeats.
                    ui.horizontal(|ui| {
                        ui.label("Length");
                        let bars = track.bars(pattern);
                        for b in BAR_CHOICES {
                            if ui.selectable_label(bars == b, b.to_string()).clicked() {
                                track.set_bars(pattern, b);
                                self.dirty = true;
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Group with picked tracks  (Ctrl+G)").clicked() {
                        action = Some(TrackAction::Group);
                        ui.close_menu();
                    }
                    if !groups.is_empty() {
                        ui.menu_button("Move to group", |ui| {
                            for (id, name) in &groups {
                                if ui.button(name).clicked() {
                                    action = Some(TrackAction::MoveTo(*id));
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                    if track.group.is_some() && ui.button("Remove from group").clicked() {
                        action = Some(TrackAction::Ungroup);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Duplicate track").clicked() {
                        action = Some(TrackAction::Duplicate);
                        ui.close_menu();
                    }
                    if ui.button("Move up").clicked() {
                        action = Some(TrackAction::Move(-1));
                        ui.close_menu();
                    }
                    if ui.button("Move down").clicked() {
                        action = Some(TrackAction::Move(1));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Delete track").clicked() {
                        action = Some(TrackAction::Delete);
                        ui.close_menu();
                    }
                });
                // Volume sits by the name, so balancing is right there.
                if mini_slider(ui, &TRACK_VOLUME, &mut track.volume, 56.0) {
                    self.dirty = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_toggle(ui, "S", track.solo, Some(SOLO))
                        .on_hover_text("Solo: hear only soloed tracks")
                        .clicked()
                    {
                        track.solo = !track.solo;
                        self.dirty = true;
                    }
                    if small_toggle(ui, "M", track.muted, None)
                        .on_hover_text("Mute")
                        .clicked()
                    {
                        track.muted = !track.muted;
                        self.dirty = true;
                    }
                });
            });
            let spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = GAP;
            for (n, &bar) in shown.iter().enumerate() {
            if n > 0 {
                ui.add_space(BAR_GAP);
            }
            // The piano roll lines its columns up with these.
            self.bar_x[n] = ui.cursor().min.x;
            let track = &mut self.project.tracks[i];
            let bars = track.bars(pattern);
            // A shorter part repeats: it shows the bar it plays there,
            // dimmed, and the playhead where the pattern is.
            let current = current
                .filter(|c| c.bar % pattern_bars == bar)
                .map(|c| c.step);
            let repeat = bar >= bars;
            let melodic = track.is_melodic();
            let lanes = track.lanes();
            let on_color = color;
            let base = track.base_note();
            let Some(grid) = track.grid_mut(pattern, bar) else {
                continue;
            };
            let shift = ui.input(|i| i.modifiers.shift);
            let pointer = held_pointer(ui);
            // A step is a column across every lane, so walk it by index.
            #[allow(clippy::needless_range_loop)]
            for step in 0..STEPS {
                // Every lane with something at this step, lowest note first.
                let notes: Vec<usize> = (0..lanes).filter(|&l| grid[l][step].is_some()).collect();
                let first = notes
                    .iter()
                    .copied()
                    .find(|&l| Step::is_hit(&grid[l][step]));
                // Melodic steps show their chord's name (or their lowest
                // note and how many more); groove settings show as marks.
                let chord = (melodic && notes.len() > 1)
                    .then(|| {
                        let pitches: Vec<u8> = notes.iter().map(|&l| base + l as u8).collect();
                        chords::name(&pitches)
                    })
                    .flatten();
                let text = match first.and_then(|l| grid[l][step].map(|s| (l, s))) {
                    Some((_, s)) if chord.is_some() => {
                        let short = chord.as_ref().map_or("", |c| c.short.as_str());
                        format!("{}{}", chords::fit(short, 4), step_marks(&s))
                    }
                    Some((l, s)) => {
                        let name = if melodic { NOTE_NAMES[l % 12] } else { "" };
                        let more = match notes.len() {
                            1 => String::new(),
                            n => format!("+{}", n - 1),
                        };
                        format!("{name}{more}{}", step_marks(&s))
                    }
                    None if !notes.is_empty() => "–".to_string(),
                    None => String::new(),
                };
                let fill = match first.and_then(|l| grid[l][step]) {
                    Some(s) => lit_fill(on_color, s.velocity),
                    None if !notes.is_empty() => color.gamma_multiply(0.45),
                    None => empty_fill(step, false),
                };
                let fill = if repeat {
                    fill.gamma_multiply(0.6)
                } else {
                    fill
                };
                // Small squares: the note, chord and groove marks show on
                // hover.
                let mut resp = cell(ui, Vec2::splat(CELL), fill, playhead(current == Some(step)));
                if let Some(c) = &chord {
                    resp = resp.on_hover_text(format!("{}  {}", c.long, text));
                } else if !text.is_empty() {
                    resp = resp.on_hover_text(text);
                }
                // Pressing a step turns it on or off; dragging on along the
                // row does the same to every step it passes.
                // (A quick tap can press and release within one frame, so
                // a click starts a paint too.)
                let pressed = resp.is_pointer_button_down_on() || resp.clicked();
                if pressed {
                    // The piano roll follows the bar you touch.
                    self.edit_bar = bar;
                }
                let start = !shift && pressed && self.paint.is_none();
                if start {
                    self.paint = Some(Paint {
                        track: i,
                        lane: None,
                        on: notes.is_empty() && self.tool != Tool::Erase,
                        last_x: None,
                    });
                }
                let painting = self.paint.filter(|p| p.track == i && p.lane.is_none());
                if let Some(p) = painting {
                    if start || p.crosses(pointer, resp.rect) {
                        if p.on && notes.is_empty() {
                            grid[0][step] = Some(Step::HIT);
                            self.dirty = true;
                        } else if !p.on && !notes.is_empty() {
                            for &l in &notes {
                                grid[l][step] = None;
                            }
                            self.dirty = true;
                        }
                    }
                }
                if resp.clicked() && shift {
                    if first.is_none() {
                        grid[0][step] = None;
                        toggle_ghost(&mut grid[0][step]);
                    }
                    for &l in &notes {
                        if Step::is_hit(&grid[l][step]) {
                            toggle_ghost(&mut grid[l][step]);
                        }
                    }
                    self.dirty = true;
                }
                if let Some(l) = first {
                    resp.context_menu(|ui| {
                        let mut edited = grid[l][step].unwrap_or(Step::HIT);
                        if step_menu(ui, &mut edited) {
                            // Apply to every note of a chord at once.
                            for &n in &notes {
                                if Step::is_hit(&grid[n][step]) {
                                    grid[n][step] = Some(edited);
                                }
                            }
                            self.dirty = true;
                        }
                    });
                }
            }
            }
            ui.spacing_mut().item_spacing.x = spacing;
            ui.add_space(4.0);
            level_bar(ui, self.track_levels[i], 40.0);
            // Mix: pan and the effect sends, as more columns on the row.
            if self.show_mix {
                let track = &mut self.project.tracks[i];
                ui.separator();
                for (p, value) in [
                    (&PAN, &mut track.pan),
                    (&REVERB_SEND, &mut track.reverb),
                    (&DELAY_SEND, &mut track.delay),
                ] {
                    if mini_slider(ui, p, value, 48.0) {
                        self.dirty = true;
                    }
                }
            }
        }));
        match action {
            Some(TrackAction::Group) => {
                if !self.picked.contains(&i) {
                    self.picked.push(i);
                }
                self.group_picked();
            }
            Some(TrackAction::MoveTo(id)) => {
                self.project.tracks[i].group = Some(id);
                self.project.prune_groups();
                self.dirty = true;
            }
            Some(TrackAction::Ungroup) => {
                self.project.tracks[i].group = None;
                self.project.prune_groups();
                self.dirty = true;
            }
            Some(TrackAction::Delete) => self.delete_track(i),
            Some(TrackAction::Duplicate) => {
                if let Some(copy) = self.project.duplicate_track(i) {
                    // The engine's voices stay with their slot, so a drone
                    // can't follow its track.
                    self.stop_drone();
                    self.midi.track_inserted(copy);
                    self.selected = copy;
                    self.selected_group = None;
                    self.dirty = true;
                }
            }
            Some(TrackAction::Move(by)) => {
                let to = i as isize + by;
                if to >= 0 && (to as usize) < self.project.tracks.len() {
                    let to = to as usize;
                    self.stop_drone();
                    self.project.swap_tracks(i, to);
                    self.midi.tracks_swapped(i, to);
                    if self.selected == i {
                        self.selected = to;
                    } else if self.selected == to {
                        self.selected = i;
                    }
                    self.dirty = true;
                }
            }
            None => {}
        }
    }

    pub fn add_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            // Select only means something in MIDI blocks; here it draws.
            Tool::bar(ui, &mut self.tool, false);
            ui.separator();
            let full = self.project.tracks.len() >= MAX_TRACKS;
            if ui
                .add_enabled(!full, egui::Button::new("+ Synth"))
                .clicked()
            {
                self.add_synth();
            }
            ui.add_enabled_ui(!full, |ui| {
                ui.menu_button("+ Drum", |ui| {
                    for (i, name) in drums::NAMES.iter().enumerate() {
                        if ui.button(*name).clicked() {
                            self.add_drum(i);
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Add a built-in drum sound");
            });
            let load = ui.add_enabled(!full, egui::Button::new("+ Sample…"));
            if load.on_hover_text("WAV, MP3, FLAC or OGG").clicked() {
                self.pick_samples(ctx);
            }
            if self.loading > 0 {
                ui.spinner();
                ui.weak(format!("Loading {}…", self.loading));
            }
        });
    }

    /// The selected synth's piano roll, or, hidden, a line to bring it back.
    pub fn piano_roll_or_bar(
        &mut self,
        ui: &mut egui::Ui,
        pattern: usize,
        current: Option<Playhead>,
    ) {
        if self.view.piano {
            ui.add_space(2.0);
            self.piano_roll(ui, pattern, current);
            ui.add_space(6.0);
        } else {
            let name = self.project.tracks[self.selected].name.clone();
            if fold_toggle(
                ui,
                false,
                &format!("Piano roll · {name}"),
                "Show the piano roll",
            )
            .on_hover_text("Shortcut: F7")
            .clicked()
            {
                self.view.piano = true;
            }
        }
    }

    pub fn piano_roll(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<Playhead>) {
        let i = self.selected;
        let bar = self.edit_bar;
        // Notes other melodic tracks play in this pattern, by pitch (C..B),
        // shown faintly so parts can be lined up.
        let mut others = [[false; STEPS]; LANES];
        if self.view.others {
            for (j, t) in self.project.tracks.iter().enumerate() {
                if j == i || !t.is_melodic() {
                    continue;
                }
                let Some(grid) = t.grid(pattern, bar) else {
                    continue;
                };
                for (lane, steps) in grid.iter().enumerate() {
                    for (step, value) in steps.iter().enumerate() {
                        if Step::is_hit(value) {
                            others[lane][step] = true;
                        }
                    }
                }
            }
        }
        let track = &mut self.project.tracks[i];
        let bars = track.bars(pattern);
        let current = current
            .filter(|c| c.bar % bars == bar % bars)
            .map(|c| c.step);
        // The chord at the step playing, or else the column hovered last
        // frame, named in full.
        let shown = current.or(self.roll_hover.take());
        let chord = shown.and_then(|step| {
            let grid = track.grid(pattern, bar)?;
            let pitches: Vec<u8> = (0..track.lanes())
                .filter(|&l| grid[l][step].is_some())
                .map(|l| track.note(l))
                .collect();
            chords::name(&pitches)
        });
        ui.horizontal(|ui| {
            let title = format!("Piano roll · {}", track.name);
            if fold_toggle(ui, true, &title, "Hide the piano roll")
                .on_hover_text(
                    "Shortcut: F7\n\
                     Click a cell: add or remove a note · drag a note: move it · drag its\n\
                     right edge: longer or shorter · several in a column make a chord ·\n\
                     right-click a note: groove",
                )
                .clicked()
            {
                self.view.piano = false;
            }
            if let Some(c) = &chord {
                ui.label(RichText::new(format!("Chord: {}", c.long)).color(note_on()));
            }
            ui.label("Octave");
            let top = track.top_octave();
            if ui
                .add(egui::DragValue::new(&mut track.octave).range(0..=top))
                .on_hover_text("The piano roll's lowest octave")
                .changed()
            {
                self.dirty = true;
            }
            ui.label("Octaves");
            if ui
                .add(egui::DragValue::new(&mut track.octaves).range(1..=MAX_OCTAVES))
                .on_hover_text(
                    "How many octaves the piano roll shows. Notes in rows\n\
                     above stay, hidden, if you make it smaller.",
                )
                .changed()
            {
                track.octave = track.octave.min(track.top_octave());
                self.dirty = true;
            }
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        // Rows as short as their cells.
        ui.spacing_mut().interact_size.y = LANE_HEIGHT;
        let base = track.base_note();
        let (shift, command, primary) = ui.input(|i| {
            (
                i.modifiers.shift,
                i.modifiers.command,
                i.pointer.primary_down(),
            )
        });
        // Ctrl-drag, or the Erase tool, erases.
        let command = command || self.tool == Tool::Erase;
        let pointer = held_pointer(ui);
        let lanes = track.lanes();
        // Lanes whose note reaches the end of the bar before, which a tie
        // on this bar's first step can hold on.
        let mut held_over = [false; LANES];
        if !bar.is_multiple_of(bars) {
            if let Some(before) = track.grid(pattern, bar - 1) {
                for (held, steps) in held_over.iter_mut().zip(before) {
                    *held = steps[STEPS - 1].is_some();
                }
            }
        }
        let Some(grid) = track.grid_mut(pattern, bar) else {
            return;
        };
        // Where each step's column and each lane's row is, for working
        // out where a note is being dragged to.
        let mut columns = [0.0_f32; STEPS];
        let mut rows = [0.0_f32; LANES];
        // Highest note on top, like a piano.
        for lane in (0..lanes).rev() {
            let name = NOTE_NAMES[lane % 12];
            let sharp = name.ends_with('#');
            // With more than one octave, each C says which one it is.
            let name = if lane % 12 == 0 && lanes > 12 {
                notes::note_name(base + lane as u8)
            } else {
                name.to_string()
            };
            let note = base + lane as u8;
            ui.horizontal(|ui| {
                let key_fill = if sharp {
                    Color32::from_gray(30)
                } else {
                    Color32::from_gray(62)
                };
                let key = cell(
                    ui,
                    Vec2::new(KEY_WIDTH, LANE_HEIGHT),
                    key_fill,
                    egui::Stroke::NONE,
                );
                ui.painter().text(
                    key.rect.right_center() - Vec2::new(4.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    &name,
                    egui::FontId::proportional(10.0),
                    if sharp {
                        Color32::from_gray(150)
                    } else {
                        Color32::from_gray(230)
                    },
                );
                if key.on_hover_text("Click to hear this note").clicked() {
                    let _ = self.events.try_send(Event::Preview { track: i, note });
                }
                // Line the columns up with the steps above.
                let at = self.bar_x[self.edit_bar % 2];
                ui.add_space((at - ui.cursor().min.x).max(0.0));
                ui.spacing_mut().item_spacing.x = GAP;
                for step in 0..STEPS {
                    let value = grid[lane][step];
                    let fill = match value {
                        Some(s) if s.tie => track_color(i).gamma_multiply(0.45),
                        Some(s) => lit_fill(track_color(i), s.velocity),
                        None if others[lane][step] => OTHER_NOTE,
                        None => empty_fill(step, sharp),
                    };
                    let size = Vec2::new(CELL, LANE_HEIGHT);
                    let mut resp = cell(ui, size, fill, playhead(current == Some(step)));
                    if let Some(s) = value.filter(|s| !s.tie) {
                        let marks = step_marks(&s);
                        if !marks.is_empty() {
                            resp = resp.on_hover_text(marks);
                        }
                    }
                    columns[step] = resp.rect.center().x;
                    rows[lane] = resp.rect.center().y;
                    if resp.hovered() {
                        self.roll_hover = Some(step);
                    }
                    let lane_steps = &mut grid[lane];
                    if value.is_some() && !command && self.paint.is_none() {
                        if let Some(head) = note_head(lane_steps, step) {
                            // The note's right edge changes its length; the
                            // rest of it moves it.
                            let last = head + note_len(lane_steps, head) - 1;
                            let edge = |x: f32| step == last && x > resp.rect.right() - RESIZE_EDGE;
                            let hover = resp.hover_pos().map(|p| p.x);
                            if self.note_drag.is_none() && hover.is_some_and(edge) {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                            } else if self.note_drag.is_none() && hover.is_some() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            if resp.drag_started_by(egui::PointerButton::Primary) {
                                // Where the press began: the pointer has moved
                                // on a little by the time it counts as a drag.
                                let at = ui.input(|i| i.pointer.press_origin()).map(|p| p.x);
                                self.note_drag = Some(if at.is_some_and(edge) {
                                    NoteDrag::Resize { lane, head }
                                } else {
                                    let grab = step - head;
                                    NoteDrag::Move { lane, head, grab }
                                });
                            }
                        }
                    }
                    // Pressing an empty cell adds a note, and dragging on
                    // along the row adds more; Ctrl-drag erases instead.
                    // (Left button only: a right-click opens the note's menu
                    // or holds the note before.)
                    let pressed = (resp.is_pointer_button_down_on() && primary) || resp.clicked();
                    // (Not while a note is being dragged: the step it was
                    // picked up from may be empty now.)
                    let start = !shift
                        && pressed
                        && self.paint.is_none()
                        && self.note_drag.is_none()
                        && (command || value.is_none());
                    if start {
                        self.paint = Some(Paint {
                            track: i,
                            lane: Some(lane),
                            on: !command,
                            last_x: None,
                        });
                        if !command {
                            let _ = self.events.try_send(Event::Preview { track: i, note });
                        }
                    }
                    let painting = self.paint.filter(|p| p.track == i && p.lane == Some(lane));
                    if let Some(p) = painting {
                        if start || p.crosses(pointer, resp.rect) {
                            if p.on && lane_steps[step].is_none() {
                                lane_steps[step] = Some(Step::HIT);
                                self.dirty = true;
                            } else if !p.on && Step::is_hit(&lane_steps[step]) {
                                erase_note(lane_steps, step);
                                self.dirty = true;
                            }
                        }
                    }
                    if resp.clicked() && (shift || self.paint.is_none()) {
                        if shift {
                            if !Step::is_hit(&value) {
                                lane_steps[step] = None;
                            }
                            toggle_ghost(&mut lane_steps[step]);
                        } else if value.is_some() {
                            erase_note(lane_steps, step);
                        }
                        self.dirty = true;
                    } else if Step::is_hit(&value) {
                        if let Some(s) = lane_steps[step].as_mut() {
                            resp.context_menu(|ui| self.dirty |= step_menu(ui, s));
                        }
                    } else if resp.secondary_clicked()
                        && value.is_none()
                        && ((step > 0 && lane_steps[step - 1].is_some())
                            || (step == 0 && held_over[lane]))
                    {
                        // Hold the note before (maybe from the bar before)
                        // through this step.
                        lane_steps[step] = Some(Step::TIE);
                        self.dirty = true;
                    }
                }
            });
        }
        let Some(drag) = self.note_drag else {
            return;
        };
        let pointer = ui.input(|i| i.pointer.primary_down().then_some(i.pointer.interact_pos()));
        let pos = match pointer {
            Some(Some(pos)) => pos,
            Some(None) => return,
            None => {
                self.note_drag = None;
                return;
            }
        };
        // The column and row nearest the pointer.
        let nearest = |at: &[f32], x: f32| {
            (0..at.len())
                .min_by(|&a, &b| (at[a] - x).abs().total_cmp(&(at[b] - x).abs()))
                .unwrap_or(0)
        };
        let column = nearest(&columns, pos.x);
        match drag {
            NoteDrag::Resize { lane, head } => {
                // Never before the note's start.
                if set_note_length(&mut grid[lane], head, column.max(head)) {
                    self.dirty = true;
                }
            }
            NoteDrag::Move { lane, head, grab } => {
                let len = note_len(&grid[lane], head);
                let to_lane = nearest(&rows[..lanes], pos.y);
                let to_head = column.saturating_sub(grab).min(STEPS - len);
                if move_note(grid, (lane, head), (to_lane, to_head)) {
                    self.note_drag = Some(NoteDrag::Move {
                        lane: to_lane,
                        head: to_head,
                        grab,
                    });
                    self.dirty = true;
                    if to_lane != lane {
                        let note = base + to_lane as u8;
                        let _ = self.events.try_send(Event::Preview { track: i, note });
                    }
                }
            }
        }
    }
}
