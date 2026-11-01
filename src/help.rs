//! The built-in "How to use" guide (Help menu or F1).

use eframe::egui::{self, RichText};

const SECTIONS: &[(&str, &str)] = &[
    (
        "The screen",
        "From top to bottom:\n\n\
         • The top bar is for playing: the menus, Jam and Arrange, ▶ and ⏺, \
         a display with where it's playing, the tempo and the swing, Click, \
         Master and the meter.\n\
         • The tracks are for editing. Their top line has the patterns as \
         tabs (A, B...), the pattern's Length and what Plays (pattern or \
         chain). Each row is a color dot and name, its volume, M and S, its \
         steps (two bars side by side) and a meter right after them. A synth's piano roll \
         opens under its row when you click its name.\n\
         • The instrument drawer: the selected track's sound. Shut, it's one \
         line that sketches the sound; open (I or F8), its sections sit side by \
         side.\n\
         • The status line: errors, and how hard the audio is working.",
    ),
    (
        "Getting started",
        "Press Space (or ▶ in the top bar) to start and stop. Master (the % \
         next to the meter) sets the whole song's volume; the meter shows the level, and LIMIT lights when \
         the limiter is stopping it from clipping (turn things down a bit). \
         Ctrl+Z (Cmd+Z on Mac) \
         undoes and Ctrl+Shift+Z redoes, also in the Edit menu.\n\n\
         Each row is a track. Click its steps to make a beat, click a track's \
         name to select it, and use the instrument drawer along the bottom to shape its sound. \
         A synth track's piano roll opens right under its row.",
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
         timeline; if it's empty, the current block's chain, or else the \
         current pattern four times.",
    ),
    (
        "Tracks",
        "• + Synth adds a synth. + Drum adds a built-in drum (clave, bongos, \
         congas, timbales, guiro, cowbell and more). + Sample… loads WAV, MP3, \
         FLAC or OGG files, or drop audio files onto the window.\n\
         • Right-click a track's name to duplicate it, move it up or down, \
         or delete it.\n\
         • M mutes a track. S solos it: when any track is soloed, only soloed \
         tracks play. Use solo to hear one part on its own.\n\
         • Each row has the track's volume (the small slider by its name; \
         double-click to reset) and a meter after its steps. Mix (over the \
         meters) adds Pan, Reverb and Delay sliders to every row.\n\
         • Mix (instrument drawer): Volume, Pan (left or right; double-click for \
         centre), and Reverb and Delay: how much of the track goes to the \
         shared effects.\n\
         • View > Effects sets up the effects: reverb Size and Tone, delay \
         Time (in 16th notes; 3/16 is a classic) and Feedback (more echoes).\n\
         • Rename a track at the start of the instrument drawer's top line; \
         More, at its end, has the MIDI channel, closed sections and Delete track.",
    ),
    (
        "Groups",
        "Groups are folders of tracks, like layer groups in Photoshop.\n\n\
         • Ctrl-click (Cmd on Mac) track names to pick several, then press \
         Ctrl+G to group them. Or right-click a track name.\n\
         • ▶/▼ collapses or expands a group. Collapsed, its row shows all its \
         hits combined.\n\
         • The group's M and S mute or solo all its tracks; the slider by its \
         name is the group's volume.\n\
         • Right-click a group's name to ungroup (the tracks stay).",
    ),
    (
        "Showing and hiding",
        "Hiding a part never deletes it or stops it playing.\n\n\
         • Every part has a small triangle at the start of its title line. \
         Pointing down, the part is open: click the line to hide it. Pointing \
         right, it's folded to that one line: click to show it again. Tracks, Piano roll, Chain and Instrument \
         all work this way, and so do the drawer's sections.\n\
         • Clicking a synth track's name always opens its piano roll.\n\
         • Shortcuts do the same: F5 chain, F6 tracks, F7 piano roll, F8 (or \
         I) instrument. The View menu has them too.\n\
         • View > Focus: Tracks only, or Piano roll and sound.\n\
         • F9 shows everything again.\n\
         • The instrument is a drawer along the bottom: its arrow (or I, or F8) \
         shuts it to one line saying what's selected, and opens it again. Drag its \
         top edge to make it taller or shorter.\n\
         • The line along the very bottom shows errors and how hard the audio \
         is working; click the % for Help > Diagnostics.\n\
         • View > Theme changes the colors: the computer's light or dark, \
         LED (white on black) or Hardware (black and orange).\n\
         • On Linux, touchpad scrolling glides on after you lift your \
         fingers, like in a browser; a click stops it.\n\
         • View > Low power redraws less often while playing, to save \
         battery.",
    ),
    (
        "Steps and groove",
        "• The Tool (over the track names) picks what clicks and drags do: \
         Draw adds notes and turns steps on, Erase removes them (MIDI blocks \
         also have Select). Handy on a touch screen.\n\
         • Click a step to turn it on or off. Drag along a row to paint \
         steps on (start on an empty step) or erase them (start on a lit one). \
         Hover a step to see its note, chord or groove marks.\n\
         • Shift-click a step for a ghost note (a quiet hit). Quieter steps \
         look dimmer.\n\
         • Right-click a step for all its settings:\n\
           – Velocity: how loud (Ghost, Normal, Accent presets).\n\
           – Chance: how often it plays, for beats that change each time.\n\
           – Ratchet: repeat it 2-4 times within the step, like a hi-hat roll.\n\
           – Nudge: shift it a little early or late.\n\
         • Swing (top bar) delays every other 16th note for a shuffled feel; \
         66% is a triplet swing.\n\
         • Humanize (instrument drawer) adds small random changes to a track's \
         timing and loudness so it sounds less robotic.",
    ),
    (
        "Piano roll",
        "• Click a cell to add a note; click it again to remove it. Several \
         notes in one column make a chord; hover the step to see its name (Cm, \
         G7, C/E), and the piano roll's title shows it in full.\n\
         • It shows the bar you last touched, lined up under that bar's steps.\n\
         • Drag from an empty cell along a row to paint notes; Ctrl-drag \
         erases them.\n\
         • Drag a note to move it (up/down: pitch, left/right: time); drag \
         its right edge to make it longer or shorter (the held part is \
         a darker shade of the track's color). Both work while it plays. Right-click the cell after \
         a note to hold it a step longer; on a bar's first step that holds \
         a note over from the bar before.\n\
         • Right-click a note itself for its velocity, chance, ratchet and \
         nudge; shift-click for a ghost note.\n\
         • Octave moves the whole roll up or down; Octaves (1-4) is how \
         many it shows. Recorded notes keep their octave if it's shown.\n\
         • Other tracks' notes show faintly behind, so you can line parts up \
         (turn off in the View menu).\n\
         • Each track plays up to 16 notes at once; past that, the oldest \
         note gives way.",
    ),
    (
        "Play from your keyboard",
        "Your laptop keys play the selected track like a piano:\n\n\
          W E   T Y U   O P    <- black keys\n\
         A S D F G H J K L     <- white keys (A = C)\n\n\
         Hold several keys for a chord or the arpeggiator. Z and X change \
         octave. With MIDI > Bend with keys 1 and 2 on, 1 and 2 are a pitch wheel: \
         hold 1 to bend the held notes down, 2 up, and let go to slide \
         back; the box under it sets how far. Turn on ⏺ Rec and press Play to write what \
         you play into the pattern. With Rec on, Play clicks one bar first \
         (the count-in) so you can come in on time.\n\n\
         Click (top bar) turns the metronome on: a click on every beat, \
         higher on beat 1.",
    ),
    (
        "Jam and Arrange",
        "Two tabs at the top left. Jam is the groovebox: patterns, chains \
         and sounds. Arrange is the timeline, where blocks become a song.\n\n\
         The units: a pattern is 1 to 8 bars of steps; a chain plays patterns \
         bar by bar; a block is a chain you can place on the timeline (like \
         a clip in other DAWs); a song is blocks on the timeline. A MIDI \
         block is the other kind of block: free notes for one track, made \
         in Arrange.",
    ),
    (
        "Patterns and chains (Jam)",
        "• The patterns are tabs on the tracks' top line (A, B...). Click one to edit it in \
         the grid below; + adds one; right-click to rename, duplicate or \
         delete.\n\
         • Edit > Copy, Paste and Clear pattern work on the pattern you're editing.\n\
         • Longer patterns: right-click a track's name for its Length in \
         this pattern (1, 2, 4 or 8 bars). Shorter parts repeat. Length (tracks' \
         top line) sets every track at once. Two bars show side by side; 1-2, \
         3-4... over the tracks pick which two.\n\
         • The Chain row (it shows in Chain mode) is the block you're editing, one square per bar. \
         Click a square to put the selected pattern there (a longer one \
         fills a square per bar); click again or right-click to empty it.\n\
         • Plays: Pattern loops the pattern; Plays: Chain plays the chain \
         (both on the tracks' top line).\n\
         • Pick a block from the drop-down; + starts a new one; Save as new \
         block copies this chain into a new block.",
    ),
    (
        "Timeline (Arrange)",
        "Arrange plays the timeline, from the cursor.\n\n\
         • Drag a block from the Blocks drawer onto the timeline, or \
         double-click the timeline to place the current block.\n\
         • Drag a block to move it; drag its edges to trim or stretch it (it \
         repeats). Alt-drag makes a copy.\n\
         • Copies are linked: edit the block in Jam and they all change. \
         Right-click one and choose Make unique to give it its own block.\n\
         • Double-click a block to edit it in Jam (a MIDI block opens its \
         notes below with one click).\n\
         • Click the ruler to set where playback starts; drag along it to \
         mark a loop, and turn Loop on or off.\n\
         • C splits the selected block at the cursor, Ctrl+D duplicates it, \
         Delete removes it.\n\
         • Snap sets what blocks line up to; hold Shift for single steps. \
         Ctrl+scroll or - and + zoom.",
    ),
    (
        "Automation (Arrange)",
        "Draws a knob's changes over the song, like a filter opening or a \
         fade out.\n\n\
         • + Automation (timeline bar): pick a track, then a knob. A lane \
         opens under the timeline.\n\
         • Click to add a point, drag one to move it, right-click a point to \
         remove it. Right-click empty space to clear or delete the lane.\n\
         • It plays while the timeline plays; stopped, the knobs are as \
         you set them.",
    ),
    (
        "MIDI blocks (Arrange)",
        "Free notes for one track: any pitch, any length, starting anywhere.\n\n\
         • + MIDI in the Blocks drawer makes one for the selected track. \
         Click a MIDI block to open its notes below the timeline.\n\
         • Plays picks the track; Bars sets the length (drawing past the end \
         makes it longer).\n\
         • Click to add a note, drag it to move, drag its end to make it \
         longer or shorter, right-click to delete.\n\
         • Ctrl-drag draws notes along the way (a pencil); right-drag erases \
         every note it passes.\n\
         • Drag empty space to select; Shift-click adds or removes one; \
         Alt-drag copies. Ctrl+A selects all, Delete deletes, Ctrl+D \
         duplicates, arrows move (Shift: octave or bar), Esc deselects.\n\
         • Snap: 1/4 to 1/32, 1/8 and 1/16 triplets, or Off. Hold Shift \
         while dragging to go off the grid.\n\
         • Length sets how long new notes are (1/16 to 4 bars) and changes \
         the selected notes to it; easier than dragging on a touch screen.\n\
         • Velocity sets how loud the selected notes (or new ones) are.\n\
         • Scroll to move around; Ctrl+scroll or - and + zoom.",
    ),
    (
        "Synth",
        "Drag a knob up or down; double-click to reset it; hover to see what \
         it does. Click anywhere on a section's title line to fold it (and \
         again to open it); its triangle shows which. Right-click the name to close it (More > + Section, or View \
         > Instrument sections, brings it back) or pop it out \
         into its own window to put anywhere (the Scope and Theremin can be \
         made big); its × puts it back.\n\n\
         • Preset: a starting sound, in groups (keys, plucks, mallets and \
         bells, bass, strings and pads, leads, effects). Most bring \
         modulation too, like Velocity for brighter notes when you play \
         harder. Then adjust any knob.\n\
         • Oscillators: Wave 1 and Wave 2 go from sine (smooth) through \
         triangle, saw (buzzy), square (hollow) and noise (hiss) to click \
         (a tick per cycle). Level 2 \
         is the second one's volume, Pitch 2 its pitch in semitones, Detune \
         thickens.\n\
         • Voice: Engine picks the sound: Analog (two oscillators), Pluck \
         (a plucked string), Mallet (marimba, vibes, glass, bell), FM \
         (electric piano, bells, basses: Ratio sets the character, Depth the \
         brightness) or Wavetable (sweep Position, or drag LFO 2 onto it). \
         Try the (pluck), (mallet), FM and wavetable presets. Voices is Poly (chords), Mono (one note at a time) or Legato (one \
         at a time, overlapping notes don't restart). Glide slides between \
         notes in Mono and Legato. Bend slides every note's pitch by that \
         many semitones (fractions too) over Bend time; drag a chip onto \
         Bend to vary it.\n\
         • Arpeggiator: plays held notes one at a time in a pattern (Up, \
         Down, Up/Down, Random, As played) at a rate tied to the tempo, over \
         1-4 octaves. Hold a chord on the keys, or draw long notes. Arp notes \
         turns one held note into a chord or scale (Major, Minor, 7th, \
         Pentatonic...); Latch keeps it going after you let go.\n\
         • Drone and Tap are on the drawer's top line. Drone plays a note \
         nonstop so you can turn knobs and listen; Tap plays one short \
         note to hear the attack and release. The slider sets the pitch: \
         Notes goes note by note (C0 to C8), Free anywhere from 0.1 to \
         20,000 Hz; - and + step a semitone. With Wave 1 on Click and Free, \
         slow ticks turn into a tone.\n\
         • Theremin (the drawer's last section): hold the mouse down on the pad \
         and move. Left to right slides the pitch smoothly, bottom to top \
         opens the filter.\n\
         • Filter: Cutoff and Reso set the tone; Env amt and F decay make \
         each note pluck.\n\
         • Envelope (ADSR): Attack fades in, Decay falls to the Sustain level \
         while a note is held, Release fades out after.\n\
         • LFO: a wobble on Pitch (vibrato), Filter (wah), Volume \
         (tremolo) or Pan (left and right). Random jumps each cycle (random tones on pitch). LFO sync \
         ties its speed to the tempo. LFO retrig restarts it at every note.\n\
         • Mutate nudges every knob a little for a new variation (right-click \
         it to leave sections alone). Keep A and Keep B remember two sounds; \
         the slider between them morphs from one to the other.\n\
         • Modulation: drag a coloured chip (LFO 1, LFO 2 and Mod env in \
         their sections, Velocity in Voice) onto any knob to make it move \
         that knob; a ring shows how far. Velocity is how hard you play. Right-click the knob to change how far or remove it.\n\n\
         Try: Cutoff down, Reso up, Env amt up, short F decay = acid bass. \
         Noise with a short decay and high cutoff = a shaker.\n\n\
         The green scope (the Scope section) shows the selected track's \
         wave as you play; click it to show more or less time, or switch it \
         to Spectrum (loudness per pitch). Drawings above the knobs show \
         the envelope, the filter's shape and each LFO's wave. View menu \
         turns them off.",
    ),
    (
        "Sample tracks",
        "• Browse drum sounds searches built-in drums and loaded song samples. Click to swap, then Tap to hear it. Notes and routing stay; trim/fades reset. Ctrl+Z undoes. Add files with + Sample.\n\
         • Pitched off: plays as recorded, like a drum hit.\n\
         • Pitched on: plays higher or lower with each note, like an \
         instrument. C at octave 4 is the original pitch.\n\
         • Start and End trim the sample (see the waveform), Fade in softens \
         it, Fade out makes the end die away (to shorten a hit without a hard \
         cut), Reverse plays it backwards.\n\
         • Ring out plays to the end; off, it fades out over Release when \
         the note ends.",
    ),
    (
        "MIDI gear",
        "Plug gear in over USB. Everything here is in the MIDI menu; \
         ⟳ rescans.\n\n\
         • MIDI in: click it to connect controllers. Each one plays either \
         the selected track (follows your clicks) or a fixed track or group. \
         A drum kit routed to the Drums group plays the matching drum for \
         each pad. Change it any time.\n\
         • With ⏺ Rec on and playing, what you play on a controller is \
         written into the pattern, chords included.\n\
         • A keyboard's sustain pedal holds notes while it's down, like a \
         piano.\n\
         • MIDI out sends clock and Start/Stop, so gear follows the tempo.\n\
         • Each track's MIDI ch (instrument drawer) sends its notes, with velocity, \
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
        "Diagnostics",
        "Help > Diagnostics shows how the sound engine is doing and a log of \
         recent errors.\n\n\
         • Audio load: how much of its time the engine needs. Near 100% the \
         sound crackles; try Low power, fewer tracks or the desktop app.\n\
         • Dropouts: how many times it ran late.\n\
         • Copy all puts it all on the clipboard, for a bug report.",
    ),
    (
        "Shortcuts",
        "Space — play / stop\n\
         Ctrl+S / Ctrl+Shift+S — save / save as (Cmd on Mac)\n\
         Ctrl+O / Ctrl+N — open / new song\n\
         Ctrl+Z / Ctrl+Shift+Z — undo / redo (Ctrl+Y redoes too)\n\
         Ctrl+G — group the picked tracks\n\
         F5 / F6 / F7 / F8 — show or hide chain, tracks, piano roll, instrument\n\
         C / Ctrl+D / Delete — split, duplicate, delete the selected block \
         (Arrange)\n\
         Ctrl+A / Ctrl+D / Delete / arrows — select all, duplicate, delete, \
         move notes (MIDI block)\n\
         I — open or shut the instrument drawer\n\
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
