//! The timeline, where a song is put together, and the pattern library
//! above the groovebox.
//!
//! Clips on the timeline point to patterns from the library, like Figma
//! instances of a component: edit a pattern and every clip of it changes.
//! "Make unique" gives one clip its own copy. Time is in steps (16ths).

use std::sync::atomic::Ordering::Relaxed;

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::project::{Clip, MAX_SONG_STEPS, STEPS};
use crate::App;

const RULER: f32 = 18.0;
const ROW: f32 = 24.0;
/// Rows shown: the ones in use, one spare, and at least this many.
const MIN_ROWS: usize = 2;
const MAX_ROWS: usize = 8;
/// How close to a clip's edge (in points) a drag trims instead of moving.
const EDGE: f32 = 6.0;
const MIN_ZOOM: f32 = 1.5;
const MAX_ZOOM: f32 = 24.0;

/// What clip moves and trims snap to.
#[derive(Clone, Copy, PartialEq)]
pub enum Snap {
    Bar,
    Beat,
    Step,
}

impl Snap {
    fn steps(self) -> usize {
        match self {
            Snap::Bar => STEPS,
            Snap::Beat => 4,
            Snap::Step => 1,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Snap::Bar => "Bar",
            Snap::Beat => "Beat",
            Snap::Step => "Step",
        }
    }
}

/// A drag in progress on the timeline.
#[derive(Clone, Copy)]
enum Drag {
    /// Moving a clip; `grab` is where it was held, in steps from its start.
    Move {
        clip: usize,
        grab: f32,
    },
    TrimStart {
        clip: usize,
        before: Clip,
    },
    TrimEnd {
        clip: usize,
    },
    /// Marking a loop region on the ruler, from this step.
    Loop {
        from: usize,
    },
}

/// The timeline's view settings and what's selected.
pub struct State {
    /// Points per step.
    zoom: f32,
    pub snap: Snap,
    pub selected: Option<usize>,
    /// Where playback starts in Song mode, and where splits happen when
    /// stopped. Click the ruler to move it.
    pub cursor: usize,
    drag: Option<Drag>,
    /// The clip whose right-click menu is open.
    menu_clip: Option<usize>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            zoom: 6.0,
            snap: Snap::Bar,
            selected: None,
            cursor: 0,
            drag: None,
            menu_clip: None,
        }
    }
}

/// A clip's color, by pattern, so linked clips match.
pub fn pattern_color(p: usize) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(70, 130, 180),
        Color32::from_rgb(190, 110, 50),
        Color32::from_rgb(90, 150, 90),
        Color32::from_rgb(150, 90, 160),
        Color32::from_rgb(170, 70, 80),
        Color32::from_rgb(60, 150, 150),
        Color32::from_rgb(160, 150, 60),
        Color32::from_rgb(110, 110, 170),
    ];
    COLORS[p % COLORS.len()]
}

fn snap_down(step: f32, to: usize) -> usize {
    (step.max(0.0) as usize / to) * to
}

fn snap_near(step: f32, to: usize) -> usize {
    ((step.max(0.0) / to as f32).round() as usize) * to
}

