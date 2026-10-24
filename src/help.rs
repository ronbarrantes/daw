//! The built-in "How to use" guide (Help menu or F1).

use eframe::egui::{self, RichText};

const SECTIONS: &[(&str, &str)] = &[
    (
        "Getting started",
        "Press Space (or ▶ Play) to start and stop.\n\n\
         Each row is a track. Click its steps to make a beat, click a track's \
         name to select it, and use the panel on the right to shape its sound. \
         Synth tracks also get a piano roll below the rows.",
    ),
    (
        "Tracks",
        "• + Synth adds a synth. + Sample… loads WAV, MP3, FLAC or OGG files, \
         or drop audio files onto the window.\n\
         • M mutes a track. S solos it: when any track is soloed, only soloed \
         tracks play. Use solo to hear one part on its own.\n\
         • Rename a track at the top of the right panel; Delete track is at the \
         bottom.",
    ),
    (
        "Steps and groove",
        "• Click a step to turn it on or off.\n\
         • Shift-click a step for a ghost note (a quiet hit). Quieter steps \
         look dimmer.\n\
         • Right-click a step for all its settings:\n\
           – Velocity: how loud (Ghost, Normal, Accent presets).\n\
           – Chance: how often it plays, for beats that change each time.\n\
           – Ratchet: repeat it 2-4 times within the step, like a hi-hat roll.\n\
           – Nudge: shift it a little early or late.\n\
         • Swing (top bar) delays every other 16th note for a shuffled feel; \
         66% is a triplet swing.\n\
         • Humanize (right panel) adds small random changes to a track's \
         timing and loudness so it sounds less robotic.",
    ),
    (
        "Piano roll",
        "• Click a cell to add a note; click it again to remove it.\n\
         • Right-click the cell after a note to hold it longer (darker blue).\n\
         • Right-click a note itself for its velocity, chance, ratchet and \
         nudge; shift-click for a ghost note.\n\
         • Octave moves the whole roll up or down.",
    ),
    (
        "Play from your keyboard",
        "Your laptop keys play the selected track like a piano:\n\n\
          W E   T Y U   O P    ← black keys\n\
         A S D F G H J K L     ← white keys (A = C)\n\n\
         Z and X change octave. Turn on ⏺ Rec and press Play to write what \
         you play into the pattern.",
    ),
    (
        "Patterns and song",
        "• A-H pick which pattern you're editing. Each holds every track's \
         steps. Copy, Paste and Clear work on the pattern you're editing.\n\
         • Pattern mode loops the pattern you're editing.\n\
         • Song mode plays the Song row left to right, one bar per box. Click \
         a box to place the selected pattern; click it again or right-click \
         to empty it.",
    ),
    (
        "Synth",
        "Drag a knob up or down; double-click to reset it; hover to see what \
         it does.\n\n\
         • Sound: Wave (saw → square), Detune (thicker), Volume, Humanize.\n\
         • Filter: Cutoff and Reso set the tone; Env amt and F decay make \
         each note pluck.\n\
         • Envelope (ADSR): Attack fades in, Decay falls to the Sustain level \
         while a note is held, Release fades out after.\n\n\
         Try: Cutoff down, Reso up, Env amt up, short F decay → acid bass.",
    ),
    (
        "Sample tracks",
        "• Pitched off: plays as recorded, like a drum hit.\n\
         • Pitched on: plays higher or lower with each note, like an \
         instrument. C at octave 4 is the original pitch.",
    ),
    (
        "MIDI gear",
        "Plug gear in over USB, then press ⟳ to rescan.\n\n\
         • MIDI out sends clock and Start/Stop, so gear follows the tempo.\n\
         • Each track's MIDI ch (right panel) sends its notes, with velocity, \
         on that channel. The built-in drums use channel 10.\n\
         • MIDI in: a note plays the one-shot track that sends the same note.\n\
         • Follow MIDI clock lets another device set the tempo and \
         start/stop.",
    ),
    (
        "Shortcuts",
        "Space — play / stop\n\
         A–P — play notes on the selected track\n\
         Z / X — octave down / up\n\
         F1 — this guide\n\
         Shift-click a step — ghost note\n\
         Right-click a step — step settings\n\
         Double-click a knob — reset it",
    ),
];

pub fn guide(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("How to use")
        .open(open)
        .default_width(460.0)
        .default_height(480.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, (title, body)) in SECTIONS.iter().enumerate() {
                    egui::CollapsingHeader::new(RichText::new(*title).strong())
                        .default_open(i == 0)
                        .show(ui, |ui| ui.label(*body));
                }
            });
        });
}

pub fn about(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("About")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("daw");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("A tiny, lightweight DAW written in Rust.");
            ui.hyperlink("https://github.com/ronbarrantes/daw");
        });
}
