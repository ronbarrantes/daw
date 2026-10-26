//! The Arrange view's note editor for MIDI blocks: a full piano roll, all
//! 128 notes, where notes can be any length and start anywhere.
//!
//! It draws itself rather than using a scroll area, so the keys stay on the
//! left and the ruler on top while the notes scroll. Note times are in
//! `PPS`ths of a step; the grid only decides where clicks and drags land.

use std::sync::atomic::Ordering::Relaxed;

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::audio::Event;
use crate::project::{Note, MAX_NOTES, MAX_SONG_STEPS, NORMAL, PPS, STEPS};
use crate::widgets::{lit_fill, NOTE_NAMES, NOTE_ON};
use crate::App;

/// Width of the keyboard on the left, height of the ruler on top.
const KEYS: f32 = 46.0;
const RULER: f32 = 16.0;
/// Height of one note's row.
const ROW: f32 = 10.0;
const PITCHES: usize = 128;
const MIN_ZOOM: f32 = 6.0;
const MAX_ZOOM: f32 = 120.0;
/// How close to a note's end (in points) a drag resizes instead of moving.
const EDGE: f32 = 6.0;
/// Empty bars shown past the block's end, to draw notes into.
const SPARE_BARS: usize = 4;
const SELECTED: Color32 = Color32::from_rgb(255, 170, 60);

/// What clicks and drags line up to. The triplet grids fit three notes
/// where two would go.
#[derive(Clone, Copy, PartialEq)]
pub enum Grid {
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    EighthTriplet,
    SixteenthTriplet,
    Off,
}

impl Grid {
    const ALL: [Grid; 7] = [
        Grid::Quarter,
        Grid::Eighth,
        Grid::Sixteenth,
        Grid::ThirtySecond,
        Grid::EighthTriplet,
        Grid::SixteenthTriplet,
        Grid::Off,
    ];

    /// Its size in `PPS`ths of a step. Off still keeps notes from being
    /// vanishingly short.
    fn parts(self) -> u32 {
        match self {
            Grid::Quarter => 4 * PPS,
            Grid::Eighth => 2 * PPS,
            Grid::Sixteenth => PPS,
            Grid::ThirtySecond => PPS / 2,
            Grid::EighthTriplet => 4 * PPS / 3,
            Grid::SixteenthTriplet => 2 * PPS / 3,
            Grid::Off => PPS / 16,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Grid::Quarter => "1/4",
            Grid::Eighth => "1/8",
            Grid::Sixteenth => "1/16",
            Grid::ThirtySecond => "1/32",
            Grid::EighthTriplet => "1/8 triplet",
            Grid::SixteenthTriplet => "1/16 triplet",
            Grid::Off => "Off",
        }
    }

    /// The nearest grid line to `parts`, or `parts` itself when off.
    fn near(self, parts: f32) -> u32 {
        let g = self.parts() as f32;
        match self {
            Grid::Off => parts.max(0.0) as u32,
            _ => ((parts.max(0.0) / g).round() * g) as u32,
        }
    }

    /// The grid line at or before `parts`.
    fn down(self, parts: f32) -> u32 {
        let g = self.parts() as f32;
        match self {
            Grid::Off => parts.max(0.0) as u32,
            _ => ((parts.max(0.0) / g).floor() * g) as u32,
        }
    }
}

/// A drag in progress. `before` is every note as it was when it started.
enum Drag {
    /// Moving the selected notes; `grab` is the note held.
    Move {
        grab: usize,
        origin: Pos2,
        before: Vec<Note>,
        heard: u8,
    },
    /// Making the selected notes longer or shorter by `grab`'s end.
    Resize {
        grab: usize,
        origin: Pos2,
        before: Vec<Note>,
    },
    /// A box to select notes, from this point.
    Select { from: Pos2 },
    /// Ctrl-drag: drawing notes, one per grid cell the pointer passes.
    /// `last` is where the pointer was last frame, so fast drags don't
    /// skip any.
    Paint { last: Pos2 },
    /// Right-drag: deleting every note the pointer passes.
    Erase { last: Pos2 },
}

