// No console window behind the app on Windows (debug builds keep it for logs).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod chords;
mod drums;
mod export;
mod file;
mod fx;
mod help;
mod midi;
mod notes;
mod project;
mod sample;
mod synth;
mod timeline;
mod undo;
mod web;
mod widgets;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Vec2};

use audio::SCOPE_LEN;
use audio::{Event, Shared};
use midi::{Midi, Played, Route};
use project::{
    note_head, set_note_length, Effects, Grid, Kind, Project, Row, Sample, Step, Track, ACCENT,
    EMPTY_GRID, GHOST, LANES, MAX_RATCHET, MAX_TRACKS, NORMAL, STEPS,
};
use synth::{Param, Unit, PARAMS};
use undo::Undo;
use widgets::{
    disclosure_arrow, empty_fill, envelope, knob, lit_fill, meter, playhead, port_picker, scope,
    waveform, DRUM_ON, NOTE_HELD, NOTE_NAMES, NOTE_ON, SCOPE_SPANS,
};

const RECORDING: Color32 = Color32::from_rgb(220, 60, 60);
const SOLO: Color32 = Color32::from_rgb(230, 200, 60);
/// Notes from other tracks, shown faintly in the piano roll.
const OTHER_NOTE: Color32 = Color32::from_rgb(55, 75, 105);
/// How many past project snapshots to keep alive for the audio thread.
const HISTORY: usize = 8;

const TRACK_VOLUME: Param = Param {
    name: "Volume",
    min: 0.0,
    max: 1.0,
    default: 0.8,
    log: false,
    unit: Unit::Percent,
};

const GROUP_VOLUME: Param = Param {
    name: "Volume",
    min: 0.0,
    max: 1.0,
    default: 1.0,
    log: false,
    unit: Unit::Percent,
};

const PAN: Param = Param {
    name: "Pan",
    min: -1.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Pan,
};

const REVERB_SEND: Param = Param {
    name: "Reverb",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

const DELAY_SEND: Param = Param {
    name: "Delay",
    ..REVERB_SEND
};

const REVERB_SIZE: Param = Param {
    name: "Size",
    min: 0.0,
    max: 1.0,
    default: Effects::DEFAULT.reverb_size,
    log: false,
    unit: Unit::Percent,
};

const REVERB_TONE: Param = Param {
    name: "Tone",
    default: Effects::DEFAULT.reverb_tone,
    ..REVERB_SIZE
};

const DELAY_TIME: Param = Param {
    name: "Time",
    min: 1.0,
    max: 8.0,
    default: Effects::DEFAULT.delay_steps,
    log: false,
    unit: Unit::Steps,
};

const DELAY_FEEDBACK: Param = Param {
    name: "Feedback",
    min: 0.0,
    max: 0.9,
    default: Effects::DEFAULT.delay_feedback,
    log: false,
    unit: Unit::Percent,
};

const SAMPLE_START: Param = Param {
    name: "Start",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

const SAMPLE_END: Param = Param {
    name: "End",
    default: 1.0,
    ..SAMPLE_START
};

const SAMPLE_ATTACK: Param = Param {
    name: "Fade in",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Seconds,
};

const SAMPLE_RELEASE: Param = Param {
    name: "Release",
    min: 0.01,
    max: 3.0,
    default: 0.1,
    log: true,
    unit: Unit::Seconds,
};

const HUMANIZE: Param = Param {
    name: "Humanize",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    // Reopen the song from last time, or start with the demo.
    let last = file::last_song().and_then(|path| Some((file::open(&path).ok()?, path)));
    let (project, path, bpm) = match last {
        Some((song, path)) => (song.project, Some(path), song.bpm),
        None => (Project::demo(), None, 120.0),
    };

    // Sized to fit the 1366x768 screen of an 11" MacBook Air.
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        // Matches daw.desktop, so Linux desktops (e.g. Hyprland) show the
        // right name and icon for the window.
        .with_app_id("daw");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "daw",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(project, path, bpm)))),
    )
}

/// The web version: draws on the page's canvas (see `web/index.html`).
#[cfg(target_arch = "wasm32")]
fn main() {
    web::show_crashes();
    // Reopen the song kept from last visit, or start with the demo.
    let kept = web::stored_song()
        .and_then(|(name, bytes)| Some((file::from_bytes(&name, bytes).ok()?, name)));
    let (project, path, bpm) = match kept {
        Some((song, name)) => {
            let path = (!name.is_empty()).then(|| PathBuf::from(name));
            (song.project, path, song.bpm)
        }
        None => (Project::demo(), None, 120.0),
    };
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                web::canvas(),
                eframe::WebOptions::default(),
                Box::new(move |_cc| Ok(Box::new(App::new(project, path, bpm)))),
            )
            .await
            .expect("couldn't start daw");
    });
}

/// The two halves of the app.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Jam,
    Arrange,
}

/// Which parts of the window are showing.
#[derive(Clone, Copy, PartialEq)]
struct View {
    song: bool,
    tracks: bool,
    piano: bool,
    panel: bool,
    /// Show other tracks' notes faintly in the piano roll.
    others: bool,
}

impl View {
    const ALL: View = View {
        song: true,
        tracks: true,
        piano: true,
        panel: true,
        others: true,
    };
}

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

/// What to do once any unsaved changes are saved or discarded.
#[derive(Clone)]
enum Then {
    NewSong,
    Demo,
    Open(Opened),
    Quit,
}

/// A song file that's been read in, ready to open.
#[derive(Clone)]
struct Opened {
    /// Where it came from; on the web, just its name.
    path: PathBuf,
    bytes: Vec<u8>,
}

/// The track to select in a newly opened song: the first melodic one, so
/// the piano roll shows something.
fn first_melodic(project: &Project) -> usize {
    project
        .tracks
        .iter()
        .position(Track::is_melodic)
        .unwrap_or(0)
}

struct App {
    shared: Arc<Shared>,
    /// The editable project; changes are published to the engine as snapshots.
    project: Project,
    /// Recent snapshots, kept so the audio thread is never the one to free one.
    history: VecDeque<Arc<Project>>,
    /// Earlier versions of the song, for Ctrl+Z.
    undo: Undo<Project>,
    /// The project changed this frame and needs publishing.
    dirty: bool,
    events: SyncSender<Event>,
    midi: Midi,
    audio_error: Option<String>,
    clipboard: Option<Vec<Grid>>,
    /// Track shown in the side panel and piano roll.
    selected: usize,
    /// Samples decoded on background threads (the desktop app only).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    load_tx: Sender<Result<Sample, String>>,
    load_rx: Receiver<Result<Sample, String>>,
    /// Files still being decoded.
    loading: usize,
    load_error: Option<String>,
    /// Keys played while playing are written into the pattern.
    recording: bool,
    /// The computer-keyboard key holding a note, its track and MIDI note.
    held_key: Option<(egui::Key, usize, u8)>,
    show_help: bool,
    show_about: bool,
    show_fx: bool,
    /// Jam (make patterns and blocks) or Arrange (the timeline).
    tab: Tab,
    timeline: timeline::State,
    /// The note editor for MIDI blocks, in Arrange.
    notes: notes::State,
    /// A piano-roll note being dragged longer or shorter: (lane, its first step).
    note_drag: Option<(usize, usize)>,
    /// The note a synth track is droning (playing nonstop), if any:
    /// (track, MIDI note).
    drone: Option<(usize, u8)>,
    /// The Drone's pitch, in Hz: anywhere, not just on the 12 notes.
    drone_hz: f32,
    /// The theremin pad's note while it's held: (track, MIDI note).
    theremin: Option<(usize, u8)>,
    /// The piano-roll column the pointer was over last frame, for naming
    /// its chord.
    roll_hover: Option<usize>,
    /// Steps being painted or erased by dragging across them.
    paint: Option<Paint>,
    /// The tool bar's tool.
    tool: Tool,
    /// The oscilloscope and envelope drawing in the instrument panel.
    show_scope: bool,
    /// Instrument panel sections popped out into their own windows, by
    /// title, with where each window was (x, y, width, height).
    popped: Vec<(String, Option<[f32; 4]>)>,
    /// `popped` changed and should be remembered once the mouse is up.
    popped_changed: bool,
    /// What the level meter shows: recent peaks, falling slowly, and how
    /// hard the limiter is working.
    meter_levels: [f32; 2],
    meter_limiting: f32,
    /// Instrument panel sections closed with their ×, by title.
    closed_sections: Vec<String>,
    /// `closed_sections` changed and should be remembered.
    sections_changed: bool,
    /// Which of `SCOPE_SPANS` the scope shows.
    scope_span: usize,
    /// The scope had something to show last frame, so keep it moving.
    scope_busy: bool,
    /// Waiting for "save changes first?" to be answered.
    confirm: Option<Then>,
    /// Extra tracks picked with Ctrl/Cmd-click, for grouping.
    picked: Vec<usize>,
    /// A group shown in the side panel instead of a track.
    selected_group: Option<u32>,
    view: View,
    /// Notes played on MIDI controllers, for Rec.
    played: Receiver<Played>,
    show_midi_inputs: bool,
    /// The song file being edited, once saved or opened.
    file: Option<PathBuf>,
    /// Changed since the last save or open.
    modified: bool,
    /// Tempo lives in the engine, so it's compared to what was saved.
    saved_bpm: f32,
    /// Closing has been confirmed; don't ask again.
    may_close: bool,
    /// The window title last set.
    title: String,
    /// Finished exports: a message to show, or an error.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    export_tx: Sender<Result<String, String>>,
    export_rx: Receiver<Result<String, String>>,
    exporting: bool,
    /// A short success message, e.g. "Exported song.wav".
    notice: Option<String>,
    /// Kept so sound keeps playing; dropping it stops audio. (The web
    /// version keeps it with the handler that starts it on first click.)
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    /// Songs and samples picked in the browser's file dialog.
    #[cfg(target_arch = "wasm32")]
    inbox_tx: Sender<web::Picked>,
    #[cfg(target_arch = "wasm32")]
    inbox_rx: Receiver<web::Picked>,
    /// Counts published changes, so autosave knows when there's news.
    #[cfg(target_arch = "wasm32")]
    edits: u64,
    /// What autosave last kept: the edit count and tempo.
    #[cfg(target_arch = "wasm32")]
    autosaved: (u64, f32),
}