impl App {
    /// The pattern library: pick a pattern to edit, drag one onto the
    /// timeline, right-click to rename, duplicate or delete.
    pub fn library_row(&mut self, ui: &mut egui::Ui) {
        let edit = self.edit_pattern();
        let ctx = ui.ctx().clone();
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Patterns").strong()),
            )
            .on_hover_text(
                "Click a pattern to edit it below. Drag it onto the timeline,\n\
                 or double-click the timeline to place it. Right-click to rename.",
            );
            for p in 0..self.project.patterns.len() {
                let name = &self.project.patterns[p].name;
                let fill = pattern_color(p).gamma_multiply(if p == edit { 1.0 } else { 0.45 });
                let button = egui::Button::new(egui::RichText::new(name).color(Color32::WHITE))
                    .fill(fill)
                    .stroke(if p == edit {
                        Stroke::new(1.5_f32, Color32::WHITE)
                    } else {
                        Stroke::NONE
                    })
                    .min_size(Vec2::new(28.0, 22.0))
                    .sense(Sense::click_and_drag());
                let uses = self.project.uses(p);
                let hint = match uses {
                    0 => "Not on the timeline yet".to_string(),
                    1 => "Used by 1 clip".to_string(),
                    n => format!("Used by {n} clips"),
                };
                let resp = ui.add(button).on_hover_text(hint);
                if resp.clicked() {
                    self.shared.edit_pattern.store(p as u8, Relaxed);
                }
                if resp.drag_started() {
                    egui::DragAndDrop::set_payload(&ctx, p);
                }
                if resp.dragged() {
                    // Show what's being dragged next to the pointer.
                    if let Some(pos) = ctx.pointer_interact_pos() {
                        egui::Area::new(egui::Id::new("dragged pattern"))
                            .fixed_pos(pos + Vec2::new(10.0, 10.0))
                            .interactable(false)
                            .show(&ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.label(self.project.patterns[p].name.clone());
                                });
                            });
                    }
                }
                resp.context_menu(|ui| self.pattern_menu(ui, p));
            }
            let full = self.project.patterns.len() >= crate::project::MAX_PATTERNS;
            let new = ui
                .add_enabled(!full, egui::Button::new("+"))
                .on_hover_text("New empty pattern");
            if new.clicked() {
                if let Some(p) = self.project.new_pattern() {
                    self.shared.edit_pattern.store(p as u8, Relaxed);
                    self.dirty = true;
                }
            }
            ui.separator();
            if ui.button("Copy").clicked() {
                self.copy_pattern();
            }
            let paste = ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"));
            if paste.clicked() {
                self.paste_pattern();
            }
            if ui.button("Clear").clicked() {
                self.clear_pattern();
            }
        });
    }

    fn pattern_menu(&mut self, ui: &mut egui::Ui, p: usize) {
        ui.label("Name");
        let rename = ui.text_edit_singleline(&mut self.project.patterns[p].name);
        if rename.changed() {
            self.dirty = true;
        }
        ui.separator();
        if ui.button("Duplicate").clicked() {
            if let Some(copy) = self.project.duplicate_pattern(p) {
                self.shared.edit_pattern.store(copy as u8, Relaxed);
                self.dirty = true;
            }
            ui.close_menu();
        }
        let uses = self.project.uses(p);
        let label = match uses {
            0 => "Delete".to_string(),
            n => format!("Delete (and its {n} clip{})", if n == 1 { "" } else { "s" }),
        };
        let can_delete = self.project.patterns.len() > 1;
        if ui
            .add_enabled(can_delete, egui::Button::new(label))
            .clicked()
        {
            self.project.delete_pattern(p);
            self.timeline.selected = None;
            self.dirty = true;
            ui.close_menu();
        }
    }

    /// Delete, C (split, like Premiere's razor) and Ctrl+D (duplicate) for
    /// the selected clip.
    pub fn timeline_keys(&mut self, ctx: &egui::Context) {
        let Some(i) = self
            .timeline
            .selected
            .filter(|&i| i < self.project.clips.len())
        else {
            return;
        };
        let (delete, split, duplicate) = ctx.input(|inp| {
            (
                inp.key_pressed(egui::Key::Delete) || inp.key_pressed(egui::Key::Backspace),
                inp.key_pressed(egui::Key::C) && !inp.modifiers.command,
                inp.key_pressed(egui::Key::D) && inp.modifiers.command,
            )
        });
        if delete {
            self.project.clips.remove(i);
            self.timeline.selected = None;
            self.dirty = true;
        } else if split {
            let at = self.split_point();
            if let Some(right) = self.project.split_clip(i, at) {
                self.timeline.selected = Some(right);
                self.dirty = true;
            }
        } else if duplicate {
            self.duplicate_clip(i);
        }
    }

    /// Where C splits: the playhead while playing, else the cursor.
    fn split_point(&self) -> usize {
        let song_mode = self.shared.song_mode.load(Relaxed);
        if self.shared.playing.load(Relaxed) && song_mode {
            self.shared.song_step.load(Relaxed)
        } else {
            self.timeline.cursor
        }
    }

    /// Puts a copy of a clip right after it.
    fn duplicate_clip(&mut self, i: usize) {
        let c = self.project.clips[i];
        if c.end() + c.len <= MAX_SONG_STEPS {
            self.project.clips.push(Clip {
                start: c.end(),
                ..c
            });
            self.timeline.selected = Some(self.project.clips.len() - 1);
            self.dirty = true;
        }
    }

    /// Starts or stops playback. In Song mode it starts from the cursor.
    pub fn set_playing(&mut self, on: bool) {
        if on && self.shared.song_mode.load(Relaxed) {
            self.shared.seek.store(self.timeline.cursor, Relaxed);
        }
        self.shared.playing.store(on, Relaxed);
    }

    pub fn timeline(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = self.shared.clone();
        let song_mode = s.song_mode.load(Relaxed);
        ui.horizontal(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Timeline").strong()),
            )
            .on_hover_text(
                "Drag patterns here, or double-click to place the selected one.\n\
                 Drag clips to move them, their edges to trim.\n\
                 Alt-drag copies. Right-click for more. C splits at the cursor.\n\
                 Click the ruler to set where playback starts; drag it to loop.",
            );
            let mut looping = s.loop_on.load(Relaxed);
            if ui
                .toggle_value(&mut looping, "Loop")
                .on_hover_text("Repeat the marked part (drag on the ruler to mark it)")
                .changed()
            {
                s.loop_on.store(looping, Relaxed);
            }
            egui::ComboBox::from_id_salt("snap")
                .selected_text(format!("Snap: {}", self.timeline.snap.label()))
                .width(90.0)
                .show_ui(ui, |ui| {
                    for snap in [Snap::Bar, Snap::Beat, Snap::Step] {
                        ui.selectable_value(&mut self.timeline.snap, snap, snap.label());
                    }
                })
                .response
                .on_hover_text("Hold Shift while dragging to move by single steps");
            if ui.button("-").on_hover_text("Zoom out").clicked() {
                self.timeline.zoom = (self.timeline.zoom / 1.5).max(MIN_ZOOM);
            }
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                self.timeline.zoom = (self.timeline.zoom * 1.5).min(MAX_ZOOM);
            }
            if !song_mode {
                ui.weak("Pattern mode: switch to Song to play the timeline");
            }
        });

        let rows = (self.project.clips.iter().map(|c| c.row + 2).max())
            .unwrap_or(0)
            .clamp(MIN_ROWS, MAX_ROWS);
        let height = RULER + rows as f32 * ROW;
        let zoom = self.timeline.zoom;
        // Room to grow: the song plus 16 bars, and at least the view.
        let steps = (self.project.song_len() + 16 * STEPS).min(MAX_SONG_STEPS);
        let width = (steps as f32 * zoom).max(ui.available_width());
        let playhead = if playing && song_mode {
            s.song_step.load(Relaxed)
        } else {
            self.timeline.cursor
        };

        egui::ScrollArea::horizontal()
            .id_salt("timeline")
            .drag_to_scroll(false)
            .show(ui, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
                // Ctrl/Cmd + scroll zooms around the pointer.
                if resp.hovered() {
                    let factor = ui.input(|i| i.zoom_delta());
                    if factor != 1.0 {
                        self.timeline.zoom = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                    }
                }
                self.timeline_input(ui, rect, &resp, rows);
                self.paint_timeline(ui, rect, rows, playhead, playing && song_mode);
                if playing && song_mode {
                    // Keep the playhead in view.
                    let x = rect.left() + playhead as f32 * zoom;
                    let line = Rect::from_x_y_ranges(x..=x + 1.0, rect.y_range());
                    if !ui.clip_rect().contains_rect(line) {
                        ui.scroll_to_rect(line, Some(egui::Align::Min));
                    }
                }
            });
    }

    /// Which clip is under a point, topmost first, and whether the point
    /// is on its start or end edge.
    fn clip_at(&self, rect: Rect, pos: Pos2) -> Option<(usize, Option<bool>)> {
        let zoom = self.timeline.zoom;
        (0..self.project.clips.len()).rev().find_map(|i| {
            let r = clip_rect(rect, zoom, &self.project.clips[i]);
            if !r.contains(pos) {
                return None;
            }
            let edge = if pos.x < r.left() + EDGE.min(r.width() / 3.0) {
                Some(true)
            } else if pos.x > r.right() - EDGE.min(r.width() / 3.0) {
                Some(false)
            } else {
                None
            };
            Some((i, edge))
        })
    }

    fn timeline_input(&mut self, ui: &egui::Ui, rect: Rect, resp: &egui::Response, rows: usize) {
        let zoom = self.timeline.zoom;
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let snap = if shift { 1 } else { self.timeline.snap.steps() };
        let step_at = |pos: Pos2| (pos.x - rect.left()) / zoom;
        let row_at = |pos: Pos2| (((pos.y - rect.top() - RULER) / ROW).max(0.0) as usize).min(rows);
        let on_ruler = |pos: Pos2| pos.y < rect.top() + RULER;
        let max_start = MAX_SONG_STEPS;

        // A pattern dropped from the library.
        if let Some(p) = resp.dnd_release_payload::<usize>() {
            if let Some(pos) = resp.hover_pos() {
                self.place_clip(*p, snap_down(step_at(pos), snap), row_at(pos));
            }
        }

        if let Some(pos) = resp.hover_pos() {
            if !on_ruler(pos) && self.timeline.drag.is_none() {
                match self.clip_at(rect, pos) {
                    Some((_, Some(_))) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                    Some((_, None)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
                    None => {}
                }
            }
        }

        if resp.drag_started() {
            // Where the button went down, not where the drag was noticed
            // a few points later (which can be past a clip's edge).
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(pos) = origin.or(resp.interact_pointer_pos()) {
                self.timeline.drag = if on_ruler(pos) {
                    Some(Drag::Loop {
                        from: snap_near(step_at(pos), snap),
                    })
                } else {
                    match self.clip_at(rect, pos) {
                        Some((i, edge)) => {
                            let mut i = i;
                            if alt && edge.is_none() {
                                // Alt-drag: leave the original, move a copy.
                                self.project.clips.push(self.project.clips[i]);
                                i = self.project.clips.len() - 1;
                            }
                            self.timeline.selected = Some(i);
                            let before = self.project.clips[i];
                            Some(match edge {
                                Some(true) => Drag::TrimStart { clip: i, before },
                                Some(false) => Drag::TrimEnd { clip: i },
                                None => Drag::Move {
                                    clip: i,
                                    grab: step_at(pos) - before.start as f32,
                                },
                            })
                        }
                        None => None,
                    }
                };
            }
        }

        if let (Some(drag), Some(pos)) = (self.timeline.drag, resp.interact_pointer_pos()) {
            let step = step_at(pos);
            match drag {
                Drag::Move { clip, grab } => {
                    if let Some(c) = self.project.clips.get(clip).copied() {
                        let start = snap_near(step - grab, snap).min(max_start - c.len);
                        let row = row_at(pos).min(MAX_ROWS - 1);
                        if (start, row) != (c.start, c.row) {
                            self.project.clips[clip].start = start;
                            self.project.clips[clip].row = row;
                            self.dirty = true;
                        }
                    }
                }
                Drag::TrimStart { clip, before } => {
                    let start = snap_near(step, snap).min(before.end() - 1);
                    if let Some(c) = self.project.clips.get_mut(clip) {
                        if start != c.start {
                            // Trimming (or extending) the start moves where
                            // the pattern begins, so the notes stay put.
                            let moved = start as isize - before.start as isize;
                            c.start = start;
                            c.len = before.end() - start;
                            c.offset = (before.offset as isize + moved).rem_euclid(STEPS as isize)
                                as usize;
                            self.dirty = true;
                        }
                    }
                }
                Drag::TrimEnd { clip } => {
                    if let Some(c) = self.project.clips.get_mut(clip) {
                        let end = snap_near(step, snap).clamp(c.start + 1, max_start);
                        if end != c.end() {
                            c.len = end - c.start;
                            self.dirty = true;
                        }
                    }
                }
                Drag::Loop { from } => {
                    let to = snap_near(step, snap);
                    let (a, b) = (from.min(to), from.max(to));
                    if b > a {
                        self.shared.loop_start.store(a, Relaxed);
                        self.shared.loop_end.store(b, Relaxed);
                        self.shared.loop_on.store(true, Relaxed);
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.timeline.drag = None;
        }

        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if on_ruler(pos) {
                    self.timeline.cursor = snap_down(step_at(pos), snap).min(max_start - 1);
                    if self.shared.playing.load(Relaxed) && self.shared.song_mode.load(Relaxed) {
                        self.shared.seek.store(self.timeline.cursor, Relaxed);
                    }
                } else {
                    self.timeline.selected = self.clip_at(rect, pos).map(|(i, _)| i);
                }
            }
        }
        if resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if !on_ruler(pos) {
                    match self.clip_at(rect, pos) {
                        // Open the clip's pattern in the editor.
                        Some((i, _)) => {
                            let p = self.project.clips[i].pattern;
                            self.shared.edit_pattern.store(p as u8, Relaxed);
                        }
                        None => {
                            let p = self.edit_pattern();
                            self.place_clip(p, snap_down(step_at(pos), snap), row_at(pos));
                        }
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            self.timeline.menu_clip = resp
                .interact_pointer_pos()
                .and_then(|pos| self.clip_at(rect, pos))
                .map(|(i, _)| i);
            self.timeline.selected = self.timeline.menu_clip;
        }
        if self.timeline.menu_clip.is_some() {
            resp.context_menu(|ui| self.clip_menu(ui));
        }
    }

    /// Adds a one-bar clip of pattern `p`.
    fn place_clip(&mut self, p: usize, start: usize, row: usize) {
        if p < self.project.patterns.len() && start + STEPS <= MAX_SONG_STEPS {
            self.project.clips.push(Clip {
                pattern: p,
                row: row.min(MAX_ROWS - 1),
                start,
                len: STEPS,
                offset: 0,
            });
            self.timeline.selected = Some(self.project.clips.len() - 1);
            self.dirty = true;
        }
    }

    fn clip_menu(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self
            .timeline
            .menu_clip
            .filter(|&i| i < self.project.clips.len())
        else {
            ui.close_menu();
            return;
        };
        let c = self.project.clips[i];
        let name = self.project.patterns[c.pattern].name.clone();
        ui.label(egui::RichText::new(&name).strong());
        if ui.button("Edit pattern").clicked() {
            self.shared.edit_pattern.store(c.pattern as u8, Relaxed);
            ui.close_menu();
        }
        let others = self.project.uses(c.pattern) - 1;
        let unique = egui::Button::new("Make unique");
        let hint = if others == 0 {
            "This is the only clip of the pattern".to_string()
        } else {
            format!(
                "Give this clip its own copy of {name}, so editing it\n\
                 doesn't change the other {others} clip{}",
                if others == 1 { "" } else { "s" }
            )
        };
        if ui
            .add_enabled(others > 0, unique)
            .on_hover_text(hint)
            .on_disabled_hover_text("This is the only clip of the pattern")
            .clicked()
        {
            if let Some(copy) = self.project.make_unique(i) {
                self.shared.edit_pattern.store(copy as u8, Relaxed);
                self.dirty = true;
            }
            ui.close_menu();
        }
        let at = self.split_point();
        let can_split = at > c.start && at < c.end();
        if ui
            .add_enabled(can_split, egui::Button::new("Split at cursor  (C)"))
            .clicked()
        {
            if let Some(right) = self.project.split_clip(i, at) {
                self.timeline.selected = Some(right);
                self.dirty = true;
            }
            ui.close_menu();
        }
        if ui.button("Duplicate  (Ctrl+D)").clicked() {
            self.duplicate_clip(i);
            ui.close_menu();
        }
        if ui.button("Delete  (Del)").clicked() {
            self.project.clips.remove(i);
            self.timeline.selected = None;
            self.timeline.menu_clip = None;
            self.dirty = true;
            ui.close_menu();
        }
    }

    fn paint_timeline(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        rows: usize,
        playhead: usize,
        moving: bool,
    ) {
        let painter = ui.painter_at(rect);
        let zoom = self.timeline.zoom;
        painter.rect_filled(rect, 0.0, Color32::from_gray(30));
        let ruler = Rect::from_min_size(rect.min, Vec2::new(rect.width(), RULER));
        painter.rect_filled(ruler, 0.0, Color32::from_gray(45));
        for r in 0..rows {
            if r % 2 == 1 {
                let y = rect.top() + RULER + r as f32 * ROW;
                let row =
                    Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), ROW));
                painter.rect_filled(row, 0.0, Color32::from_gray(35));
            }
        }

        // Only draw what's on screen.
        let visible = ui.clip_rect();
        let first = ((visible.left() - rect.left()) / zoom).max(0.0) as usize;
        let last = ((visible.right() - rect.left()) / zoom) as usize + 1;

        // Loop region.
        let s = &self.shared;
        let (ls, le) = (s.loop_start.load(Relaxed), s.loop_end.load(Relaxed));
        if le > ls {
            let on = s.loop_on.load(Relaxed);
            let x0 = rect.left() + ls as f32 * zoom;
            let x1 = rect.left() + le as f32 * zoom;
            let band = Rect::from_x_y_ranges(x0..=x1, ruler.y_range());
            let alpha = if on { 120 } else { 40 };
            painter.rect_filled(
                band,
                0.0,
                Color32::from_rgba_unmultiplied(230, 200, 60, alpha),
            );
            if on {
                let tint = Color32::from_rgba_unmultiplied(230, 200, 60, 18);
                painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, rect.y_range()), 0.0, tint);
            }
        }

        // Bar and beat lines, with bar numbers on the ruler.
        let beat_lines = zoom >= 3.0;
        let label_every = if zoom >= 4.0 {
            1
        } else if zoom >= 2.0 {
            2
        } else {
            4
        };
        for step in (first / 4 * 4..=last).step_by(4) {
            let x = rect.left() + step as f32 * zoom;
            let bar = step % STEPS == 0;
            if bar {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.0_f32, Color32::from_gray(70)),
                );
                let n = step / STEPS;
                if n.is_multiple_of(label_every) {
                    painter.text(
                        Pos2::new(x + 3.0, rect.top() + 2.0),
                        Align2::LEFT_TOP,
                        (n + 1).to_string(),
                        FontId::proportional(11.0),
                        Color32::from_gray(190),
                    );
                }
            } else if beat_lines {
                painter.line_segment(
                    [
                        Pos2::new(x, rect.top() + RULER),
                        Pos2::new(x, rect.bottom()),
                    ],
                    Stroke::new(1.0_f32, Color32::from_gray(42)),
                );
            }
        }

        // Clips. Ones of the pattern being edited are brighter, so you can
        // see everything an edit will change.
        let edit = self.edit_pattern();
        for (i, c) in self.project.clips.iter().enumerate() {
            let r = clip_rect(rect, zoom, c);
            if r.right() < visible.left() || r.left() > visible.right() {
                continue;
            }
            let base = pattern_color(c.pattern);
            let fill = if c.pattern == edit {
                base
            } else {
                base.gamma_multiply(0.6)
            };
            painter.rect_filled(r, 3.0, fill);
            // Marks where the pattern repeats inside a long clip.
            let mut repeat = c.start + (STEPS - c.offset % STEPS) % STEPS;
            if repeat == c.start {
                repeat += STEPS;
            }
            while repeat < c.end() {
                let x = rect.left() + repeat as f32 * zoom;
                painter.line_segment(
                    [Pos2::new(x, r.top() + 4.0), Pos2::new(x, r.bottom() - 4.0)],
                    Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
                );
                repeat += STEPS;
            }
            let selected = self.timeline.selected == Some(i);
            let stroke = if selected {
                Stroke::new(2.0_f32, Color32::WHITE)
            } else {
                Stroke::new(1.0_f32, Color32::from_black_alpha(120))
            };
            painter.rect_stroke(r, 3.0, stroke);
            let text_clip = r.shrink2(Vec2::new(3.0, 0.0));
            ui.painter()
                .with_clip_rect(text_clip.intersect(visible))
                .text(
                    Pos2::new(r.left() + 4.0, r.center().y),
                    Align2::LEFT_CENTER,
                    &self.project.patterns[c.pattern].name,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
        }

        // The playhead (or, when stopped, where playback will start).
        let x = rect.left() + playhead as f32 * zoom;
        let color = if moving {
            Color32::WHITE
        } else {
            Color32::from_rgb(255, 140, 40)
        };
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.5_f32, color),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x - 5.0, rect.top()),
                Pos2::new(x + 5.0, rect.top()),
                Pos2::new(x, rect.top() + 7.0),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

fn clip_rect(rect: Rect, zoom: f32, c: &Clip) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            rect.left() + c.start as f32 * zoom,
            rect.top() + RULER + c.row as f32 * ROW + 2.0,
        ),
        Vec2::new((c.len as f32 * zoom).max(2.0), ROW - 4.0),
    )
}