/// The note editor's view settings and what's selected.
pub struct State {
    /// The MIDI block being edited.
    pub open: Option<usize>,
    /// Points per step.
    zoom: f32,
    /// What's scrolled into view: steps from the left, rows from the top.
    scroll_x: f32,
    scroll_y: f32,
    grid: Grid,
    /// Selected notes, by index into the block's notes.
    pub selected: Vec<usize>,
    drag: Option<Drag>,
    /// Velocity for new notes.
    velocity: u8,
    /// Length of the last note drawn or resized, used for the next one.
    last_len: Option<u32>,
    /// The block last scrolled to its notes, so that happens once.
    framed: Option<usize>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: None,
            zoom: 24.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            grid: Grid::Sixteenth,
            selected: Vec::new(),
            drag: None,
            velocity: NORMAL,
            last_len: None,
            framed: None,
        }
    }
}

impl State {
    /// Forgets the selection and any drag, e.g. after an undo, when note
    /// numbers may have changed.
    pub fn deselect(&mut self) {
        self.selected.clear();
        self.drag = None;
    }
}

/// A note's name with its octave, e.g. C4 for 60.
fn note_name(pitch: u8) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[pitch as usize % 12],
        pitch as i32 / 12 - 1
    )
}

fn is_black(pitch: u8) -> bool {
    NOTE_NAMES[pitch as usize % 12].ends_with('#')
}

/// Where things are on screen, for one frame.
struct View {
    /// The notes area (right of the keys, below the ruler).
    area: Rect,
    zoom: f32,
    scroll_x: f32,
    scroll_y: f32,
}

impl View {
    fn x(&self, parts: f32) -> f32 {
        self.area.left() + (parts / PPS as f32 - self.scroll_x) * self.zoom
    }

    fn parts_at(&self, x: f32) -> f32 {
        ((x - self.area.left()) / self.zoom + self.scroll_x) * PPS as f32
    }

    fn y(&self, pitch: u8) -> f32 {
        self.area.top() + ((PITCHES - 1 - pitch as usize) as f32 - self.scroll_y) * ROW
    }

    fn pitch_at(&self, y: f32) -> u8 {
        let row = ((y - self.area.top()) / ROW + self.scroll_y).floor();
        (PITCHES as f32 - 1.0 - row).clamp(0.0, 127.0) as u8
    }

    fn note_rect(&self, n: &Note) -> Rect {
        let top = self.y(n.pitch);
        let left = self.x(n.start as f32);
        let right = self.x(n.end() as f32).max(left + 3.0);
        Rect::from_min_max(
            Pos2::new(left, top + 0.5),
            Pos2::new(right, top + ROW - 0.5),
        )
    }
}

impl App {
    /// The MIDI block open in the note editor, if it's still there.
    pub fn open_notes(&self) -> Option<usize> {
        self.notes
            .open
            .filter(|&b| self.project.blocks.get(b).is_some_and(|b| b.is_midi()))
    }

    /// Opens a MIDI block in the note editor and selects its track, so the
    /// panel and computer keyboard go with it.
    pub fn edit_notes(&mut self, b: usize) {
        if self.notes.open != Some(b) {
            self.notes.deselect();
            self.notes.framed = None;
        }
        self.notes.open = Some(b);
        let track = self.project.blocks[b].midi.as_ref().and_then(|m| m.track);
        if let Some(t) = track.filter(|&t| t < self.project.tracks.len()) {
            self.selected = t;
            self.selected_group = None;
        }
    }

    fn preview_note(&self, b: usize, pitch: u8) {
        let track = self.project.blocks[b].midi.as_ref().and_then(|m| m.track);
        if let Some(track) = track.filter(|&t| t < self.project.tracks.len()) {
            let _ = self.events.try_send(Event::Preview { track, note: pitch });
        }
    }

    /// After notes moved or grew, makes block `b` long enough for them,
    /// and its whole copies on the timeline with it.
    fn notes_changed(&mut self, b: usize) {
        let old_len = self.project.block_len(b);
        if let Some(m) = self.project.blocks[b].midi.as_mut() {
            if m.fit_notes() {
                self.project.fit_clips(b, old_len);
            }
        }
        self.dirty = true;
    }

