//! The built-in "How to use" guide (Help menu or F1).

use eframe::egui::{self, RichText};

const SECTIONS: &[(&str, &str)] = &[
    (
        "Getting started",
        "Press Space (or ▶ Play) to start and stop. Ctrl+Z (Cmd+Z on Mac) \
         undoes and Ctrl+Shift+Z redoes, also in the Edit menu.\n\n\
         Each row is a track. Click its steps to make a beat, click a track's \
         name to select it, and use the panel on the right to shape its sound. \
         Synth tracks also get a piano roll below the rows.",
    ),
    (
        "Saving songs",
        "• File > Save (Ctrl+S) saves your song as a .daw file. Save as… \
         (Ctrl+Shift+S) picks a new name; Open… (Ctrl+O) opens one.\n\
         • Samples you loaded are saved inside the file, so it works on any \
         computer.\n\
         • The song you had open reopens next time you start the app.\n\
         • A dot in the window title means unsaved changes. daw asks before \
         anything would lose them.\n\
         • File > Export WAV… saves the sound to share or post. Export MIDI… \
         saves the notes for other DAWs or hardware synths. Both export the \
         timeline, or the current pattern four times if it's empty.",
    ),
    (
        "Tracks",
        "• + Synth adds a synth. + Sample… loads WAV, MP3, FLAC or OGG files, \
         or drop audio files onto the window.\n\
         • M mutes a track. S solos it: when any track is soloed, only soloed \
         tracks play. Use solo to hear one part on its own.\n\
         • Mix (right panel): Volume, Pan (left or right; double-click for \
         centre), and Reverb and Delay: how much of the track goes to the \
         shared effects.\n\
         • FX (top bar) sets up the effects: reverb Size and Tone, delay \
         Time (in 16th notes; 3/16 is a classic) and Feedback (more echoes).\n\
         • Rename a track at the top of the right panel; Delete track is at the \
         bottom.",
    ),
    (
        "Groups",
        "Groups are folders of tracks, like layer groups in Photoshop.\n\n\
         • Ctrl-click (Cmd on Mac) track names to pick several, then press \
         Ctrl+G to group them. Or right-click a track name.\n\
         • ▶/▼ collapses or expands a group. Collapsed, its row shows all its \
         hits combined.\n\
         • The group's M and S mute or solo all its tracks; click its name for \
         a group volume.\n\
         • Right-click a group's name to ungroup (the tracks stay).",
    ),
    (
        "Showing and hiding",
        "Hiding a part never deletes it or stops it playing.\n\n\
         • View menu, or F5 song and patterns, F6 tracks, F7 piano roll, \
         F8 instrument panel.\n\
         • View > Focus: Tracks only, or Piano roll and sound.\n\
         • F9 shows everything again.",
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
        "• Click a cell to add a note; click it again to remove it. Several \
         notes in one column make a chord.\n\
         • Right-click the cell after a note to hold it longer (darker blue).\n\
         • Right-click a note itself for its velocity, chance, ratchet and \
         nudge; shift-click for a ghost note.\n\
         • Octave moves the whole roll up or down.\n\
         • Other tracks' notes show faintly behind, so you can line parts up \
         (turn off in the View menu).",
    ),
    (
        "Play from your keyboard",
        "Your laptop keys play the selected track like a piano:\n\n\
          W E   T Y U   O P    <- black keys\n\
         A S D F G H J K L     <- white keys (A = C)\n\n\
         Z and X change octave. Turn on ⏺ Rec and press Play to write what \
         you play into the pattern. With Rec on, Play clicks one bar first \
         (the count-in) so you can come in on time.\n\n\
         Click (top bar) turns the metronome on: a click on every beat, \
         higher on beat 1.",
    ),
    (
        "Patterns",
        "The Patterns row is your library. Click one to edit it in the grid \
         below; each holds every track's steps.\n\n\
         • + adds a pattern. Right-click one to rename, duplicate or delete \
         it.\n\
         • Copy, Paste and Clear work on the pattern you're editing.\n\
         • Pattern mode (top bar) loops the pattern you're editing, for \
         jamming.",
    ),
    (
        "Timeline",
        "The timeline is where a song comes together. Song mode (top bar) \
         plays it.\n\n\
         • Drag a pattern from the library onto it, or double-click it to \
         place the selected pattern.\n\
         • Drag a clip to move it; drag its edges to trim or stretch it (a \
         long clip repeats its pattern). Alt-drag makes a copy.\n\
         • Clips are linked to their pattern: edit the pattern and every \
         clip of it changes. Right-click a clip and choose Make unique to \
         give it its own copy.\n\
         • Double-click a clip to edit its pattern.\n\
         • Click the ruler to set where playback starts; drag along it to \
         mark a loop, and turn Loop on or off.\n\
         • C splits the selected clip at the cursor, Ctrl+D duplicates it, \
         Delete removes it.\n\
         • Snap sets what clips line up to; hold Shift for single steps. \
         Ctrl+scroll or - and + zoom.",
    ),
    (
        "Synth",
        "Drag a knob up or down; double-click to reset it; hover to see what \
         it does.\n\n\
         • Sound: Wave (saw to square), Detune (thicker), Humanize.\n\
         • Filter: Cutoff and Reso set the tone; Env amt and F decay make \
         each note pluck.\n\
         • Envelope (ADSR): Attack fades in, Decay falls to the Sustain level \
         while a note is held, Release fades out after.\n\n\
         Try: Cutoff down, Reso up, Env amt up, short F decay = acid bass.",
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
         • MIDI in: click it to connect controllers. Each one plays either \
         the selected track (follows your clicks) or a fixed track or group. \
         A drum kit routed to the Drums group plays the matching drum for \
         each pad. Change it any time.\n\
         • With ⏺ Rec on and playing, what you play on a controller is \
         written into the pattern, chords included.\n\
         • MIDI out sends clock and Start/Stop, so gear follows the tempo.\n\
         • Each track's MIDI ch (right panel) sends its notes, with velocity, \
         on that channel. The built-in drums use channel 10.\n\
         • Follow MIDI clock lets another device set the tempo and \
         start/stop.",
    ),
    (
        "In a browser",
        "The web version works like the desktop app, except:\n\n\
         • Sound starts on your first click or key press.\n\
         • Save downloads the .daw file; Open… uploads one. Exports download \
         too.\n\
         • Your song is kept in the browser, so it's back next visit.\n\
         • MIDI gear works in Chrome and Edge only.\n\
         • If nothing responds to clicks, a browser extension may be in the \
         way; try a private window.\n\
         • Ctrl+N opens a browser window; use File > New song.",
    ),
    (
        "Shortcuts",
        "Space — play / stop\n\
         Ctrl+S / Ctrl+Shift+S — save / save as (Cmd on Mac)\n\
         Ctrl+O / Ctrl+N — open / new song\n\
         Ctrl+Z / Ctrl+Shift+Z — undo / redo (Ctrl+Y redoes too)\n\
         Ctrl+G — group the picked tracks\n\
         F5 / F6 / F7 / F8 — show or hide timeline, tracks, piano roll, panel\n\
         C / Ctrl+D / Delete — split, duplicate, delete the selected clip\n\
         F9 — show everything\n\
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
