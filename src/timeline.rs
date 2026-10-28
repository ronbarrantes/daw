//! The Jam view's Chain row and pattern library, and the Arrange view's
//! timeline and block drawer.
//!
//! The units: patterns are chained bar by bar into blocks (the Chain row
//! in Jam edits the current block), and blocks are placed on the timeline
//! to make a song. Each placed block points to its block, like a Figma
//! instance: edit the block and every copy changes. "Make unique" gives one
//! copy its own block. Time is in steps (16ths).
//!
//! MIDI blocks are made here too (+ MIDI in the drawer) and open in the
//! note editor below (see `notes`).

use std::sync::atomic::Ordering::Relaxed;

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::project::{
    AutoLane, AutoPoint, Clip, Kind, Target, CHAIN_SLOTS, MAX_BLOCKS, MAX_PATTERNS, MAX_SONG_STEPS,
    MIX_TARGETS, PPS, STEPS,
};
use crate::widgets::playhead;
use crate::{App, Tab};

/// What's carried when a block is dragged from the drawer.
struct BlockDrag(usize);

const RULER: f32 = 18.0;
const ROW: f32 = 24.0;
/// Rows shown: the ones in use, one spare, and at least this many.
const MIN_ROWS: usize = 2;
const MAX_ROWS: usize = 8;
/// How close to a clip's edge (in points) a drag trims instead of moving.
const EDGE: f32 = 6.0;
/// Height of an automation lane.
const LANE: f32 = 40.0;
/// How close (in points) a press must be to grab an automation point.
const GRAB: f32 = 7.0;
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
    /// An automation point being dragged: (lane, point).
    auto_drag: Option<(usize, usize)>,
    /// The automation lane whose right-click menu is open.
    auto_menu: Option<usize>,
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
            auto_drag: None,
            auto_menu: None,
        }
    }
}

/// A block's color on the timeline and in the drawer, so copies match.
pub fn block_color(b: usize) -> Color32 {
    pattern_color(b + 3)
}