impl App {
    fn new(project: Project, path: Option<PathBuf>, bpm: f32) -> Self {
        let snapshot = Arc::new(project.clone());
        let shared = Arc::new(Shared::new(snapshot.clone()));
        let (events, events_rx) = sync_channel(256);
        let (midi_tx, midi_rx) = sync_channel(1024);
        let (played_tx, played) = channel();
        let midi = Midi::new(shared.clone(), events.clone(), played_tx, midi_rx);
        let stream = audio::start(shared.clone(), events_rx, midi_tx);
        let audio_error = stream.as_ref().err().cloned();
        #[cfg(target_arch = "wasm32")]
        if let Ok(stream) = stream {
            web::play_on_first_input(stream);
        }
        let (load_tx, load_rx) = channel();
        let (export_tx, export_rx) = channel();
        #[cfg(target_arch = "wasm32")]
        let (inbox_tx, inbox_rx) = channel();
        shared.set_bpm(bpm);
        let selected = first_melodic(&project);
        let undo = Undo::new(project.clone());
        App {
            shared,
            project,
            history: VecDeque::from([snapshot]),
            undo,
            dirty: false,
            events,
            midi,
            audio_error,
            clipboard: None,
            selected,
            load_tx,
            load_rx,
            loading: 0,
            load_error: None,
            recording: false,
            held_key: None,
            show_help: false,
            show_about: false,
            show_fx: false,
            tab: Tab::Jam,
            timeline: timeline::State::default(),
            notes: notes::State::default(),
            note_drag: None,
            paint: None,
            tool: Tool::Draw,
            roll_hover: None,
            drone: None,
            drone_hz: synth::midi_to_hz(48),
            theremin: None,
            show_scope: true,
            popped: file::popped_sections(),
            popped_changed: false,
            meter_levels: [0.0; 2],
            meter_limiting: 0.0,
            closed_sections: file::closed_sections(),
            sections_changed: false,
            scope_span: 1,
            scope_busy: false,
            confirm: None,
            picked: Vec::new(),
            selected_group: None,
            view: View::ALL,
            played,
            show_midi_inputs: false,
            file: path,
            modified: false,
            saved_bpm: bpm,
            may_close: false,
            title: String::new(),
            export_tx,
            export_rx,
            exporting: false,
            notice: None,
            #[cfg(not(target_arch = "wasm32"))]
            _stream: stream.ok(),
            #[cfg(target_arch = "wasm32")]
            inbox_tx,
            #[cfg(target_arch = "wasm32")]
            inbox_rx,
            #[cfg(target_arch = "wasm32")]
            edits: 0,
            #[cfg(target_arch = "wasm32")]
            autosaved: (0, bpm),
        }
    }
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
    fn bar(ui: &mut egui::Ui, tool: &mut Tool, select: bool) {
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

/// A drag across steps that turns them on or off, like a pencil or an
/// eraser. It stays on the track (and piano-roll row) it started on.
#[derive(Clone, Copy, PartialEq)]
struct Paint {
    track: usize,
    /// The piano-roll row, or `None` in the track's step row.
    lane: Option<usize>,
    /// Painting notes in, or erasing them.
    on: bool,
    /// Where the pointer was last frame, so a fast drag doesn't skip steps.
    last_x: Option<f32>,
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

/// The preset whose sound the knobs are set to, or "Custom" once any
/// knob differs from every preset.
fn preset_name(values: &synth::Values) -> &'static str {
    if *values == PARAMS.map(|p| p.default) {
        return "Default";
    }
    (0..synth::PRESETS.len())
        .find(|&i| synth::preset(i) == *values)
        .map_or("Custom", |i| synth::PRESETS[i].0)
}

/// A pitch in Hz as a MIDI note to hold and how far to bend it (0-1
/// semitones), for pitches between the usual notes.
fn free_pitch(hz: f32) -> (u8, f32) {
    let exact = (69.0 + 12.0 * (hz / 440.0).log2()).clamp(0.0, 127.0);
    let note = exact.floor();
    (note as u8, exact - note)
}

/// Every section the instrument panel can show, in order.
const SECTIONS: [&str; 9] = [
    "Scope",
    "Oscillators",
    "Filter",
    "Envelope (ADSR)",
    "LFO",
    "Arpeggiator",
    "Sample",
    "Mix",
    "Theremin",
];

/// Whether a panel section belongs on a synth track (`synth`) or a sample
/// track.
fn section_fits(title: &str, synth: bool) -> bool {
    match title {
        "Sample" => !synth,
        "Scope" | "Mix" => true,
        _ => synth,
    }
}

/// The instrument panel's sections, while it's being drawn: each one can
/// be folded (click its heading; egui remembers that while the app runs),
/// popped out into its own window with ⬈, or closed with × and brought
/// back from + Section or the View menu.
struct Sections {
    /// Closed sections, by title.
    closed: Vec<String>,
    /// A section whose × was clicked this frame.
    close: Option<String>,
    /// A section whose ⬈ was clicked this frame.
    pop: Option<String>,
}

impl Sections {
    fn is_open(&self, title: &str) -> bool {
        !self.closed.iter().any(|c| c == title)
    }

    /// Shows a section unless it's closed. `open` is whether it starts
    /// unfolded.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        open: bool,
        body: impl FnOnce(&mut egui::Ui),
    ) {
        if !self.is_open(title) {
            return;
        }
        let id = ui.make_persistent_id(("panel section", title));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open)
            .show_header(ui, |ui| {
                ui.label(RichText::new(title).small().strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = ui
                        .small_button("×")
                        .on_hover_text("Close this section (+ Section brings it back)");
                    if close.clicked() {
                        self.close = Some(title.to_string());
                    }
                    let pop = ui.small_button("⬈").on_hover_text(
                        "Pop out into its own window, to move anywhere.\n\
                         Its × puts it back here.",
                    );
                    if pop.clicked() {
                        self.pop = Some(title.to_string());
                    }
                });
            })
            .body(body);
    }
}

/// What File > Export writes.
#[derive(Clone, Copy)]
enum ExportKind {
    Wav,
    Midi,
}

/// Computer keys laid out like a piano, as in FL Studio: the home row is
/// the white keys and the row above has the black keys.
fn key_semitone(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    let keys = [A, W, S, E, D, F, T, G, Y, H, U, J, K, O, L, P];
    keys.iter().position(|&k| k == key).map(|i| i as u8)
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let playing = self.shared.playing.load(Relaxed);
        // Undo or deleting a pattern or block can leave the editor
        // pointing past the last one.
        let last = self.project.patterns.len() - 1;
        if self.shared.edit_pattern.load(Relaxed) as usize > last {
            self.shared.edit_pattern.store(last as u8, Relaxed);
        }
        self.shared.edit_block.store(self.edit_block(), Relaxed);
        // Arrange plays the timeline; Jam plays a pattern or a chain.
        self.shared
            .song_mode
            .store(self.tab == Tab::Arrange, Relaxed);
        if self
            .timeline
            .selected
            .is_some_and(|i| i >= self.project.clips.len())
        {
            self.timeline.selected = None;
        }
        // Nothing is recorded during the count-in bar.
        let rolling = playing && !self.shared.counting_in.load(Relaxed);
        self.shared.count_in.store(self.recording, Relaxed);
        if ctx.input(|i| i.viewport().close_requested()) && !self.may_close && self.unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Then::Quit);
        }
        self.file_shortcuts(ctx);
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.set_playing(!playing);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
                self.show_help = !self.show_help;
            }
            self.view_keys(ctx);
            self.undo_keys(ctx);
            // With notes selected, Delete and the rest are the note editor's.
            if self.tab == Tab::Arrange && !self.note_keys(ctx) {
                self.timeline_keys(ctx);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G)) {
                self.group_picked();
            }
            self.computer_keyboard(ctx, rolling);
        }
        self.shared.selected_track.store(self.selected, Relaxed);
        while let Ok(result) = self.export_rx.try_recv() {
            self.exporting = false;
            match result {
                Ok(message) => self.notice = Some(message),
                Err(e) => self.load_error = Some(e),
            }
        }
        while let Ok(p) = self.played.try_recv() {
            if self.recording && rolling {
                self.record(p.track, p.note, p.velocity);
            }
        }

        while let Ok(result) = self.load_rx.try_recv() {
            self.loading -= 1;
            match result {
                Ok(sample) => self.add_track(Track::sampler(Arc::new(sample), false)),
                Err(e) => self.load_error = Some(e),
            }
        }
        // Desktop drops come as paths; browser drops as the file's bytes.
        let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = f.path {
                self.load_file(ctx, path);
                continue;
            }
            if let Some(bytes) = f.bytes {
                self.load_bytes(&f.name, bytes.to_vec());
            }
        }
        #[cfg(target_arch = "wasm32")]
        while let Ok(picked) = self.inbox_rx.try_recv() {
            match picked {
                web::Picked::Song(name, bytes) => {
                    let path = PathBuf::from(name);
                    self.request(ctx, Then::Open(Opened { path, bytes }));
                }
                web::Picked::Sample(name, bytes) => self.load_bytes(&name, bytes),
            }
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui, ctx));

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.add_space(4.0);
            for err in [&self.audio_error, &self.midi.error, &self.load_error]
                .into_iter()
                .flatten()
            {
                ui.colored_label(Color32::RED, err);
            }
            if self.exporting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Exporting…");
                });
            } else if let Some(notice) = &self.notice {
                ui.colored_label(Color32::LIGHT_GREEN, notice);
            }
            self.transport_row(ui, playing);
            self.midi_row(ui);
            ui.add_space(4.0);
        });

        if self.view.panel {
            egui::SidePanel::right("instrument")
                .resizable(false)
                .exact_width(290.0)
                .show(ctx, |ui| {
                    // Scrolls when the window is short (the scope adds height).
                    egui::ScrollArea::vertical().show(ui, |ui| match self.selected_group {
                        Some(id) => self.group_panel(ui, id),
                        None => self.instrument_panel(ui),
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.tab == Tab::Arrange {
                    self.timeline(ui, playing);
                    ui.add_space(8.0);
                    self.block_drawer(ui);
                    if let Some(b) = self.open_notes() {
                        ui.add_space(8.0);
                        self.note_editor(ui, b, playing);
                    }
                    return;
                }
                if self.view.song {
                    self.chain_row(ui, playing);
                    ui.add_space(6.0);
                    self.library_row(ui);
                    ui.add_space(6.0);
                }
                let edit = self.shared.edit_pattern.load(Relaxed) as usize;
                // Only show the playhead on the pattern that's actually playing.
                let current = (playing && self.shared.play_pattern.load(Relaxed) as usize == edit)
                    .then(|| self.shared.current_step.load(Relaxed));
                if self.view.tracks {
                    self.track_list(ui, edit, current);
                    ui.add_space(4.0);
                    self.add_row(ui, ctx);
                }
                let melodic = self
                    .project
                    .tracks
                    .get(self.selected)
                    .is_some_and(Track::is_melodic);
                if self.view.piano && melodic {
                    ui.add_space(8.0);
                    self.piano_roll(ui, edit, current);
                }
                if !self.view.song && !self.view.tracks && !(self.view.piano && melodic) {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.weak(
                            "Nothing to show here. Use the View menu, or F9 to show everything.",
                        );
                    });
                }
            });
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                let painter = ui.painter();
                let rect = ui.max_rect();
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop to add as sample tracks",
                    egui::FontId::proportional(24.0),
                    Color32::WHITE,
                );
            }
        });

        help::guide(ctx, &mut self.show_help);
        help::about(ctx, &mut self.show_about);
        self.fx_window(ctx);
        self.popped_windows(ctx);
        self.confirm_dialog(ctx);
        self.midi_inputs_window(ctx);

        if self.dirty {
            self.publish();
            self.undo.changed();
        }
        // A knob drag or a burst of typing is one undo step, settled once
        // it's over.
        if !ctx.input(|i| i.pointer.any_down()) && !ctx.wants_keyboard_input() {
            self.undo.settle(&self.project);
        }
        match ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.interact_pos())) {
            Some(pos) => {
                if let Some(p) = &mut self.paint {
                    p.last_x = pos.map(|pos| pos.x).or(p.last_x);
                }
            }
            None => {
                self.paint = None;
                // The theremin's note ends when the mouse comes up, even if
                // its pad was hidden mid-drag.
                if let Some((track, note)) = self.theremin.take() {
                    let _ = self.events.try_send(Event::Release { track, note });
                }
            }
        }
        self.update_title(ctx);
        #[cfg(target_arch = "wasm32")]
        {
            self.midi.send_queued();
            // Autosave once a change is finished (not mid-drag).
            let news = self.autosaved != (self.edits, self.shared.bpm());
            if news && !ctx.input(|i| i.pointer.any_down()) {
                self.autosave();
            }
        }
        // The engine only feeds the scope while it's showing.
        let in_panel = self.view.panel && self.selected_group.is_none();
        let scope = self.show_scope && (in_panel || self.is_popped("Scope"));
        self.shared.scope_on.store(scope, Relaxed);
        // Only redraw continuously while playing (or the scope has sound
        // to show), to keep CPU low when idle.
        if playing || (scope && self.scope_busy) {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}