    /// Delete, Ctrl+A (select all), Ctrl+D (duplicate), arrows (move; Shift
    /// for octaves or bars) and Esc for the note editor. Returns whether
    /// notes are selected, in which case these keys are the editor's.
    pub fn note_keys(&mut self, ctx: &egui::Context) -> bool {
        use egui::Key;
        let Some(b) = self.open_notes() else {
            return false;
        };
        let count = self.project.blocks[b]
            .midi
            .as_ref()
            .map_or(0, |m| m.notes.len());
        self.notes.selected.retain(|&i| i < count);
        let (all, escape) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(Key::A),
                i.key_pressed(Key::Escape),
            )
        });
        if all {
            self.notes.selected = (0..count).collect();
        }
        if escape {
            self.notes.deselect();
        }
        if self.notes.selected.is_empty() {
            return false;
        }
        let (delete, duplicate, up, down, left, right, shift) = ctx.input(|i| {
            (
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.modifiers.command && i.key_pressed(Key::D),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.modifiers.shift,
            )
        });
        let grid = self.notes.grid.parts();
        let Some(m) = self.project.blocks[b].midi.as_mut() else {
            return false;
        };
        let selected = &mut self.notes.selected;
        if delete {
            let mut gone = selected.clone();
            gone.sort_unstable();
            for &i in gone.iter().rev() {
                m.notes.remove(i);
            }
            selected.clear();
            self.dirty = true;
            return true;
        }
        if duplicate {
            if m.notes.len() + selected.len() <= MAX_NOTES {
                // The copies go right after the selection, and are selected.
                let first = selected.iter().map(|&i| m.notes[i].start).min();
                let last = selected.iter().map(|&i| m.notes[i].end()).max();
                let span = last.unwrap_or(0) - first.unwrap_or(0);
                let shift_by = span.div_ceil(grid) * grid;
                let copies: Vec<Note> = selected
                    .iter()
                    .map(|&i| Note {
                        start: m.notes[i].start + shift_by,
                        ..m.notes[i]
                    })
                    .collect();
                *selected = (m.notes.len()..m.notes.len() + copies.len()).collect();
                m.notes.extend(copies);
                self.notes_changed(b);
            }
            return true;
        }
        let semis: i32 = match (up, down) {
            (true, false) => 1,
            (false, true) => -1,
            _ => 0,
        } * if shift { 12 } else { 1 };
        let step = if shift { PPS * STEPS as u32 } else { grid } as i64;
        let parts: i64 = match (left, right) {
            (true, false) => -step,
            (false, true) => step,
            _ => 0,
        };
        let fits = selected.iter().all(|&i| {
            let n = m.notes[i];
            (0..=127).contains(&(n.pitch as i32 + semis)) && n.start as i64 + parts >= 0
        });
        if (semis != 0 || parts != 0) && fits {
            for &i in selected.iter() {
                let n = &mut m.notes[i];
                n.pitch = (n.pitch as i32 + semis) as u8;
                n.start = (n.start as i64 + parts) as u32;
            }
            let heard = (semis != 0).then(|| m.notes[selected[0]].pitch);
            self.notes_changed(b);
            if let Some(pitch) = heard {
                self.preview_note(b, pitch);
            }
        }
        true
    }

    /// The note editor for MIDI block `b`.
    pub fn note_editor(&mut self, ui: &mut egui::Ui, b: usize, playing: bool) {
        self.note_toolbar(ui, b);
        let height = (ui.ctx().screen_rect().bottom() - ui.cursor().top() - 12.0)
            .clamp(160.0, RULER + PITCHES as f32 * ROW);
        let size = Vec2::new(ui.available_width(), height);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let area = Rect::from_min_max(Pos2::new(rect.left() + KEYS, rect.top() + RULER), rect.max);
        let len = self.project.block_len(b);
        let shown = len + SPARE_BARS * STEPS;
        let st = &mut self.notes;

        // Scroll with the wheel or trackpad; Ctrl+scroll zooms.
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if zoom != 1.0 {
                st.zoom = (st.zoom * zoom).clamp(MIN_ZOOM, MAX_ZOOM);
            } else {
                st.scroll_x -= scroll.x / st.zoom;
                st.scroll_y -= scroll.y / ROW;
            }
            // Don't also scroll the page.
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        let rows = area.height() / ROW;
        if st.framed != Some(b) {
            // Start with the notes (or middle C) in the middle.
            let notes: &[Note] = self.project.blocks[b]
                .midi
                .as_ref()
                .map_or(&[], |m| &m.notes);
            let pitch = match notes.len() {
                0 => 60.0,
                n => notes.iter().map(|n| n.pitch as f32).sum::<f32>() / n as f32,
            };
            st.scroll_y = 127.0 - pitch - rows / 2.0;
            st.scroll_x = 0.0;
            st.framed = Some(b);
        }
        st.scroll_y = st.scroll_y.clamp(0.0, (PITCHES as f32 - rows).max(0.0));
        let steps_in_view = area.width() / st.zoom;
        st.scroll_x = st
            .scroll_x
            .clamp(0.0, (shown as f32 - steps_in_view).max(0.0));
        let view = View {
            area,
            zoom: st.zoom,
            scroll_x: st.scroll_x,
            scroll_y: st.scroll_y,
        };

        self.note_input(ui, b, &view, &resp);
        let playhead = (playing && self.shared.song_mode.load(Relaxed))
            .then(|| self.notes_playhead(b))
            .flatten();
        self.paint_notes(ui, b, &view, rect, playhead);
    }

    fn note_toolbar(&mut self, ui: &mut egui::Ui, b: usize) {
        ui.horizontal_wrapped(|ui| {
            let name = self.project.blocks[b].name.clone();
            ui.strong(format!("Notes · {name}"))
                .on_hover_text("A MIDI block: free notes, played by the track you pick");
            ui.label("Plays");
            let track = self.project.blocks[b].midi.as_ref().and_then(|m| m.track);
            let current = track
                .and_then(|t| self.project.tracks.get(t))
                .map_or("(no track)".to_string(), |t| t.name.clone());
            let mut pick = None;
            egui::ComboBox::from_id_salt("notes track")
                .selected_text(current)
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (i, t) in self.project.tracks.iter().enumerate() {
                        if ui.selectable_label(track == Some(i), &t.name).clicked() {
                            pick = Some(i);
                        }
                    }
                })
                .response
                .on_hover_text("The track that plays these notes");
            if let Some(i) = pick {
                if let Some(m) = self.project.blocks[b].midi.as_mut() {
                    m.track = Some(i);
                }
                self.selected = i;
                self.selected_group = None;
                self.dirty = true;
            }

            ui.label("Bars");
            let old_len = self.project.block_len(b);
            let mut bars = old_len.div_ceil(STEPS);
            let max_bars = MAX_SONG_STEPS / STEPS;
            if ui
                .add(egui::DragValue::new(&mut bars).range(1..=max_bars))
                .on_hover_text("How long the block is; notes past the end don't play")
                .changed()
            {
                if let Some(m) = self.project.blocks[b].midi.as_mut() {
                    m.len = bars * STEPS;
                }
                self.project.fit_clips(b, old_len);
                self.dirty = true;
            }

            let st = &mut self.notes;
            egui::ComboBox::from_id_salt("notes grid")
                .selected_text(format!("Snap: {}", st.grid.label()))
                .width(120.0)
                .show_ui(ui, |ui| {
                    for grid in Grid::ALL {
                        ui.selectable_value(&mut st.grid, grid, grid.label());
                    }
                })
                .response
                .on_hover_text(
                    "What notes line up to. Hold Shift while dragging to go off the grid.",
                );

            // Velocity of the selected notes, or of new ones.
            ui.label("Velocity");
            let notes = self.project.blocks[b].midi.as_mut().map(|m| &mut m.notes);
            let first = st.selected.first().copied();
            let mut velocity = match (&notes, first) {
                (Some(n), Some(i)) if i < n.len() => n[i].velocity,
                _ => st.velocity,
            };
            let hint = if first.is_some() {
                "How loud the selected notes are"
            } else {
                "How loud new notes are"
            };
            if ui
                .add(egui::DragValue::new(&mut velocity).range(1..=127))
                .on_hover_text(hint)
                .changed()
            {
                st.velocity = velocity;
                if let Some(notes) = notes {
                    for &i in &st.selected {
                        if let Some(n) = notes.get_mut(i) {
                            n.velocity = velocity;
                        }
                    }
                    self.dirty |= first.is_some();
                }
            }
            if ui.button("-").on_hover_text("Zoom out").clicked() {
                st.zoom = (st.zoom / 1.5).max(MIN_ZOOM);
            }
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                st.zoom = (st.zoom * 1.5).min(MAX_ZOOM);
            }
        });
        ui.weak(
            "Click: add a note · drag it: move · drag its end: longer/shorter · \
             right-click: delete · drag empty space: select · Alt-drag: copy · \
             Ctrl-drag: draw · right-drag: erase",
        );
    }

    /// Which note is under a point, topmost first, and whether the point is
    /// on its end.
    fn note_at(&self, b: usize, view: &View, pos: Pos2) -> Option<(usize, bool)> {
        let notes = &self.project.blocks[b].midi.as_ref()?.notes;
        (0..notes.len()).rev().find_map(|i| {
            let r = view.note_rect(&notes[i]);
            r.contains(pos)
                .then(|| (i, pos.x > r.right() - EDGE.min(r.width() / 3.0)))
        })
    }

    fn note_input(&mut self, ui: &egui::Ui, b: usize, view: &View, resp: &egui::Response) {
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let in_area = |pos: Pos2| view.area.contains(pos);
        let on_keys = |pos: Pos2| pos.x < view.area.left() && pos.y > view.area.top();

        if let Some(pos) = resp.hover_pos() {
            if self.notes.drag.is_none() && in_area(pos) {
                match self.note_at(b, view, pos) {
                    Some((_, true)) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                    Some((_, false)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
                    None => {}
                }
            }
        }

        let (command, secondary) = ui.input(|i| (i.modifiers.command, i.pointer.secondary_down()));
        let origin = ui.input(|i| i.pointer.press_origin());
        if let (true, true, Some(last)) = (resp.drag_started(), secondary || command, origin) {
            self.notes.drag = Some(if secondary {
                Drag::Erase { last }
            } else {
                Drag::Paint { last }
            });
        } else if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(pos) = origin
                .or(resp.interact_pointer_pos())
                .filter(|&p| in_area(p))
            {
                self.notes.drag = match self.note_at(b, view, pos) {
                    Some((mut grab, edge)) => {
                        if !self.notes.selected.contains(&grab) {
                            self.notes.selected = vec![grab];
                        }
                        let m = self.project.blocks[b].midi.as_mut().expect("a MIDI block");
                        if alt && !edge && m.notes.len() + self.notes.selected.len() <= MAX_NOTES {
                            // Alt-drag: leave the notes, move copies.
                            let first = m.notes.len();
                            let at = self.notes.selected.iter().position(|&i| i == grab);
                            for &i in &self.notes.selected {
                                m.notes.push(m.notes[i]);
                            }
                            self.notes.selected = (first..m.notes.len()).collect();
                            grab = first + at.unwrap_or(0);
                            self.dirty = true;
                        }
                        let before = m.notes.clone();
                        Some(if edge {
                            Drag::Resize {
                                grab,
                                origin: pos,
                                before,
                            }
                        } else {
                            let heard = before[grab].pitch;
                            Drag::Move {
                                grab,
                                origin: pos,
                                before,
                                heard,
                            }
                        })
                    }
                    None => Some(Drag::Select { from: pos }),
                };
            }
        }

        let pointer = resp.interact_pointer_pos();
        if let (Some(mut drag), Some(pos)) = (self.notes.drag.take(), pointer) {
            let grid = if shift { Grid::Off } else { self.notes.grid };
            let dx = (pos.x - view_origin(&drag).x) / view.zoom * PPS as f32;
            let selected = self.notes.selected.clone();
            let notes = &mut self.project.blocks[b]
                .midi
                .as_mut()
                .expect("a MIDI block")
                .notes;
            let mut changed = false;
            let mut hear = None;
            match &mut drag {
                Drag::Move {
                    grab,
                    origin,
                    before,
                    heard,
                } => {
                    let g = before[*grab];
                    // The held note lands on the grid; the rest keep their
                    // places around it.
                    let start = grid.near(g.start as f32 + dx) as i64;
                    let earliest = selected.iter().map(|&i| before[i].start).min().unwrap_or(0);
                    let d = (start - g.start as i64).max(-(earliest as i64));
                    let rows = ((origin.y - pos.y) / ROW).round() as i32;
                    let low = selected.iter().map(|&i| before[i].pitch).min().unwrap_or(0) as i32;
                    let high = selected.iter().map(|&i| before[i].pitch).max().unwrap_or(0) as i32;
                    let semis = rows.clamp(-low, 127 - high);
                    for &i in &selected {
                        let moved = Note {
                            start: (before[i].start as i64 + d) as u32,
                            pitch: (before[i].pitch as i32 + semis) as u8,
                            ..before[i]
                        };
                        changed |= notes[i] != moved;
                        notes[i] = moved;
                    }
                    let pitch = notes[*grab].pitch;
                    if pitch != *heard {
                        hear = Some(pitch);
                    }
                    *heard = pitch;
                }
                Drag::Resize { grab, before, .. } => {
                    let g = before[*grab];
                    let min = grid.parts().max(1);
                    let end = grid.near(g.end() as f32 + dx).max(g.start + min);
                    let d = end as i64 - g.end() as i64;
                    for &i in &selected {
                        let len = (before[i].len as i64 + d).max(min as i64) as u32;
                        changed |= notes[i].len != len;
                        notes[i].len = len;
                    }
                    self.notes.last_len = Some(notes[*grab].len);
                }
                Drag::Select { from } => {
                    let r = Rect::from_two_pos(*from, pos);
                    self.notes.selected = (0..notes.len())
                        .filter(|&i| view.note_rect(&notes[i]).intersects(r))
                        .collect();
                }
                Drag::Paint { last } => {
                    // Every grid cell between last frame's pointer and
                    // this one, on the row it's on now.
                    let pitch = view.pitch_at(pos.y);
                    let grid = self.notes.grid;
                    let len = grid.parts().max(PPS / 4);
                    let (a, b) = (view.parts_at(last.x), view.parts_at(pos.x));
                    let mut cell = grid.down(a.min(b));
                    while cell as f32 <= a.max(b) && notes.len() < MAX_NOTES {
                        let taken = notes
                            .iter()
                            .any(|n| n.pitch == pitch && n.start <= cell && n.end() > cell);
                        if !taken {
                            notes.push(Note {
                                start: cell,
                                len,
                                pitch,
                                velocity: self.notes.velocity,
                            });
                            changed = true;
                            hear = Some(pitch);
                        }
                        cell += len;
                    }
                    *last = pos;
                }
                Drag::Erase { last } => {
                    let path = Rect::from_two_pos(*last, pos).expand(1.0);
                    let before = notes.len();
                    notes.retain(|n| !view.note_rect(n).intersects(path));
                    if notes.len() != before {
                        self.notes.selected.clear();
                        changed = true;
                    }
                    *last = pos;
                }
            }
            self.notes.drag = Some(drag);
            if changed {
                self.notes_changed(b);
            }
            if let Some(pitch) = hear {
                self.preview_note(b, pitch);
            }
        }
        if resp.drag_stopped() {
            self.notes.drag = None;
        }

        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if on_keys(pos) {
                    self.preview_note(b, view.pitch_at(pos.y));
                } else if in_area(pos) {
                    match self.note_at(b, view, pos) {
                        Some((i, _)) if shift => {
                            match self.notes.selected.iter().position(|&s| s == i) {
                                Some(at) => {
                                    self.notes.selected.remove(at);
                                }
                                None => self.notes.selected.push(i),
                            }
                        }
                        Some((i, _)) => {
                            self.notes.selected = vec![i];
                            let pitch = self.project.blocks[b]
                                .midi
                                .as_ref()
                                .map(|m| m.notes[i].pitch);
                            if let Some(pitch) = pitch {
                                self.preview_note(b, pitch);
                            }
                        }
                        None => self.add_note(b, view, pos),
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            let hit = resp
                .interact_pointer_pos()
                .and_then(|pos| self.note_at(b, view, pos));
            if let (Some((i, _)), Some(m)) = (hit, self.project.blocks[b].midi.as_mut()) {
                m.notes.remove(i);
                self.notes.deselect();
                self.dirty = true;
            }
        }
    }

    /// Clicking empty space: a new note there, as long as the last one.
    fn add_note(&mut self, b: usize, view: &View, pos: Pos2) {
        let st = &mut self.notes;
        let Some(m) = self.project.blocks[b].midi.as_mut() else {
            return;
        };
        if m.notes.len() >= MAX_NOTES {
            return;
        }
        let pitch = view.pitch_at(pos.y);
        let start = st.grid.down(view.parts_at(pos.x));
        let len = st.last_len.unwrap_or(st.grid.parts().max(PPS / 4));
        m.notes.push(Note {
            start,
            len,
            pitch,
            velocity: st.velocity,
        });
        st.selected = vec![m.notes.len() - 1];
        self.notes_changed(b);
        self.preview_note(b, pitch);
    }

    /// Where the playhead is in block `b`, in steps: in the selected copy
    /// on the timeline if that's playing, else any copy that is.
    fn notes_playhead(&self, b: usize) -> Option<f32> {
        let pos = self.shared.song_step.load(Relaxed);
        let len = self.project.block_len(b);
        let selected = self
            .timeline
            .selected
            .and_then(|i| self.project.clips.get(i));
        selected
            .into_iter()
            .chain(&self.project.clips)
            .filter(|c| c.block == b)
            .find_map(|c| c.step_at(pos, len))
            .map(|s| s as f32)
    }

    fn paint_notes(&self, ui: &egui::Ui, b: usize, view: &View, rect: Rect, playhead: Option<f32>) {
        let painter = ui.painter_at(rect);
        let area = view.area;
        let notes_painter = ui.painter_at(area);
        let len = self.project.block_len(b);
        painter.rect_filled(rect, 0.0, Color32::from_gray(30));

        // Rows: black keys darker, a line under each C.
        let top = view.scroll_y.floor() as usize;
        let rows = (area.height() / ROW).ceil() as usize + 1;
        for row in top..(top + rows).min(PITCHES) {
            let pitch = (PITCHES - 1 - row) as u8;
            let y = view.y(pitch);
            let r = Rect::from_x_y_ranges(area.x_range(), y..=y + ROW);
            let fill = if is_black(pitch) {
                Color32::from_gray(34)
            } else {
                Color32::from_gray(44)
            };
            notes_painter.rect_filled(r, 0.0, fill);
            if pitch.is_multiple_of(12) {
                notes_painter.line_segment(
                    [
                        Pos2::new(area.left(), y + ROW),
                        Pos2::new(area.right(), y + ROW),
                    ],
                    Stroke::new(1.0_f32, Color32::from_gray(70)),
                );
            }
        }

        // Grid lines: the snap grid faintly, beats and bars stronger.
        let first = view.scroll_x;
        let last = first + area.width() / view.zoom;
        let grid = match self.notes.grid {
            Grid::Off => PPS,
            g => g.parts(),
        };
        let fine = grid as f32 / PPS as f32 * view.zoom >= 5.0;
        let mut line = (first * PPS as f32 / grid as f32).floor() as u32 * grid;
        while (line as f32) <= last * PPS as f32 {
            let x = view.x(line as f32);
            let (bar, beat) = (
                line.is_multiple_of(PPS * STEPS as u32),
                line.is_multiple_of(PPS * 4),
            );
            let gray = if bar {
                90
            } else if beat {
                60
            } else {
                50
            };
            if bar || beat || fine {
                notes_painter.line_segment(
                    [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
                    Stroke::new(1.0_f32, Color32::from_gray(gray)),
                );
            }
            line += grid;
        }

        // Past the block's end: notes there don't play.
        let end_x = view.x((len as u32 * PPS) as f32);
        if end_x < area.right() {
            let r = Rect::from_x_y_ranges(end_x.max(area.left())..=area.right(), area.y_range());
            notes_painter.rect_filled(r, 0.0, Color32::from_black_alpha(110));
            notes_painter.line_segment(
                [
                    Pos2::new(end_x, area.top()),
                    Pos2::new(end_x, area.bottom()),
                ],
                Stroke::new(2.0_f32, Color32::from_rgb(230, 200, 60)),
            );
        }

        // The notes.
        if let Some(m) = &self.project.blocks[b].midi {
            for (i, n) in m.notes.iter().enumerate() {
                let r = view.note_rect(n);
                if !r.intersects(area) {
                    continue;
                }
                let selected = self.notes.selected.contains(&i);
                let fill = lit_fill(if selected { SELECTED } else { NOTE_ON }, n.velocity);
                notes_painter.rect_filled(r, 2.0, fill);
                notes_painter.rect_stroke(
                    r,
                    2.0,
                    Stroke::new(1.0_f32, Color32::from_black_alpha(140)),
                );
                if selected {
                    notes_painter.rect_stroke(r, 2.0, Stroke::new(1.0_f32, Color32::WHITE));
                }
            }
        }
        if let (Some(Drag::Select { from }), Some(pos)) =
            (&self.notes.drag, ui.input(|i| i.pointer.interact_pos()))
        {
            let r = Rect::from_two_pos(*from, pos);
            notes_painter.rect_filled(r, 0.0, Color32::from_white_alpha(20));
            notes_painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, Color32::from_white_alpha(120)));
        }

        // The ruler, with bar numbers.
        let ruler = Rect::from_x_y_ranges(area.x_range(), rect.top()..=area.top());
        painter.rect_filled(ruler, 0.0, Color32::from_gray(45));
        let ruler_painter = ui.painter_at(ruler);
        let first_bar = (first / STEPS as f32).floor() as usize;
        let last_bar = (last / STEPS as f32).ceil() as usize;
        let every = if view.zoom * STEPS as f32 >= 40.0 {
            1
        } else {
            4
        };
        for bar in (first_bar..=last_bar).filter(|b| b % every == 0) {
            let x = view.x((bar as u32 * STEPS as u32 * PPS) as f32);
            ruler_painter.text(
                Pos2::new(x + 3.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                (bar + 1).to_string(),
                FontId::proportional(11.0),
                Color32::from_gray(190),
            );
        }

        // The keyboard.
        let keys = Rect::from_x_y_ranges(rect.left()..=area.left(), area.y_range());
        let keys_painter = ui.painter_at(keys);
        keys_painter.rect_filled(keys, 0.0, Color32::from_gray(200));
        for row in top..(top + rows).min(PITCHES) {
            let pitch = (PITCHES - 1 - row) as u8;
            let y = view.y(pitch);
            let r = Rect::from_x_y_ranges(keys.left()..=keys.right() - 1.0, y..=y + ROW);
            if is_black(pitch) {
                let width = keys.width() * 0.6;
                let key = Rect::from_min_size(r.min, Vec2::new(width, ROW));
                keys_painter.rect_filled(key, 1.0, Color32::from_gray(25));
            }
            // The gaps between white keys with no black key between them.
            if matches!(pitch % 12, 0 | 5) {
                keys_painter.line_segment(
                    [
                        Pos2::new(r.left(), r.bottom()),
                        Pos2::new(r.right(), r.bottom()),
                    ],
                    Stroke::new(1.0_f32, Color32::from_gray(120)),
                );
            }
            if pitch.is_multiple_of(12) {
                keys_painter.text(
                    Pos2::new(r.right() - 3.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    note_name(pitch),
                    FontId::proportional(9.0),
                    Color32::BLACK,
                );
            }
        }

        if let Some(step) = playhead {
            let x = view.x(step * PPS as f32);
            if area.x_range().contains(x) {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(1.5_f32, Color32::WHITE),
                );
            }
        }
    }
}

/// Where a drag started.
fn view_origin(drag: &Drag) -> Pos2 {
    match drag {
        Drag::Move { origin, .. } | Drag::Resize { origin, .. } => *origin,
        Drag::Select { from } => *from,
        Drag::Paint { last } | Drag::Erase { last } => *last,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_snap_to_straight_and_triplet_lines() {
        assert_eq!(Grid::Sixteenth.near(370.0), 480);
        assert_eq!(Grid::Sixteenth.down(470.0), 240);
        // Three 1/8 triplets fill a beat (four 16ths).
        assert_eq!(Grid::EighthTriplet.parts() * 3, 4 * PPS);
        assert_eq!(Grid::SixteenthTriplet.near(170.0), 160);
        assert_eq!(Grid::Off.near(123.7), 123);
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(21), "A0");
    }
}