/// A pattern's color, in the library and the Chain row.
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
    /// The pattern library: pick a pattern to edit; right-click to rename,
    /// duplicate or delete.
    pub fn library_row(&mut self, ui: &mut egui::Ui) {
        let edit = self.edit_pattern();
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Patterns").strong()),
            )
            .on_hover_text(
                "Click a pattern to edit it below; click a Chain square to put it\n\
                 in that bar. Right-click to rename, duplicate or delete.",
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
                    .min_size(Vec2::new(28.0, 22.0));
                let hint = match self.project.pattern_uses(p) {
                    0 => "Not in a block yet".to_string(),
                    1 => "Used in 1 block".to_string(),
                    n => format!("Used in {n} blocks"),
                };
                let resp = ui.add(button).on_hover_text(hint);
                if resp.clicked() {
                    self.shared.edit_pattern.store(p as u8, Relaxed);
                }
                resp.context_menu(|ui| self.pattern_menu(ui, p));
            }
            let full = self.project.patterns.len() >= MAX_PATTERNS;
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
        let label = match self.project.pattern_uses(p) {
            0 => "Delete".to_string(),
            n => format!(
                "Delete (its bars in {n} block{} go silent)",
                if n == 1 { "" } else { "s" }
            ),
        };
        let can_delete = self.project.patterns.len() > 1;
        if ui
            .add_enabled(can_delete, egui::Button::new(label))
            .clicked()
        {
            self.project.delete_pattern(p);
            // Keep editing the same pattern, or the one that took the
            // deleted one's place, so the grid below never points past
            // the end this frame.
            let edit = self.shared.edit_pattern.load(Relaxed) as usize;
            let edit = if edit > p { edit - 1 } else { edit };
            let last = self.project.patterns.len() - 1;
            self.shared
                .edit_pattern
                .store(edit.min(last) as u8, Relaxed);
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

    /// Starts or stops playback. In Arrange it starts from the cursor.
    pub fn set_playing(&mut self, on: bool) {
        if on && self.tab == Tab::Arrange {
            self.shared.seek.store(self.timeline.cursor, Relaxed);
        }
        self.shared.playing.store(on, Relaxed);
    }

    /// The block shown in the Jam view: always a chain block.
    pub fn edit_block(&self) -> usize {
        let b = self.shared.edit_block.load(Relaxed);
        match self.project.blocks.get(b) {
            Some(block) if !block.is_midi() => b,
            _ => self.project.first_chain_block(),
        }
    }

    /// Opens a block to edit: a chain in the Jam view, or a MIDI block's
    /// notes in the note editor.
    fn open_block(&mut self, b: usize) {
        if self.project.blocks[b].is_midi() {
            self.edit_notes(b);
            return;
        }
        self.shared.edit_block.store(b, Relaxed);
        self.shared.chain_mode.store(true, Relaxed);
        if let Some(p) = self.project.blocks[b].chain.iter().flatten().next() {
            self.shared.edit_pattern.store(*p as u8, Relaxed);
        }
        self.tab = Tab::Jam;
    }

    /// The Jam view's Chain row: the current block's patterns, bar by bar.
    /// Click a square to put the selected pattern there; click it again or
    /// right-click to empty it.
    pub fn chain_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let b = self.edit_block();
        let edit = self.edit_pattern();
        let chain_mode = self.shared.chain_mode.load(Relaxed);
        let pos = self.shared.song_step.load(Relaxed) / STEPS;
        ui.horizontal(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Chain").strong()),
            )
            .on_hover_text(
                "A block: patterns played bar by bar. Click a square to put the\n\
                 selected pattern in that bar; click again or right-click to empty it.\n\
                 Blocks go on the timeline in Arrange (like clips in other DAWs).",
            );
            egui::ComboBox::from_id_salt("block")
                .selected_text(self.project.blocks[b].name.clone())
                .width(110.0)
                .show_ui(ui, |ui| {
                    let chains = self.project.blocks.iter().enumerate();
                    for (i, block) in chains.filter(|(_, b)| !b.is_midi()) {
                        if ui.selectable_label(i == b, &block.name).clicked() {
                            self.shared.edit_block.store(i, Relaxed);
                        }
                    }
                })
                .response
                .on_hover_text("The block you're editing")
                .context_menu(|ui| self.block_menu(ui, b));
            let full = self.project.blocks.len() >= MAX_BLOCKS;
            if ui
                .add_enabled(!full, egui::Button::new("Save as new block"))
                .on_hover_text("Copy this chain into a new block and keep editing that")
                .clicked()
            {
                if let Some(copy) = self.project.duplicate_block(b) {
                    self.shared.edit_block.store(copy, Relaxed);
                    self.dirty = true;
                }
            }
            if ui
                .add_enabled(!full, egui::Button::new("+"))
                .on_hover_text("New empty block")
                .clicked()
            {
                if let Some(new) = self.project.new_block() {
                    self.shared.edit_block.store(new, Relaxed);
                    self.dirty = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(66.0);
            for bar in 0..CHAIN_SLOTS {
                let (slot, run, _) = self.project.blocks[b].at(bar * STEPS);
                let fill = match slot {
                    Some(p) => pattern_color(p),
                    None if bar % 4 == 0 => Color32::from_gray(60),
                    None => Color32::from_gray(45),
                };
                // A pattern longer than a bar fills a cell per bar: its
                // name on the first, then the bar number.
                let label = match slot {
                    Some(p) => match run % self.project.pattern_bars(p) {
                        0 => self.project.patterns[p].name.clone(),
                        k => (k + 1).to_string(),
                    },
                    None => String::new(),
                };
                let cell =
                    egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).small())
                        .fill(fill)
                        .stroke(playhead(playing && chain_mode && bar == pos))
                        .min_size(Vec2::new(22.0, 22.0));
                let resp = ui.add(cell);
                let old_len = self.project.block_len(b);
                if resp.secondary_clicked() || (resp.clicked() && slot == Some(edit)) {
                    self.project.clear_pattern_at(b, bar);
                    self.dirty = true;
                } else if resp.clicked() {
                    self.project.place_pattern(b, bar, edit);
                    self.dirty = true;
                }
                if self.project.block_len(b) != old_len {
                    self.project.fit_clips(b, old_len);
                }
            }
        });
    }

    /// Rename, duplicate or delete a block.
    fn block_menu(&mut self, ui: &mut egui::Ui, b: usize) {
        ui.label("Name");
        if ui
            .text_edit_singleline(&mut self.project.blocks[b].name)
            .changed()
        {
            self.dirty = true;
        }
        ui.separator();
        let midi = self.project.blocks[b].is_midi();
        if ui
            .button(if midi { "Edit notes" } else { "Edit in Jam" })
            .clicked()
        {
            self.open_block(b);
            ui.close_menu();
        }
        if ui.button("Duplicate").clicked() {
            if let Some(copy) = self.project.duplicate_block(b) {
                if midi {
                    self.edit_notes(copy);
                } else {
                    self.shared.edit_block.store(copy, Relaxed);
                }
                self.dirty = true;
            }
            ui.close_menu();
        }
        let label = match self.project.block_uses(b) {
            0 => "Delete".to_string(),
            n => format!(
                "Delete (and its {n} cop{} on the timeline)",
                if n == 1 { "y" } else { "ies" }
            ),
        };
        if ui
            .add_enabled(self.project.can_delete_block(b), egui::Button::new(label))
            .on_disabled_hover_text("The Jam view needs at least one chain block")
            .clicked()
        {
            self.project.delete_block(b);
            self.timeline.selected = None;
            self.notes.open = match self.notes.open {
                Some(o) if o == b => None,
                Some(o) if o > b => Some(o - 1),
                other => other,
            };
            self.notes.deselect();
            self.dirty = true;
            ui.close_menu();
        }
    }

    /// The Arrange view's block drawer: drag a block onto the timeline,
    /// double-click it to edit it in Jam, right-click for more.
    pub fn block_drawer(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let edit = self.edit_block();
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Blocks").strong()),
            )
            .on_hover_text(
                "Drag a block onto the timeline. Double-click one to edit it.\n\
                 Chain blocks are made in Jam's Chain row; + MIDI makes a MIDI\n\
                 block, free notes for one track (they're like clips in other DAWs).",
            );
            let open = self.open_notes();
            for b in 0..self.project.blocks.len() {
                let block = &self.project.blocks[b];
                let current = if block.is_midi() {
                    open == Some(b)
                } else {
                    b == edit
                };
                let fill = block_color(b).gamma_multiply(if current { 1.0 } else { 0.6 });
                let text = format!(
                    "{}  ({}{} bar{})",
                    block.name,
                    if block.is_midi() { "MIDI, " } else { "" },
                    block.bars(),
                    if block.bars() == 1 { "" } else { "s" }
                );
                let button = egui::Button::new(egui::RichText::new(text).color(Color32::WHITE))
                    .fill(fill)
                    .min_size(Vec2::new(28.0, 24.0))
                    .sense(Sense::click_and_drag());
                let hint = match self.project.block_uses(b) {
                    0 => "Not on the timeline yet".to_string(),
                    1 => "On the timeline once".to_string(),
                    n => format!("On the timeline {n} times"),
                };
                let resp = ui.add(button).on_hover_text(hint);
                let midi = self.project.blocks[b].is_midi();
                if resp.double_clicked() || (resp.clicked() && midi) {
                    self.open_block(b);
                } else if resp.clicked() {
                    self.shared.edit_block.store(b, Relaxed);
                }
                if resp.drag_started() {
                    egui::DragAndDrop::set_payload(&ctx, BlockDrag(b));
                }
                if resp.dragged() {
                    // Show what's being dragged next to the pointer.
                    if let Some(pos) = ctx.pointer_interact_pos() {
                        egui::Area::new(egui::Id::new("dragged block"))
                            .fixed_pos(pos + Vec2::new(10.0, 10.0))
                            .interactable(false)
                            .show(&ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.label(self.project.blocks[b].name.clone());
                                });
                            });
                    }
                }
                resp.context_menu(|ui| self.block_menu(ui, b));
            }
            if ui
                .add_enabled(
                    self.project.blocks.len() < MAX_BLOCKS,
                    egui::Button::new("+"),
                )
                .on_hover_text("New empty block (opens in Jam)")
                .clicked()
            {
                if let Some(new) = self.project.new_block() {
                    self.dirty = true;
                    self.open_block(new);
                }
            }
            if ui
                .add_enabled(
                    self.project.blocks.len() < MAX_BLOCKS,
                    egui::Button::new("+ MIDI"),
                )
                .on_hover_text(
                    "New MIDI block for the selected track: free notes,\n\
                     any pitch and length, edited below",
                )
                .clicked()
            {
                let track = (self.selected < self.project.tracks.len()).then_some(self.selected);
                if let Some(new) = self.project.new_midi_block(track) {
                    self.dirty = true;
                    self.edit_notes(new);
                }
            }
        });
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
                "Drag blocks here from the drawer, or double-click to place the\n\
                 current one. Drag a block to move it, its edges to trim or stretch.\n\
                 Alt-drag copies. Double-click one to edit it in Jam. Right-click for\n\
                 more. C splits at the cursor. Click the ruler to set where playback\n\
                 starts; drag along it to loop.",
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
            ui.separator();
            self.add_automation_menu(ui);
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
                self.automation_lanes(ui, width, playhead, playing && song_mode);
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

    /// + Automation: pick a track, then the knob to draw changes for.
    fn add_automation_menu(&mut self, ui: &mut egui::Ui) {
        let resp = ui.menu_button("+ Automation", |ui| {
            for i in 0..self.project.tracks.len() {
                let name = self.project.tracks[i].name.clone();
                ui.menu_button(name, |ui| {
                    let mut targets: Vec<Target> = MIX_TARGETS.iter().map(|(t, _)| *t).collect();
                    if matches!(self.project.tracks[i].kind, Kind::Synth(_)) {
                        targets.extend(crate::synth::automatable().map(Target::Synth));
                    }
                    for target in targets {
                        if target == Target::Synth(crate::synth::automatable().next().unwrap_or(0))
                        {
                            ui.separator();
                        }
                        let taken = self
                            .project
                            .automation
                            .iter()
                            .any(|l| l.track == i && l.target == target);
                        let button = egui::Button::new(target.label());
                        if ui.add_enabled(!taken, button).clicked() {
                            self.add_lane(i, target);
                            ui.close_menu();
                        }
                    }
                });
            }
        });
        resp.response.on_hover_text(
            "Draw a knob's changes over the song, like a filter opening or a\n\
             fade out. Pick a track, then the knob.",
        );
    }

    /// A new automation lane, holding the knob where it's set now.
    fn add_lane(&mut self, track: usize, target: Target) {
        let Some(t) = self.project.tracks.get(track) else {
            return;
        };
        let now = match target {
            Target::Volume => t.volume,
            Target::Pan => t.pan,
            Target::Reverb => t.reverb,
            Target::Delay => t.delay,
            Target::Synth(k) => match &t.kind {
                Kind::Synth(v) => v[k],
                Kind::Sampler { .. } => return,
            },
        };
        self.project.automation.push(AutoLane {
            track,
            target,
            points: vec![AutoPoint {
                at: self.timeline.cursor as u32 * PPS,
                value: target.param().to_unit(now).clamp(0.0, 1.0),
            }],
        });
        self.dirty = true;
    }

    /// The automation lanes under the timeline: a line per knob. Click to
    /// add a point, drag one to move it, right-click a point to remove it
    /// (or empty space for the lane's menu).
    fn automation_lanes(&mut self, ui: &mut egui::Ui, width: f32, playhead: usize, moving: bool) {
        let n = self.project.automation.len();
        if n == 0 {
            return;
        }
        let size = Vec2::new(width, n as f32 * LANE);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        // Room for the scroll bar, so it doesn't cover the last lane.
        ui.add_space(12.0);
        let zoom = self.timeline.zoom;
        let lane_rect = |l: usize| {
            Rect::from_min_size(
                Pos2::new(rect.left(), rect.top() + l as f32 * LANE),
                Vec2::new(rect.width(), LANE),
            )
        };
        let x_of = |at: u32| rect.left() + at as f32 / PPS as f32 * zoom;
        let y_of = |l: usize, v: f32| {
            let r = lane_rect(l).shrink(4.0);
            r.bottom() - v.clamp(0.0, 1.0) * r.height()
        };
        let lane_at = |pos: Pos2| ((pos.y - rect.top()) / LANE).floor().max(0.0) as usize;
        let point_at = |lanes: &[AutoLane], pos: Pos2| -> Option<(usize, usize)> {
            let l = lane_at(pos).min(lanes.len().checked_sub(1)?);
            lanes[l]
                .points
                .iter()
                .position(|p| Pos2::new(x_of(p.at), y_of(l, p.value)).distance(pos) <= GRAB)
                .map(|k| (l, k))
        };
        let shift = ui.input(|i| i.modifiers.shift);
        let snap = if shift { 1 } else { self.timeline.snap.steps() };
        // A point from where the pointer is: when, snapped, and how high.
        let place = |l: usize, pos: Pos2| {
            let step = snap_near((pos.x - rect.left()) / zoom, snap).min(MAX_SONG_STEPS);
            let r = lane_rect(l).shrink(4.0);
            AutoPoint {
                at: step as u32 * PPS,
                value: ((r.bottom() - pos.y) / r.height()).clamp(0.0, 1.0),
            }
        };

        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(pos) = origin {
                self.timeline.auto_drag = point_at(&self.project.automation, pos).or_else(|| {
                    // Dragging empty space makes a point and moves it.
                    let l = lane_at(pos).min(n - 1);
                    let lane = &mut self.project.automation[l];
                    lane.points.push(place(l, pos));
                    lane.sort();
                    let p = place(l, pos);
                    lane.points.iter().position(|q| *q == p).map(|k| (l, k))
                });
            }
        }
        if let (Some((l, k)), Some(pos)) = (self.timeline.auto_drag, resp.interact_pointer_pos()) {
            if let Some(lane) = self.project.automation.get_mut(l) {
                let p = place(l, pos);
                if lane.points.get(k).is_some_and(|q| *q != p) {
                    lane.points[k] = p;
                    lane.sort();
                    // It may have moved past a neighbour.
                    let k = lane.points.iter().position(|q| *q == p).unwrap_or(k);
                    self.timeline.auto_drag = Some((l, k));
                    self.dirty = true;
                }
            }
        }
        if resp.drag_stopped() {
            self.timeline.auto_drag = None;
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if point_at(&self.project.automation, pos).is_none() {
                    let l = lane_at(pos).min(n - 1);
                    let lane = &mut self.project.automation[l];
                    lane.points.push(place(l, pos));
                    lane.sort();
                    self.dirty = true;
                }
            }
        }
        if resp.secondary_clicked() {
            let pos = resp.interact_pointer_pos();
            match pos.and_then(|pos| point_at(&self.project.automation, pos)) {
                Some((l, k)) => {
                    self.project.automation[l].points.remove(k);
                    self.timeline.auto_menu = None;
                    self.dirty = true;
                }
                None => self.timeline.auto_menu = pos.map(|pos| lane_at(pos).min(n - 1)),
            }
        }
        if self.timeline.auto_menu.is_some() {
            resp.context_menu(|ui| {
                let Some(l) = self
                    .timeline
                    .auto_menu
                    .filter(|&l| l < self.project.automation.len())
                else {
                    ui.close_menu();
                    return;
                };
                if ui.button("Clear points").clicked() {
                    self.project.automation[l].points.clear();
                    self.dirty = true;
                    ui.close_menu();
                }
                if ui.button("Delete lane").clicked() {
                    self.project.automation.remove(l);
                    self.timeline.auto_menu = None;
                    self.timeline.auto_drag = None;
                    self.dirty = true;
                    ui.close_menu();
                }
            });
        }

        // Paint.
        let painter = ui.painter_at(rect);
        let visible = ui.clip_rect();
        for (l, lane) in self.project.automation.iter().enumerate() {
            let r = lane_rect(l);
            let fill = if l % 2 == 0 { 28 } else { 33 };
            painter.rect_filled(r, 0.0, Color32::from_gray(fill));
            painter.line_segment(
                [r.left_bottom(), r.right_bottom()],
                Stroke::new(1.0_f32, Color32::from_gray(50)),
            );
            let color = Color32::from_rgb(230, 170, 60);
            // The line: flat before the first point and after the last.
            if let (Some(first), Some(last)) = (lane.points.first(), lane.points.last()) {
                let mut line = vec![Pos2::new(r.left(), y_of(l, first.value))];
                line.extend(
                    lane.points
                        .iter()
                        .map(|p| Pos2::new(x_of(p.at), y_of(l, p.value))),
                );
                line.push(Pos2::new(r.right(), y_of(l, last.value)));
                painter.add(egui::Shape::line(line, Stroke::new(1.5_f32, color)));
                for p in &lane.points {
                    painter.circle_filled(Pos2::new(x_of(p.at), y_of(l, p.value)), 3.5, color);
                }
            }
            // Its name, kept in view at the left.
            let track = self
                .project
                .tracks
                .get(lane.track)
                .map_or("?", |t| t.name.as_str());
            painter.text(
                Pos2::new(visible.left().max(r.left()) + 4.0, r.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{track} · {}", lane.target.label()),
                FontId::proportional(11.0),
                Color32::from_gray(200),
            );
        }
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

        // A block dropped from the drawer.
        if let Some(drag) = resp.dnd_release_payload::<BlockDrag>() {
            if let Some(pos) = resp.hover_pos() {
                self.place_clip(drag.0, snap_down(step_at(pos), snap), row_at(pos));
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
                    let len = self.project.block_len(before.block) as isize;
                    if let Some(c) = self.project.clips.get_mut(clip) {
                        if start != c.start {
                            // Trimming (or extending) the start moves where
                            // the block begins, so the notes stay put.
                            let moved = start as isize - before.start as isize;
                            c.start = start;
                            c.len = before.end() - start;
                            c.offset = (before.offset as isize + moved).rem_euclid(len) as usize;
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
                    // A MIDI block opens in the note editor right away.
                    if let Some(i) = self.timeline.selected {
                        let b = self.project.clips[i].block;
                        if self.project.blocks[b].is_midi() {
                            self.edit_notes(b);
                        }
                    }
                }
            }
        }
        if resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if !on_ruler(pos) {
                    match self.clip_at(rect, pos) {
                        Some((i, _)) => self.open_block(self.project.clips[i].block),
                        None => {
                            let b = self.edit_block();
                            self.place_clip(b, snap_down(step_at(pos), snap), row_at(pos));
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

    /// Places block `b` on the timeline, once through.
    fn place_clip(&mut self, b: usize, start: usize, row: usize) {
        let len = self.project.block_len(b);
        if b < self.project.blocks.len() && start + len <= MAX_SONG_STEPS {
            self.project.clips.push(Clip {
                block: b,
                row: row.min(MAX_ROWS - 1),
                start,
                len,
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
        let name = self.project.blocks[c.block].name.clone();
        ui.label(egui::RichText::new(&name).strong());
        let midi = self.project.blocks[c.block].is_midi();
        if ui
            .button(if midi { "Edit notes" } else { "Edit in Jam" })
            .clicked()
        {
            self.open_block(c.block);
            ui.close_menu();
        }
        let others = self.project.block_uses(c.block) - 1;
        let unique = egui::Button::new("Make unique");
        let hint = format!(
            "Give this copy its own block, so editing it\n\
             doesn't change the other {others} cop{} of {name}",
            if others == 1 { "y" } else { "ies" }
        );
        if ui
            .add_enabled(others > 0, unique)
            .on_hover_text(hint)
            .on_disabled_hover_text("This is the only copy of the block")
            .clicked()
        {
            if let Some(copy) = self.project.make_unique(i) {
                self.shared.edit_block.store(copy, Relaxed);
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

        // Blocks. Copies of the block being edited are brighter, so you can
        // see everything an edit will change.
        let edit = self.edit_block();
        for (i, c) in self.project.clips.iter().enumerate() {
            let r = clip_rect(rect, zoom, c);
            if r.right() < visible.left() || r.left() > visible.right() {
                continue;
            }
            let base = block_color(c.block);
            let fill = if c.block == edit || self.open_notes() == Some(c.block) {
                base
            } else {
                base.gamma_multiply(0.6)
            };
            painter.rect_filled(r, 3.0, fill);
            // Marks where the block repeats when stretched.
            let len = self.project.block_len(c.block);
            let mut repeat = c.start + (len - c.offset % len) % len;
            if repeat == c.start {
                repeat += len;
            }
            while repeat < c.end() {
                let x = rect.left() + repeat as f32 * zoom;
                painter.line_segment(
                    [Pos2::new(x, r.top() + 3.0), Pos2::new(x, r.bottom() - 3.0)],
                    Stroke::new(1.5_f32, Color32::from_black_alpha(110)),
                );
                repeat += len;
            }
            if let Some(m) = &self.project.blocks[c.block].midi {
                paint_clip_notes(&painter, r, rect.left(), zoom, c, &m.notes, len);
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
                    &self.project.blocks[c.block].name,
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

/// A MIDI block's notes in miniature inside its clip, so it's clear
/// what's where. `left` is where step 0 is.
fn paint_clip_notes(
    painter: &egui::Painter,
    r: Rect,
    left: f32,
    zoom: f32,
    c: &Clip,
    notes: &[crate::project::Note],
    len: usize,
) {
    let (Some(low), Some(high)) = (
        notes.iter().map(|n| n.pitch).min(),
        notes.iter().map(|n| n.pitch).max(),
    ) else {
        return;
    };
    let span = (high - low).max(12) as f32;
    let inner = r.shrink2(Vec2::new(0.0, 3.0));
    let painter = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let color = Color32::from_white_alpha(90);
    // Each time the block repeats within the clip.
    let mut origin = c.start as f32 - c.offset as f32;
    let mut drawn = 0;
    while origin < c.end() as f32 && drawn < 4000 {
        for n in notes {
            let x0 = left + (origin + n.start as f32 / PPS as f32) * zoom;
            let x1 = left + (origin + n.end() as f32 / PPS as f32) * zoom;
            if n.start as usize >= len * PPS as usize || x1 < r.left() || x0 > r.right() {
                continue;
            }
            let y = inner.bottom() - (n.pitch - low) as f32 / span * inner.height();
            painter.line_segment(
                [Pos2::new(x0, y), Pos2::new(x1.max(x0 + 1.0), y)],
                Stroke::new(2.0_f32, color),
            );
            drawn += 1;
        }
        origin += len as f32;
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