impl App {
    fn publish(&mut self) {
        let snapshot = Arc::new(self.project.clone());
        self.shared.publish(snapshot.clone());
        self.history.push_back(snapshot);
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.dirty = false;
        self.modified = true;
        #[cfg(target_arch = "wasm32")]
        {
            self.edits += 1;
        }
    }

    /// Ctrl+Z undoes; Ctrl+Shift+Z or Ctrl+Y redoes (Cmd on Mac).
    fn undo_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        // Check Shift+Z first: plain Ctrl+Z would also match it.
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || pressed(Modifiers::COMMAND, Key::Y)
        {
            self.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
    }

    fn undo(&mut self) {
        if let Some(project) = self.undo.undo(&self.project) {
            self.restore(project);
        }
    }

    fn redo(&mut self) {
        if let Some(project) = self.undo.redo(&self.project) {
            self.restore(project);
        }
    }

    /// Switches to an earlier or later version of the song.
    fn restore(&mut self, project: Project) {
        self.project = project;
        self.selected = self
            .selected
            .min(self.project.tracks.len().saturating_sub(1));
        if self
            .selected_group
            .is_some_and(|id| self.project.group(id).is_none())
        {
            self.selected_group = None;
        }
        self.picked.clear();
        self.notes.deselect();
        self.publish();
    }

    fn unsaved(&self) -> bool {
        self.modified || self.shared.bpm() != self.saved_bpm
    }

    fn song_name(&self) -> String {
        self.file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// Shows the song's name, with a dot while there are unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let dot = if self.unsaved() { "• " } else { "" };
        let title = format!("{dot}{} - daw", self.song_name());
        if title != self.title {
            #[cfg(target_arch = "wasm32")]
            web::set_title(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Ctrl+N, Ctrl+O, Ctrl+S and Ctrl+Shift+S (Cmd on Mac).
    fn file_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        // Check Shift+S first: plain Ctrl+S would also match it.
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save_as();
        } else if pressed(Modifiers::COMMAND, Key::S) {
            self.save();
        } else if pressed(Modifiers::COMMAND, Key::O) {
            self.open_dialog(ctx);
        } else if pressed(Modifiers::COMMAND, Key::N) {
            self.request(ctx, Then::NewSong);
        }
    }

    /// Does `then`, first asking to save if there are unsaved changes.
    fn request(&mut self, ctx: &egui::Context, then: Then) {
        if self.unsaved() {
            self.confirm = Some(then);
        } else {
            self.carry_out(ctx, then);
        }
    }

    fn carry_out(&mut self, ctx: &egui::Context, then: Then) {
        match then {
            Then::NewSong => self.replace_song(Project::empty(), None, 120.0),
            Then::Demo => self.replace_song(Project::demo(), None, 120.0),
            Then::Open(Opened { path, bytes }) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                match file::from_bytes(&name, bytes) {
                    Ok(song) => {
                        #[cfg(not(target_arch = "wasm32"))]
                        file::remember(&path);
                        self.replace_song(song.project, Some(path), song.bpm);
                    }
                    Err(e) => self.load_error = Some(e),
                }
            }
            Then::Quit => {
                self.may_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn replace_song(&mut self, project: Project, file: Option<PathBuf>, bpm: f32) {
        self.shared.playing.store(false, Relaxed);
        self.stop_drone();
        self.selected = first_melodic(&project);
        self.project = project;
        self.selected_group = None;
        self.picked.clear();
        self.shared.edit_pattern.store(0, Relaxed);
        self.shared.set_bpm(bpm);
        self.publish();
        self.undo.reset(self.project.clone());
        self.file = file;
        self.modified = false;
        self.saved_bpm = bpm;
        self.load_error = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("daw song", &[file::EXTENSION])
            .pick_file();
        if let Some(path) = picked {
            match std::fs::read(&path) {
                Ok(bytes) => self.request(ctx, Then::Open(Opened { path, bytes })),
                Err(e) => self.load_error = Some(format!("Couldn't open the song: {e}")),
            }
        }
    }

    /// The browser's picker answers later, through `inbox`.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        web::pick(ctx, self.inbox_tx.clone(), true);
    }

    /// Saves to the current file, or asks where. Returns false if cancelled
    /// or it failed.
    fn save(&mut self) -> bool {
        match self.file.clone() {
            Some(path) => self.write_song(path),
            None => self.save_as(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) -> bool {
        let picked = rfd::FileDialog::new()
            .add_filter("daw song", &[file::EXTENSION])
            .set_file_name(format!("{}.{}", self.song_name(), file::EXTENSION))
            .save_file();
        match picked {
            Some(path) => self.write_song(path.with_extension(file::EXTENSION)),
            None => false,
        }
    }

    /// Browsers decide where downloads go, so this just names the file.
    #[cfg(target_arch = "wasm32")]
    fn save_as(&mut self) -> bool {
        let name = format!("{}.{}", self.song_name(), file::EXTENSION);
        self.write_song(PathBuf::from(name))
    }

    /// Asks where to export, then renders on a background thread so the
    /// app stays responsive.
    #[cfg(not(target_arch = "wasm32"))]
    fn export(&mut self, ctx: &egui::Context, kind: ExportKind) {
        let (label, ext) = match kind {
            ExportKind::Wav => ("WAV audio", "wav"),
            ExportKind::Midi => ("MIDI file", "mid"),
        };
        let picked = rfd::FileDialog::new()
            .add_filter(label, &[ext])
            .set_file_name(format!("{}.{ext}", self.song_name()))
            .save_file();
        let Some(path) = picked else { return };
        let path = path.with_extension(ext);
        let project = self.project.clone();
        let bpm = self.shared.bpm();
        let edit = self.edit_now();
        let tx = self.export_tx.clone();
        let ctx = ctx.clone();
        self.exporting = true;
        self.notice = None;
        thread::spawn(move || {
            let bytes = match kind {
                ExportKind::Wav => export::wav(&project, bpm, edit),
                ExportKind::Midi => Ok(export::midi(&project, bpm, edit)),
            };
            let result = bytes.and_then(|bytes| {
                std::fs::write(&path, bytes).map_err(|e| format!("Couldn't export: {e}"))
            });
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let _ = tx.send(result.map(|()| format!("Exported {name}")));
            ctx.request_repaint();
        });
    }

    /// Renders right away (browsers have no background threads here) and
    /// downloads the result.
    #[cfg(target_arch = "wasm32")]
    fn export(&mut self, _ctx: &egui::Context, kind: ExportKind) {
        let bpm = self.shared.bpm();
        let edit = self.edit_now();
        let (bytes, ext, mime) = match kind {
            ExportKind::Wav => (export::wav(&self.project, bpm, edit), "wav", "audio/wav"),
            ExportKind::Midi => (
                Ok(export::midi(&self.project, bpm, edit)),
                "mid",
                "audio/midi",
            ),
        };
        let name = format!("{}.{ext}", self.song_name());
        match bytes.and_then(|bytes| web::download(&name, &bytes, mime)) {
            Ok(()) => self.notice = Some(format!("Exported {name}")),
            Err(e) => self.load_error = Some(e),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_song(&mut self, path: PathBuf) -> bool {
        let bpm = self.shared.bpm();
        match file::save(&path, &self.project, bpm) {
            Ok(()) => {
                file::remember(&path);
                self.file = Some(path);
                self.modified = false;
                self.saved_bpm = bpm;
                self.load_error = None;
                true
            }
            Err(e) => {
                self.load_error = Some(e);
                false
            }
        }
    }

    /// Downloads the song, and keeps it in the browser for next visit.
    #[cfg(target_arch = "wasm32")]
    fn write_song(&mut self, path: PathBuf) -> bool {
        let bpm = self.shared.bpm();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let saved = file::to_bytes(&self.project, bpm)
            .and_then(|bytes| web::download(&name, &bytes, "application/zip"));
        match saved {
            Ok(()) => {
                self.file = Some(path);
                self.modified = false;
                self.saved_bpm = bpm;
                self.load_error = None;
                self.autosave();
                true
            }
            Err(e) => {
                self.load_error = Some(e);
                false
            }
        }
    }

    /// Keeps the song in the browser, so it's back after a reload or next
    /// visit. Runs a moment after each change.
    #[cfg(target_arch = "wasm32")]
    fn autosave(&mut self) {
        let bpm = self.shared.bpm();
        self.autosaved = (self.edits, bpm);
        let name = self
            .file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kept =
            file::to_bytes(&self.project, bpm).and_then(|bytes| web::store_song(&name, &bytes));
        if let Err(e) = kept {
            self.load_error = Some(e);
        }
    }

    /// Briefly plays one lane of a track.
    fn preview(&self, track: usize, lane: usize) {
        if let Some(t) = self.project.tracks.get(track) {
            let note = t.note(lane);
            let _ = self.events.try_send(Event::Preview { track, note });
        }
    }

    /// What the Jam view has open, for exports.
    fn edit_now(&self) -> audio::Edit {
        audio::Edit {
            pattern: self.edit_pattern(),
            block: self.edit_block(),
        }
    }

    /// Rec: writes a played note into the pattern at the step that's playing.
    fn record(&mut self, track: usize, note: u8, velocity: u8) {
        let pattern = self.edit_pattern();
        let step = self.shared.current_step.load(Relaxed);
        if let Some(t) = self.project.tracks.get_mut(track) {
            let lane = t.lane_for(note);
            t.patterns[pattern][lane][step] = Some(Step {
                velocity,
                ..Step::HIT
            });
            self.dirty = true;
        }
    }

    fn edit_pattern(&self) -> usize {
        self.shared.edit_pattern.load(Relaxed) as usize
    }

    fn add_track(&mut self, track: Track) {
        if self.project.tracks.len() >= MAX_TRACKS {
            self.load_error = Some(format!("Can't add {}: {MAX_TRACKS} tracks max", track.name));
            return;
        }
        self.project.add_track(track);
        self.selected = self.project.tracks.len() - 1;
        self.load_error = None;
        self.dirty = true;
    }

    fn add_synth(&mut self) {
        let n = 1 + self
            .project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, Kind::Synth(_)))
            .count();
        self.add_track(Track::synth(&format!("Synth {n}")));
    }

    /// Adds built-in drum `i`, on the drum channel, into the Drums group
    /// if there is one.
    fn add_drum(&mut self, i: usize) {
        let Some(sample) = project::builtin_drums().into_iter().nth(i) else {
            return;
        };
        let mut track = Track::sampler(sample, false);
        track.midi_channel = Some(9);
        track.midi_note = drums::NOTES[i];
        track.group = self
            .project
            .groups
            .iter()
            .find(|g| g.name == "Drums")
            .map(|g| g.id);
        self.add_track(track);
    }

    fn delete_selected(&mut self) {
        self.delete_track(self.selected);
    }

    /// Stops the Drone button's note.
    fn stop_drone(&mut self) {
        if let Some((track, note)) = self.drone.take() {
            let _ = self.events.try_send(Event::Release { track, note });
        }
    }

    fn delete_track(&mut self, i: usize) {
        if i < self.project.tracks.len() {
            // The engine's voices don't move with the tracks, so a drone
            // can't follow its track to a new place.
            self.stop_drone();
            self.project.remove_track(i);
            self.midi.track_deleted(i);
            self.selected = self
                .selected
                .min(self.project.tracks.len().saturating_sub(1));
            self.picked.clear();
            self.selected_group = None;
            self.dirty = true;
        }
    }

    /// Ctrl+G: puts the picked tracks (or just the selected one) in a new group.
    fn group_picked(&mut self) {
        let mut tracks = self.picked.clone();
        if !tracks.contains(&self.selected) {
            tracks.push(self.selected);
        }
        tracks.retain(|&i| i < self.project.tracks.len());
        if tracks.is_empty() {
            return;
        }
        let name = format!("Group {}", self.project.groups.len() + 1);
        let id = self.project.new_group(&tracks, &name);
        self.selected_group = Some(id);
        self.picked.clear();
        self.dirty = true;
    }

    /// F5-F8 show or hide each part of the window; F9 shows everything.
    fn view_keys(&mut self, ctx: &egui::Context) {
        use egui::Key::*;
        let v = &mut self.view;
        ctx.input(|i| {
            for (key, shown) in [
                (F5, &mut v.song),
                (F6, &mut v.tracks),
                (F7, &mut v.piano),
                (F8, &mut v.panel),
            ] {
                if i.key_pressed(key) {
                    *shown = !*shown;
                }
            }
            if i.key_pressed(F9) {
                *v = View {
                    others: v.others,
                    ..View::ALL
                };
            }
        });
    }

    /// Opens the file picker and loads the chosen audio files.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_samples(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio", &sample::EXTENSIONS)
            .pick_files();
        for path in picked.into_iter().flatten() {
            self.load_file(ctx, path);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn pick_samples(&mut self, ctx: &egui::Context) {
        web::pick(ctx, self.inbox_tx.clone(), false);
    }

    /// Decodes audio that's already in memory, e.g. dropped in a browser.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn load_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        match sample::decode(name, bytes) {
            Ok(sample) => self.add_track(Track::sampler(Arc::new(sample), false)),
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Decodes a file on a background thread so the UI doesn't freeze.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_file(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.loading += 1;
        let tx = self.load_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(sample::load(&path));
            ctx.request_repaint();
        });
    }

    fn copy_pattern(&mut self) {
        let edit = self.edit_pattern();
        self.clipboard = Some(
            self.project
                .tracks
                .iter()
                .map(|t| t.patterns[edit])
                .collect(),
        );
    }

    fn paste_pattern(&mut self) {
        let edit = self.edit_pattern();
        if let Some(copied) = &self.clipboard {
            for (track, steps) in self.project.tracks.iter_mut().zip(copied) {
                track.patterns[edit] = *steps;
            }
            self.dirty = true;
        }
    }

    fn clear_pattern(&mut self) {
        let edit = self.edit_pattern();
        for track in &mut self.project.tracks {
            track.patterns[edit] = EMPTY_GRID;
        }
        self.dirty = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                let item = |ui: &mut egui::Ui, label: &str, keys: &str| {
                    let clicked = ui
                        .add(egui::Button::new(label).shortcut_text(keys))
                        .clicked();
                    if clicked {
                        ui.close_menu();
                    }
                    clicked
                };
                if item(ui, "New song", "Ctrl+N") {
                    self.request(ctx, Then::NewSong);
                }
                if item(ui, "Open…", "Ctrl+O") {
                    self.open_dialog(ctx);
                }
                if item(ui, "Save", "Ctrl+S") {
                    self.save();
                }
                // In a browser, Save downloads the song, so Save as is the same.
                #[cfg(not(target_arch = "wasm32"))]
                if item(ui, "Save as…", "Ctrl+Shift+S") {
                    self.save_as();
                }
                ui.separator();
                // Exports the timeline, or the current pattern four times if it's empty.
                if item(ui, "Export WAV…", "") {
                    self.export(ctx, ExportKind::Wav);
                }
                if item(ui, "Export MIDI…", "") {
                    self.export(ctx, ExportKind::Midi);
                }
                ui.separator();
                if item(ui, "Load demo song", "") {
                    self.request(ctx, Then::Demo);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = egui::Button::new("Undo  (Ctrl+Z)");
                if ui.add_enabled(self.undo.can_undo(), undo).clicked() {
                    self.undo();
                    ui.close_menu();
                }
                let redo = egui::Button::new("Redo  (Ctrl+Shift+Z)");
                if ui.add_enabled(self.undo.can_redo(), redo).clicked() {
                    self.redo();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Copy pattern").clicked() {
                    self.copy_pattern();
                    ui.close_menu();
                }
                let paste = egui::Button::new("Paste pattern");
                if ui.add_enabled(self.clipboard.is_some(), paste).clicked() {
                    self.paste_pattern();
                    ui.close_menu();
                }
                if ui.button("Clear pattern").clicked() {
                    self.clear_pattern();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Add synth track").clicked() {
                    self.add_synth();
                    ui.close_menu();
                }
                if ui.button("Add sample track…").clicked() {
                    ui.close_menu();
                    self.pick_samples(ctx);
                }
                let has_track = self.selected < self.project.tracks.len();
                ui.separator();
                let group = egui::Button::new("Group selected tracks  (Ctrl+G)");
                if ui.add_enabled(has_track, group).clicked() {
                    self.group_picked();
                    ui.close_menu();
                }
                ui.weak("Ctrl/Cmd-click track names to pick several.");
                ui.separator();
                let delete = egui::Button::new("Delete selected track");
                if ui.add_enabled(has_track, delete).clicked() {
                    self.delete_selected();
                    ui.close_menu();
                }
            });
            ui.menu_button("View", |ui| {
                let v = &mut self.view;
                ui.checkbox(&mut v.song, "Chain and patterns  (F5)");
                ui.checkbox(&mut v.tracks, "Tracks  (F6)");
                ui.checkbox(&mut v.piano, "Piano roll  (F7)");
                ui.checkbox(&mut v.panel, "Instrument panel  (F8)");
                ui.checkbox(&mut v.others, "Other tracks' notes in piano roll");
                ui.checkbox(
                    &mut self.show_scope,
                    "Scope and envelope (instrument panel)",
                );
                ui.menu_button("Instrument panel sections", |ui| {
                    for t in SECTIONS {
                        let mut on = !self.closed_sections.iter().any(|c| c == t);
                        if ui.checkbox(&mut on, t).changed() {
                            self.closed_sections.retain(|c| c != t);
                            if !on {
                                self.closed_sections.push(t.to_string());
                            }
                            file::remember_closed_sections(&self.closed_sections);
                        }
                    }
                });
                ui.separator();
                ui.label(RichText::new("Focus on one thing").small().weak());
                if ui.button("Tracks only").clicked() {
                    *v = View {
                        song: false,
                        tracks: true,
                        piano: false,
                        panel: false,
                        ..*v
                    };
                    ui.close_menu();
                }
                if ui.button("Piano roll and sound").clicked() {
                    *v = View {
                        song: false,
                        tracks: false,
                        piano: true,
                        panel: true,
                        ..*v
                    };
                    ui.close_menu();
                }
                if ui.button("Show everything  (F9)").clicked() {
                    *v = View::ALL;
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("How to use (F1)").clicked() {
                    self.show_help = true;
                    ui.close_menu();
                }
                if ui.button("About").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
            ui.separator();
            ui.weak("Space = play · A–K = notes · Z/X = octave · F1 = help");
        });
    }

    /// Lists MIDI inputs: tick to connect, and pick where each one's notes go.
    fn midi_inputs_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_midi_inputs;
        let tracks: Vec<String> = self.project.tracks.iter().map(|t| t.name.clone()).collect();
        let groups: Vec<(u32, String)> = self
            .project
            .groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        let describe = |route: Route| match route {
            Route::Selected => "Selected track".to_string(),
            Route::Track(i) => tracks.get(i).cloned().unwrap_or_default(),
            Route::Group(id) => groups
                .iter()
                .find(|(g, _)| *g == id)
                .map_or_else(String::new, |(_, name)| format!("{name} (group)")),
        };
        egui::Window::new("MIDI inputs")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let midi = &mut self.midi;
                if ui.button("⟳ Rescan").clicked() {
                    midi.refresh();
                }
                if midi.in_ports.is_empty() {
                    ui.weak("No MIDI inputs found. Plug one in and press Rescan.");
                }
                ui.add_space(4.0);
                egui::Grid::new("inputs").num_columns(2).show(ui, |ui| {
                    for name in midi.in_ports.clone() {
                        let route = midi.route(&name);
                        let mut on = route.is_some();
                        if ui.checkbox(&mut on, &name).changed() {
                            midi.set_connected(&name, on);
                        }
                        if let Some(route) = route {
                            let mut picked = route;
                            egui::ComboBox::from_id_salt(&name)
                                .selected_text(describe(route))
                                .width(170.0)
                                .show_ui(ui, |ui| {
                                    let mut option = |ui: &mut egui::Ui, r: Route| {
                                        ui.selectable_value(&mut picked, r, describe(r));
                                    };
                                    option(ui, Route::Selected);
                                    for &(id, _) in &groups {
                                        option(ui, Route::Group(id));
                                    }
                                    for i in 0..tracks.len() {
                                        option(ui, Route::Track(i));
                                    }
                                });
                            if picked != route {
                                midi.set_route(&name, picked);
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.weak(
                    "Selected track follows your clicks. A group sends each drum\n\
                         note to its matching track: the kick pad plays Kick.",
                );
            });
        self.show_midi_inputs = open;
    }

    /// "Save changes first?", shown before anything that would lose them.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(then) = self.confirm.clone() else {
            return;
        };
        #[derive(PartialEq)]
        enum Choice {
            Save,
            Discard,
            Cancel,
        }
        let mut choice = None;
        egui::Window::new("Save changes?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Save the changes to \"{}\" first?",
                    self.song_name()
                ));
                ui.weak("If you don't, they'll be lost.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(Choice::Save);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(Choice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
        match choice {
            Some(Choice::Save) => {
                // If saving is cancelled or fails, stay put and keep asking.
                if self.save() {
                    self.confirm = None;
                    self.carry_out(ctx, then);
                }
            }
            Some(Choice::Discard) => {
                self.confirm = None;
                self.carry_out(ctx, then);
            }
            Some(Choice::Cancel) => self.confirm = None,
            None => {}
        }
    }

    fn transport_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = self.shared.clone();
        ui.horizontal(|ui| {
            for (tab, name, hint) in [
                (Tab::Jam, "Jam", "Make patterns and chain them into blocks"),
                (
                    Tab::Arrange,
                    "Arrange",
                    "Put blocks on the timeline to make a song",
                ),
            ] {
                let text = RichText::new(name).size(15.0).strong();
                if ui
                    .selectable_label(self.tab == tab, text)
                    .on_hover_text(hint)
                    .clicked()
                    && self.tab != tab
                {
                    self.tab = tab;
                    if tab == Tab::Arrange && playing {
                        // Pick up the song from the cursor.
                        s.seek.store(self.timeline.cursor, Relaxed);
                    }
                }
            }
            ui.separator();
            let label = if playing { "■ Stop" } else { "▶ Play" };
            if ui.button(label).clicked() {
                self.set_playing(!playing);
            }
            let mut bpm = s.bpm();
            ui.label("BPM");
            let drag = egui::DragValue::new(&mut bpm)
                .range(40.0..=240.0)
                .speed(0.5);
            if ui.add(drag).changed() {
                s.set_bpm(bpm);
            }
            let mut swing = self.project.swing * 100.0;
            ui.label("Swing");
            let drag = egui::DragValue::new(&mut swing)
                .range(0.0..=100.0)
                .speed(0.5)
                .suffix("%");
            let hint = "Delays every other 16th note for a shuffled feel.\n66% is a triplet swing.";
            if ui.add(drag).on_hover_text(hint).changed() {
                self.project.swing = swing / 100.0;
                self.dirty = true;
            }
            ui.separator();
            if self.tab == Tab::Jam {
                let chain = s.chain_mode.load(Relaxed);
                if ui
                    .selectable_label(!chain, "Pattern")
                    .on_hover_text("Loop the pattern you're editing")
                    .clicked()
                {
                    s.chain_mode.store(false, Relaxed);
                }
                if ui
                    .selectable_label(chain, "Chain")
                    .on_hover_text("Play the block's chain, bar by bar")
                    .clicked()
                {
                    s.chain_mode.store(true, Relaxed);
                }
            } else {
                ui.weak("Plays the timeline");
            }
            ui.separator();
            let rec_color = if self.recording {
                RECORDING
            } else {
                Color32::GRAY
            };
            let rec_label = if s.counting_in.load(Relaxed) {
                "⏺ Count-in"
            } else {
                "⏺ Rec"
            };
            let rec = egui::Button::new(RichText::new(rec_label).color(rec_color))
                .selected(self.recording);
            let rec_hint = "While playing, notes you play on the keyboard\n\
                            are written into the selected track.\n\
                            Play counts in one bar first.";
            if ui.add(rec).on_hover_text(rec_hint).clicked() {
                self.recording = !self.recording;
            }
            let metronome = s.metronome.load(Relaxed);
            if ui
                .selectable_label(metronome, "Click")
                .on_hover_text("Metronome: a click on every beat")
                .clicked()
            {
                s.metronome.store(!metronome, Relaxed);
            }
            if ui
                .selectable_label(self.show_fx, "FX")
                .on_hover_text("Reverb and delay settings")
                .clicked()
            {
                self.show_fx = !self.show_fx;
            }
            ui.separator();
            // Master volume and the output meter.
            ui.label("Master");
            let mut master = self.project.master * 100.0;
            let drag = egui::DragValue::new(&mut master)
                .range(0.0..=100.0)
                .speed(0.5)
                .suffix("%");
            let hint = "Volume of the whole song. A limiter after it keeps the mix from clipping.";
            if ui.add(drag).on_hover_text(hint).changed() {
                self.project.master = master / 100.0;
                self.dirty = true;
            }
            let (peak, limiting) = s.take_meter();
            for (shown, new) in self.meter_levels.iter_mut().zip(peak) {
                *shown = new.max(*shown * 0.85);
            }
            self.meter_limiting = limiting.max(self.meter_limiting * 0.9);
            meter(ui, self.meter_levels, self.meter_limiting);
            if self.meter_levels.iter().any(|l| *l > 0.001) {
                // Keep the meter falling after the sound stops.
                ui.ctx().request_repaint_after(Duration::from_millis(30));
            }
        });
    }

    /// Plays the selected track from the computer keyboard, like a MIDI
    /// keyboard, and records into the pattern when Rec is on.
    /// `rolling`: playing and past any count-in, so Rec writes notes.
    fn computer_keyboard(&mut self, ctx: &egui::Context, rolling: bool) {
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            return;
        };
        let keys: Vec<(egui::Key, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    // Skip shortcuts like Ctrl+G.
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat: false,
                        modifiers,
                        ..
                    } if !(modifiers.command || modifiers.ctrl || modifiers.alt) => {
                        Some((*key, *pressed))
                    }
                    _ => None,
                })
                .collect()
        });
        let base = track.base_note();
        let mut octave_changed = false;
        let mut to_record = Vec::new();
        for (key, pressed) in keys {
            match key {
                egui::Key::Z if pressed => {
                    track.octave = track.octave.saturating_sub(1);
                    octave_changed = true;
                }
                egui::Key::X if pressed => {
                    track.octave = (track.octave + 1).min(7);
                    octave_changed = true;
                }
                _ => {
                    let Some(semitone) = key_semitone(key) else {
                        continue;
                    };
                    if pressed {
                        if let Some((_, t, note)) = self.held_key.take() {
                            let _ = self.events.try_send(Event::Release { track: t, note });
                        }
                        let note = base + semitone;
                        self.held_key = Some((key, self.selected, note));
                        let _ = self.events.try_send(Event::Hold {
                            track: self.selected,
                            note,
                            velocity: NORMAL,
                        });
                        if self.recording && rolling {
                            to_record.push(note);
                        }
                    } else if let Some((_, t, note)) = self.held_key.filter(|&(k, _, _)| k == key) {
                        self.held_key = None;
                        let _ = self.events.try_send(Event::Release { track: t, note });
                    }
                }
            }
        }
        if octave_changed {
            self.dirty = true;
        }
        for note in to_record {
            self.record(self.selected, note, NORMAL);
        }
    }

    fn midi_row(&mut self, ui: &mut egui::Ui) {
        let s = self.shared.clone();
        ui.horizontal(|ui| {
            let m = &mut self.midi;
            if let Some(pick) = port_picker(ui, "MIDI out", &m.out_ports, &m.out_name) {
                m.select_out(pick);
            }
            let inputs = format!("MIDI in: {} connected", m.connected_inputs());
            let hint = "Connect controllers and choose which track each one plays";
            if ui.button(inputs).on_hover_text(hint).clicked() {
                self.show_midi_inputs = !self.show_midi_inputs;
            }
            if ui
                .button("⟳")
                .on_hover_text("Rescan MIDI devices")
                .clicked()
            {
                m.refresh();
            }
            let mut ext = s.external_sync.load(Relaxed);
            if ui.checkbox(&mut ext, "Follow MIDI clock").changed() {
                s.external_sync.store(ext, Relaxed);
            }
        });
    }

    /// The track list: group headers, and tracks unless their group is collapsed.
    fn track_list(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<usize>) {
        for row in self.project.rows() {
            match row {
                Row::Group(g) => self.group_row(ui, g, pattern),
                Row::Track(i) => {
                    let track = &self.project.tracks[i];
                    let group = track.group.and_then(|id| self.project.group(id));
                    if !group.is_some_and(|g| g.collapsed) {
                        self.track_row(ui, i, pattern, current);
                    }
                }
            }
        }
    }

    /// A group header: expand arrow, name, mute, solo, and every hit in the
    /// group combined.
    fn group_row(&mut self, ui: &mut egui::Ui, g: usize, pattern: usize) {
        let id = self.project.groups[g].id;
        let mut lit = [false; STEPS];
        for t in self.project.tracks.iter().filter(|t| t.group == Some(id)) {
            for (step, on) in lit.iter_mut().enumerate() {
                *on |= t.hit_at(pattern, step);
            }
        }
        let mut ungroup = false;
        ui.horizontal(|ui| {
            let group = &mut self.project.groups[g];
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
            let name = egui::Button::new(RichText::new(&group.name).small().strong())
                .selected(self.selected_group == Some(id));
            let resp = ui
                .add_sized([54.0, 26.0], name)
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
            let mute = egui::Button::new(RichText::new("M").small())
                .selected(group.muted)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(mute).on_hover_text("Mute the whole group").clicked() {
                group.muted = !group.muted;
                self.dirty = true;
            }
            let solo_color = if group.solo { SOLO } else { Color32::GRAY };
            let solo = egui::Button::new(RichText::new("S").small().color(solo_color))
                .selected(group.solo)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(solo).on_hover_text("Solo the whole group").clicked() {
                group.solo = !group.solo;
                self.dirty = true;
            }
            // A read-only summary of the group's hits; click to expand.
            for (step, on) in lit.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), Sense::click());
                let fill = if *on {
                    DRUM_ON.gamma_multiply(0.55)
                } else {
                    empty_fill(step, true)
                };
                ui.painter()
                    .rect_filled(rect.shrink2(Vec2::new(0.0, 6.0)), 2.0, fill);
                if resp.clicked() && group.collapsed {
                    group.collapsed = false;
                    self.dirty = true;
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
    fn track_row(&mut self, ui: &mut egui::Ui, i: usize, pattern: usize, current: Option<usize>) {
        let mut action = None;
        ui.horizontal(|ui| {
            let groups: Vec<(u32, String)> = self
                .project
                .groups
                .iter()
                .map(|g| (g.id, g.name.clone()))
                .collect();
            let track = &mut self.project.tracks[i];
            // Tracks in a group are indented under its header.
            let width = if track.group.is_some() {
                ui.add_space(12.0);
                68.0
            } else {
                80.0
            };
            let highlighted =
                (self.selected == i && self.selected_group.is_none()) || self.picked.contains(&i);
            let name = egui::Button::new(RichText::new(&track.name).small()).selected(highlighted);
            let hint = "Click to select and hear it · Ctrl/Cmd-click to pick several · right-click for more";
            let resp = ui.add_sized([width, 26.0], name).on_hover_text(hint);
            if resp.clicked() {
                if ui.input(|i| i.modifiers.command) {
                    if let Some(at) = self.picked.iter().position(|&p| p == i) {
                        self.picked.remove(at);
                    } else {
                        self.picked.push(i);
                    }
                } else {
                    self.selected = i;
                    self.selected_group = None;
                    self.picked.clear();
                    let note = track.note(0);
                    let _ = self.events.try_send(Event::Preview { track: i, note });
                }
            }
            resp.context_menu(|ui| {
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
            let mute = egui::Button::new(RichText::new("M").small())
                .selected(track.muted)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(mute).on_hover_text("Mute").clicked() {
                track.muted = !track.muted;
                self.dirty = true;
            }
            let solo_color = if track.solo { SOLO } else { Color32::GRAY };
            let solo = egui::Button::new(RichText::new("S").small().color(solo_color))
                .selected(track.solo)
                .min_size(Vec2::new(22.0, 26.0));
            let solo_hint = "Solo: hear only soloed tracks";
            if ui.add(solo).on_hover_text(solo_hint).clicked() {
                track.solo = !track.solo;
                self.dirty = true;
            }
            let melodic = track.is_melodic();
            let lanes = track.lanes();
            let on_color = if melodic { NOTE_ON } else { DRUM_ON };
            let base = track.base_note();
            let grid = &mut track.patterns[pattern];
            let shift = ui.input(|i| i.modifiers.shift);
            let pointer = held_pointer(ui);
            // A step is a column across every lane, so walk it by index.
            #[allow(clippy::needless_range_loop)]
            for step in 0..STEPS {
                // Every lane with something at this step, lowest note first.
                let notes: Vec<usize> = (0..lanes).filter(|&l| grid[l][step].is_some()).collect();
                let first = notes.iter().copied().find(|&l| Step::is_hit(&grid[l][step]));
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
                        let name = if melodic { NOTE_NAMES[l] } else { "" };
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
                    None if !notes.is_empty() => NOTE_HELD,
                    None => empty_fill(step, false),
                };
                let pad = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                    .fill(fill)
                    .stroke(playhead(current == Some(step)))
                    .min_size(Vec2::new(28.0, 26.0));
                let mut resp = ui.add(pad);
                if let Some(c) = &chord {
                    resp = resp.on_hover_text(&c.long);
                }
                // Pressing a step turns it on or off; dragging on along the
                // row does the same to every step it passes.
                // (A quick tap can press and release within one frame, so
                // a click starts a paint too.)
                let pressed = resp.is_pointer_button_down_on() || resp.clicked();
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
        });
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

    fn add_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(84.0);
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
            } else {
                ui.weak("or drop audio files onto the window · right-click a step for groove");
            }
        });
    }

    fn piano_roll(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<usize>) {
        let i = self.selected;
        // Notes other melodic tracks play in this pattern, by pitch (C..B),
        // shown faintly so parts can be lined up.
        let mut others = [[false; STEPS]; LANES];
        if self.view.others {
            for (j, t) in self.project.tracks.iter().enumerate() {
                if j == i || !t.is_melodic() {
                    continue;
                }
                for (lane, steps) in t.patterns[pattern].iter().enumerate() {
                    for (step, value) in steps.iter().enumerate() {
                        if Step::is_hit(value) {
                            others[lane][step] = true;
                        }
                    }
                }
            }
        }
        let track = &mut self.project.tracks[i];
        // The chord at the step playing, or else the column hovered last
        // frame, named in full.
        let shown = current.or(self.roll_hover.take());
        let chord = shown.and_then(|step| {
            let grid = &track.patterns[pattern];
            let pitches: Vec<u8> = (0..LANES)
                .filter(|&l| grid[l][step].is_some())
                .map(|l| track.base_note() + l as u8)
                .collect();
            chords::name(&pitches)
        });
        ui.horizontal(|ui| {
            ui.strong(format!("Piano roll · {}", track.name));
            if let Some(c) = &chord {
                ui.label(RichText::new(format!("Chord: {}", c.long)).color(NOTE_ON));
            }
            ui.label("Octave");
            if ui
                .add(egui::DragValue::new(&mut track.octave).range(0..=7))
                .changed()
            {
                self.dirty = true;
            }
            ui.weak("Click: add/remove · drag a note sideways: longer/shorter · several per column = chord · right-click a note: groove");
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let base = track.base_note();
        let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        // Ctrl-drag, or the Erase tool, erases.
        let command = command || self.tool == Tool::Erase;
        let pointer = held_pointer(ui);
        let grid = &mut track.patterns[pattern];
        // Where each step's column is, for working out where a note is
        // being dragged to.
        let mut columns = [0.0_f32; STEPS];
        // Highest note on top, like a piano.
        for lane in (0..LANES).rev() {
            let name = NOTE_NAMES[lane];
            let sharp = name.ends_with('#');
            let note = base + lane as u8;
            ui.horizontal(|ui| {
                let key_fill = if sharp {
                    Color32::from_gray(25)
                } else {
                    Color32::from_gray(90)
                };
                let key = egui::Button::new(name)
                    .fill(key_fill)
                    .min_size(Vec2::new(80.0, 17.0));
                if ui.add(key).clicked() {
                    let _ = self.events.try_send(Event::Preview { track: i, note });
                }
                ui.add_space(52.0); // line up with the steps above
                for step in 0..STEPS {
                    let value = grid[lane][step];
                    let fill = match value {
                        Some(s) if s.tie => NOTE_HELD,
                        Some(s) => lit_fill(NOTE_ON, s.velocity),
                        None if others[lane][step] => OTHER_NOTE,
                        None => empty_fill(step, sharp),
                    };
                    let text = match value {
                        Some(s) if !s.tie => step_marks(&s),
                        _ => String::new(),
                    };
                    let cell = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                        .fill(fill)
                        .stroke(playhead(current == Some(step)))
                        .min_size(Vec2::new(28.0, 17.0))
                        .sense(Sense::click_and_drag());
                    let resp = ui.add(cell);
                    columns[step] = resp.rect.center().x;
                    if resp.hovered() {
                        self.roll_hover = Some(step);
                    }
                    let lane_steps = &mut grid[lane];
                    if value.is_some() && !command && self.paint.is_none() {
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        if resp.drag_started() {
                            // Drag any part of a note to change its length.
                            self.note_drag = note_head(lane_steps, step).map(|head| (lane, head));
                        }
                    }
                    // Pressing an empty cell adds a note, and dragging on
                    // along the row adds more; Ctrl-drag erases instead.
                    let pressed = resp.is_pointer_button_down_on() || resp.clicked();
                    let start =
                        !shift && pressed && self.paint.is_none() && (command || value.is_none());
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
                        && step > 0
                        && lane_steps[step - 1].is_some()
                    {
                        // Hold the note before through this step.
                        lane_steps[step] = Some(Step::TIE);
                        self.dirty = true;
                    }
                }
            });
        }
        if let Some((lane, head)) = self.note_drag {
            let pointer =
                ui.input(|i| i.pointer.primary_down().then_some(i.pointer.interact_pos()));
            match pointer {
                Some(Some(pos)) => {
                    // The column nearest the pointer, but never before the note.
                    let end = (0..STEPS)
                        .min_by(|&a, &b| {
                            (columns[a] - pos.x)
                                .abs()
                                .total_cmp(&(columns[b] - pos.x).abs())
                        })
                        .unwrap_or(head)
                        .max(head);
                    if set_note_length(&mut grid[lane], head, end) {
                        self.dirty = true;
                    }
                }
                Some(None) => {}
                None => self.note_drag = None,
            }
        }
    }

    /// The shared reverb and delay. Tracks send to them with their Reverb
    /// and Delay knobs.
    fn fx_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_fx;
        let fx = &mut self.project.effects;
        let mut changed = false;
        egui::Window::new("Effects")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.weak("Each track's Reverb and Delay knobs send to these.");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Reverb").small().strong());
                        ui.horizontal(|ui| {
                            changed |= knob(ui, &REVERB_SIZE, &mut fx.reverb_size);
                            changed |= knob(ui, &REVERB_TONE, &mut fx.reverb_tone);
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Delay").small().strong());
                        ui.horizontal(|ui| {
                            changed |= knob(ui, &DELAY_TIME, &mut fx.delay_steps);
                            changed |= knob(ui, &DELAY_FEEDBACK, &mut fx.delay_feedback);
                        });
                    });
                });
            });
        self.show_fx = open;
        if changed {
            fx.delay_steps = fx.delay_steps.round();
            self.dirty = true;
        }
    }

    /// Settings for the selected group.
    fn group_panel(&mut self, ui: &mut egui::Ui, id: u32) {
        ui.add_space(4.0);
        let members: Vec<(usize, String)> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.group == Some(id))
            .map(|(i, t)| (i, t.name.clone()))
            .collect();
        let Some(group) = self.project.group_mut(id) else {
            self.selected_group = None;
            return;
        };
        let mut changed = false;
        let name = egui::TextEdit::singleline(&mut group.name)
            .font(egui::TextStyle::Heading)
            .desired_width(200.0);
        changed |= ui.add(name).changed();
        ui.weak("Group");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            changed |= knob(ui, &GROUP_VOLUME, &mut group.volume);
            ui.vertical(|ui| {
                changed |= ui.checkbox(&mut group.muted, "Mute").changed();
                changed |= ui.checkbox(&mut group.solo, "Solo").changed();
                changed |= ui.checkbox(&mut group.collapsed, "Collapsed").changed();
            });
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Tracks").small().strong());
        for (i, name) in members {
            if ui.button(&name).clicked() {
                self.selected = i;
                self.selected_group = None;
            }
        }
        ui.add_space(12.0);
        if ui.button("Ungroup (keep the tracks)").clicked() {
            self.project.ungroup(id);
            self.selected_group = None;
            changed = true;
        }
        if changed {
            self.dirty = true;
        }
    }

    /// Settings for the selected track.
    /// Reads the latest scope samples and draws them at `size`. Returns
    /// true if clicked (to show more or less time).
    fn draw_scope(&mut self, ui: &mut egui::Ui, size: Vec2) -> bool {
        let mut samples = [0.0_f32; SCOPE_LEN];
        self.shared.read_scope(&mut samples);
        self.scope_busy = samples.iter().any(|s| s.abs() > 1e-4);
        let rate = self.shared.sample_rate.load(Relaxed) as f32;
        scope(ui, &samples, rate, SCOPE_SPANS[self.scope_span], size)
    }

    fn is_popped(&self, title: &str) -> bool {
        self.popped.iter().any(|(t, _)| t == title)
    }

    /// The sections popped out of the instrument panel, each in its own
    /// window that can be moved (and for the scope and theremin, resized).
    /// They show the selected track, like the panel. Closing one puts it
    /// back in the panel.
    fn popped_windows(&mut self, ctx: &egui::Context) {
        let name = self
            .project
            .tracks
            .get(self.selected)
            .map(|t| t.name.clone());
        let synth = matches!(
            self.project.tracks.get(self.selected).map(|t| &t.kind),
            Some(Kind::Synth(_))
        );
        let mut close = None;
        for i in 0..self.popped.len() {
            let (title, rect) = self.popped[i].clone();
            if title == "Scope" && !self.show_scope {
                continue;
            }
            let big = matches!(title.as_str(), "Scope" | "Theremin");
            let heading = match &name {
                Some(name) => format!("{title} · {name}"),
                None => title.clone(),
            };
            let mut open = true;
            let mut window = egui::Window::new(heading)
                .id(egui::Id::new(("section window", &title)))
                .open(&mut open)
                .collapsible(true)
                .resizable(big);
            window = match rect {
                // Only the scope and theremin have a size to keep; the
                // others fit their knobs.
                Some([x, y, w, h]) if big => window.default_pos([x, y]).default_size([w, h]),
                Some([x, y, ..]) => window.default_pos([x, y]),
                None if big => window.default_size([480.0, 240.0]),
                None => window,
            };
            if big {
                window = window.min_size([200.0, 80.0]);
            }
            let mut inner = None;
            let shown = window.show(ctx, |ui| {
                inner = Some(ui.available_size());
                if name.is_none() {
                    ui.weak("Add a track to get started.");
                } else if !section_fits(&title, synth) {
                    let kind = if title == "Sample" { "sample" } else { "synth" };
                    ui.weak(format!("Select a {kind} track to see its {title}."));
                    if big {
                        // Keeps its size for when it's back.
                        ui.allocate_space(ui.available_size());
                    }
                } else {
                    self.section(ui, &title, true);
                }
            });
            if !open {
                close = Some(i);
            } else if let (Some(shown), Some(size)) = (shown, inner) {
                // Remember where it is (not while it's folded).
                let at = shown.response.rect.min;
                let now = [at.x, at.y, size.x, size.y];
                let moved =
                    rect.is_none_or(|a| a.iter().zip(now).any(|(p, q)| (p - q).abs() > 0.5));
                if moved {
                    self.popped[i].1 = Some(now);
                    self.popped_changed = true;
                }
            }
        }
        if let Some(i) = close {
            self.popped.remove(i);
            self.popped_changed = true;
        }
        if self.popped_changed && !ctx.input(|i| i.pointer.any_down()) {
            self.popped_changed = false;
            file::remember_popped_sections(&self.popped);
        }
    }

    /// A pad to play the selected synth like a theremin: hold the mouse
    /// down and move it. Left to right slides the pitch smoothly over four
    /// octaves (no steps), bottom to top opens the filter.
    fn theremin_pad(&mut self, ui: &mut egui::Ui, size: Vec2) {
        const LOW: f32 = 36.0; // C2
        const HIGH: f32 = 84.0; // C6
        const NOTE: u8 = 60;
        let (rect, resp) = ui.allocate_exact_size(size, Sense::drag());
        let resp = resp.on_hover_text(
            "Hold the mouse down and move: left to right is pitch (smooth,\n\
             between notes too), bottom to top opens the filter",
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 18, 32));
        // A line at each C, so you can find your way.
        for c in (LOW as u8..=HIGH as u8).step_by(12) {
            let x = rect.left() + (c as f32 - LOW) / (HIGH - LOW) * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0_f32, Color32::from_gray(60)),
            );
            painter.text(
                egui::pos2(x + 2.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                notes::note_name(c),
                egui::FontId::proportional(9.0),
                Color32::from_gray(140),
            );
        }
        let i = self.selected;
        let pointer = resp
            .is_pointer_button_down_on()
            .then(|| resp.interact_pointer_pos())
            .flatten();
        match (pointer, self.theremin) {
            (Some(pos), held) => {
                let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let y = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
                let pitch = LOW + x * (HIGH - LOW);
                if held.is_none() {
                    self.theremin = Some((i, NOTE));
                    let _ = self.events.try_send(Event::Hold {
                        track: i,
                        note: NOTE,
                        velocity: NORMAL,
                    });
                }
                let (track, note) = self.theremin.unwrap_or((i, NOTE));
                let _ = self.events.try_send(Event::Bend {
                    track,
                    note,
                    semis: pitch - note as f32,
                    filter: y * 6.0 - 2.0,
                });
                painter.circle_filled(pos, 6.0, Color32::from_rgb(180, 140, 255));
                let hz = 440.0 * 2f32.powf((pitch - 69.0) / 12.0);
                painter.text(
                    rect.left_top() + Vec2::new(4.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("{hz:.0} Hz"),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
                ui.ctx().request_repaint();
            }
            (None, Some((track, note))) => {
                self.theremin = None;
                let _ = self.events.try_send(Event::Release { track, note });
            }
            (None, None) => {}
        }
    }

    /// The Drone button and its pitch slider: holds a note on the selected
    /// synth nonstop, at any pitch, to shape the sound by ear.
    fn drone_controls(&mut self, ui: &mut egui::Ui) {
        let i = self.selected;
        let on = self.drone.is_some_and(|(t, _)| t == i);
        let mut toggle = on;
        let hint = "Play a note nonstop, so you can turn knobs and hear\n\
                                the sound change. Click again to stop.";
        let flip = ui.toggle_value(&mut toggle, "Drone").on_hover_text(hint);
        // Any pitch, sliding smoothly: the note it's held on
        // is bent the rest of the way.
        let pick = egui::Slider::new(&mut self.drone_hz, 30.0..=2000.0)
            .logarithmic(true)
            .show_value(false);
        let moved = ui
            .add(pick)
            .on_hover_text("The drone's pitch: slide anywhere, between notes too")
            .changed();
        let (base, semis) = free_pitch(self.drone_hz);
        ui.label(
            RichText::new(format!(
                "{:.0} Hz  {}",
                self.drone_hz,
                notes::note_name((base as f32 + semis).round() as u8)
            ))
            .small(),
        );
        if flip.clicked() {
            if let Some((t, note)) = self.drone.take() {
                let _ = self.events.try_send(Event::Release { track: t, note });
            }
            if toggle {
                self.drone = Some((i, base));
                let _ = self.events.try_send(Event::Hold {
                    track: i,
                    note: base,
                    velocity: NORMAL,
                });
            }
        }
        if let Some((t, note)) = self.drone.filter(|_| moved || flip.clicked()) {
            let semis = semis + base as f32 - note as f32;
            let _ = self.events.try_send(Event::Bend {
                track: t,
                note,
                semis,
                filter: 0.0,
            });
        }
    }

    /// One section's knobs for the selected track, in the instrument panel
    /// or in its own window (`big`: the scope and theremin fill it).
    fn section(&mut self, ui: &mut egui::Ui, title: &str, big: bool) {
        let fill = |ui: &egui::Ui, height: f32| {
            if big {
                ui.available_size().max(Vec2::new(200.0, 80.0))
            } else {
                Vec2::new(ui.available_width().min(270.0), height)
            }
        };
        match title {
            "Scope" => {
                if self.draw_scope(ui, fill(ui, 80.0)) {
                    self.scope_span = (self.scope_span + 1) % SCOPE_SPANS.len();
                }
                return;
            }
            "Theremin" => {
                self.theremin_pad(ui, fill(ui, 110.0));
                return;
            }
            _ => {}
        }
        let show_scope = self.show_scope;
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            return;
        };
        let mut changed = false;
        match (title, &mut track.kind) {
            ("Oscillators", Kind::Synth(values)) => {
                ui.horizontal(|ui| {
                    for i in [synth::WAVE, synth::WAVE_2, synth::LEVEL_2, synth::PITCH_2] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
                ui.horizontal(|ui| {
                    for i in [synth::DETUNE, synth::VOICES, synth::GLIDE] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                    changed |= knob(ui, &HUMANIZE, &mut track.humanize);
                });
            }
            ("Filter", Kind::Synth(values)) => {
                ui.horizontal(|ui| {
                    for i in [
                        synth::CUTOFF,
                        synth::RESONANCE,
                        synth::ENV_AMOUNT,
                        synth::FILTER_DECAY,
                    ] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
            }
            ("Envelope (ADSR)", Kind::Synth(values)) => {
                ui.horizontal(|ui| {
                    for i in [synth::ATTACK, synth::DECAY, synth::SUSTAIN, synth::RELEASE] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
                if show_scope {
                    envelope(
                        ui,
                        values[synth::ATTACK],
                        values[synth::DECAY],
                        values[synth::SUSTAIN],
                        values[synth::RELEASE],
                    );
                }
            }
            ("LFO", Kind::Synth(values)) => {
                ui.horizontal(|ui| {
                    for i in [
                        synth::LFO_TARGET,
                        synth::LFO_SHAPE,
                        synth::LFO_AMOUNT,
                        synth::LFO_RATE,
                    ] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
                ui.horizontal(|ui| {
                    let i = synth::LFO_SYNC;
                    changed |= knob(ui, &PARAMS[i], &mut values[i]);
                });
            }
            ("Arpeggiator", Kind::Synth(values)) => {
                ui.horizontal(|ui| {
                    for i in [
                        synth::ARP_MODE,
                        synth::ARP_RATE,
                        synth::ARP_OCTAVES,
                        synth::ARP_GATE,
                    ] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
                ui.horizontal(|ui| {
                    for i in [synth::ARP_NOTES, synth::ARP_LATCH] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                });
            }
            (
                "Sample",
                Kind::Sampler {
                    sample,
                    pitched,
                    shape,
                },
            ) => {
                ui.label(format!("Sample: {}", sample.name));
                ui.weak(format!("{:.2} s, {} Hz", sample.seconds(), sample.rate));
                waveform(ui, &sample.data, shape.start, shape.end, shape.reverse);
                ui.horizontal(|ui| {
                    let hint = "Off: plays as recorded, like a drum.\n\
                                On: plays at the pitch of each note, like an instrument (C4 = as recorded).";
                    changed |= ui.checkbox(pitched, "Pitched").on_hover_text(hint).changed();
                    changed |= ui
                        .checkbox(&mut shape.reverse, "Reverse")
                        .on_hover_text("Play the sample backwards")
                        .changed();
                    changed |= ui
                        .checkbox(&mut shape.ring, "Ring out")
                        .on_hover_text(
                            "On: plays to the end however short the note, like a drum.\n\
                             Off: fades out (Release) when the note ends.",
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    changed |= knob(ui, &SAMPLE_START, &mut shape.start);
                    changed |= knob(ui, &SAMPLE_END, &mut shape.end);
                    changed |= knob(ui, &SAMPLE_ATTACK, &mut shape.attack);
                    ui.add_enabled_ui(!shape.ring, |ui| {
                        changed |= knob(ui, &SAMPLE_RELEASE, &mut shape.release);
                    });
                });
                changed |= knob(ui, &HUMANIZE, &mut track.humanize);
            }
            ("Mix", _) => {
                ui.horizontal(|ui| {
                    changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
                    changed |= knob(ui, &PAN, &mut track.pan);
                    changed |= knob(ui, &REVERB_SEND, &mut track.reverb);
                    changed |= knob(ui, &DELAY_SEND, &mut track.delay);
                });
            }
            _ => {}
        }
        if changed {
            self.dirty = true;
        }
    }

    fn instrument_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            ui.weak("Add a track to get started.");
            return;
        };
        let mut changed = false;
        let name = egui::TextEdit::singleline(&mut track.name)
            .font(egui::TextStyle::Heading)
            .desired_width(200.0);
        changed |= ui.add(name).changed();
        ui.add_space(4.0);
        // Hear the track: a quick note, or a drone that keeps playing.
        let synth = matches!(self.project.tracks[self.selected].kind, Kind::Synth(_));
        ui.horizontal(|ui| {
            if ui.button("Play a note").clicked() {
                self.preview(self.selected, 0);
            }
            if synth {
                self.drone_controls(ui);
            }
        });
        ui.add_space(4.0);
        let mut sections = Sections {
            closed: std::mem::take(&mut self.closed_sections),
            close: None,
            pop: None,
        };
        // A section shows here unless it doesn't fit this kind of track or
        // it's popped out into its own window.
        let here = |app: &Self, t: &str| {
            section_fits(t, synth) && !app.is_popped(t) && (t != "Scope" || app.show_scope)
        };
        if here(self, "Scope") {
            sections.show(ui, "Scope", true, |ui| self.section(ui, "Scope", false));
            ui.add_space(4.0);
        }
        let track = &mut self.project.tracks[self.selected];

        ui.horizontal(|ui| {
            // Shown 1-16 like on hardware, stored 0-15; 0 means off.
            let mut ch = track.midi_channel.map_or(0, |c| c + 1);
            ui.label("MIDI ch");
            let drag = egui::DragValue::new(&mut ch)
                .range(0..=16)
                .custom_formatter(|v, _| {
                    if v == 0.0 {
                        "Off".into()
                    } else {
                        format!("{v}")
                    }
                });
            if ui
                .add(drag)
                .on_hover_text("Send this track's notes to MIDI gear")
                .changed()
            {
                track.midi_channel = (ch > 0).then(|| ch - 1);
                changed = true;
            }
            if !track.is_melodic() {
                ui.label("note");
                let drag = egui::DragValue::new(&mut track.midi_note).range(0..=127);
                changed |= ui
                    .add(drag)
                    .on_hover_text("MIDI note this track sends")
                    .changed();
            }
        });
        ui.add_space(8.0);

        if let Kind::Synth(values) = &mut track.kind {
            ui.weak("Drag knobs up/down. Double-click to reset. Click a heading to fold it.");
            ui.add_space(4.0);
            // A starting sound, adjusted from there with the knobs.
            ui.horizontal(|ui| {
                ui.label("Preset");
                egui::ComboBox::from_id_salt("preset")
                    .selected_text(preset_name(values))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (i, (name, _)) in synth::PRESETS.iter().enumerate() {
                            if ui.selectable_label(false, *name).clicked() {
                                *values = synth::preset(i);
                                changed = true;
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "Start from an instrument sound, then adjust any knob.\n\
                         Ctrl+Z undoes it.",
                    );
            });
            ui.add_space(4.0);
        }
        if changed {
            self.dirty = true;
        }
        for title in SECTIONS.into_iter().skip(1) {
            if here(self, title) {
                let open = !matches!(title, "Arpeggiator" | "Theremin");
                sections.show(ui, title, open, |ui| self.section(ui, title, false));
            }
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            // Closed sections that belong on this kind of track.
            let reopen: Vec<&str> = SECTIONS
                .into_iter()
                .filter(|t| section_fits(t, synth) && !sections.is_open(t))
                .collect();
            if !reopen.is_empty() {
                ui.menu_button("+ Section", |ui| {
                    for t in reopen {
                        if ui.button(t).clicked() {
                            sections.closed.retain(|c| c != t);
                            self.sections_changed = true;
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Bring back a section you closed");
            }
        });
        if let Some(t) = sections.close.take() {
            sections.closed.push(t);
            self.sections_changed = true;
        }
        if let Some(t) = sections.pop.take() {
            self.popped.push((t, None));
            self.popped_changed = true;
        }
        self.closed_sections = sections.closed;
        if std::mem::take(&mut self.sections_changed) {
            file::remember_closed_sections(&self.closed_sections);
        }
        ui.add_space(12.0);
        if ui
            .button(RichText::new("Delete track").color(Color32::LIGHT_RED))
            .clicked()
        {
            self.delete_selected();
        }
    }
}
