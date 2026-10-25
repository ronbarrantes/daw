# daw

A tiny, lightweight DAW in Rust: synth and sample tracks, 8 patterns you can
arrange into a song, and MIDI for hardware like the M-VAVE FM-1 and Roland
AIRA Compacts.

**Try it in your browser: [ronbarrantes.github.io/daw](https://ronbarrantes.github.io/daw/)**,
or install it below for the best performance.

## Install on Mac (Apple Silicon)

1. Download **[daw-macos-arm64.app.zip](https://github.com/ronbarrantes/daw/releases/latest/download/daw-macos-arm64.app.zip)**
   and open it to unzip (Safari may do this for you). You get **daw.app**.
2. Drag **daw.app** into **Applications**.
3. Open it. The first time, macOS says it can't check the developer, because
   the app isn't signed with a paid Apple developer account. To allow it:
   - **macOS 15 and later:** click **Done**, then open **System Settings →
     Privacy & Security**, scroll down to the message about daw and click
     **Open Anyway**.
   - **macOS 14 and earlier:** Control-click **daw.app**, choose **Open**,
     then **Open** again.
   - **Or, in Terminal:** `xattr -dr com.apple.quarantine /Applications/daw.app`

   You only need to do this once per update.

## Install on Linux (Arch, Hyprland and others)

```sh
curl -L https://github.com/ronbarrantes/daw/releases/latest/download/daw-linux-x86_64.tar.gz | tar xz
./daw/install.sh
```

This puts `daw` in `~/.local/bin` and adds an icon and a menu entry, so it
shows up in app launchers (wofi, rofi, fuzzel and so on). No sudo needed;
you only need `alsa-lib`, which any desktop already has.

- **Update:** run the same two lines again.
- **Uninstall:** `./daw/install.sh --uninstall`
- **Hyprland:** the window class is `daw`, e.g.
  `windowrulev2 = float, class:^(daw)$`

## Use it in a browser

**[ronbarrantes.github.io/daw](https://ronbarrantes.github.io/daw/)** runs
the same app in Chrome, Edge, Firefox or Safari, nothing to install. It's
updated with every merge to `main`.

It works like the desktop app, with a few differences:

- **Sound starts on your first click or key press** (browsers require it).
- **Save** (Ctrl+S) downloads the `.daw` file; **Open…** uploads one. The
  same files open in the desktop app and vice versa.
- **Export WAV / MIDI** download the file.
- Your song is **kept in the browser** and is back when you return, even
  without saving. Songs with long samples may be too big for that (about
  5 MB); then use Save.
- **MIDI gear** works in Chrome and Edge (they'll ask for permission).
  Firefox and Safari don't support MIDI.
- Ctrl+N opens a new browser window instead; use **File → New song**.
- On the 2012 MacBook Air, the desktop app runs much better.

To build it yourself: `rustup target add wasm32-unknown-unknown`,
`cargo install wasm-bindgen-cli` (the version in `Cargo.lock`), then
`./web/build.sh` and serve `web/dist`, e.g. `python3 -m http.server -d web/dist`.

## Other downloads

The links above get the newest numbered version. Every merge to `main` also
updates a **latest** build with the newest changes; all builds are on the
[Releases](https://github.com/ronbarrantes/daw/releases) page.

## Build it yourself

### Setup (once)

**Arch Linux**

```sh
sudo pacman -S --needed rustup base-devel alsa-lib
rustup default stable
```

**macOS**

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then open a new terminal.

### Run

```sh
git clone https://github.com/ronbarrantes/daw.git
cd daw
cargo run --release
```

The first build takes a few minutes. After that it's fast.

## Use

Press **F1** (or **Help → How to use**) for the built-in guide.

- **Space** or **▶ Play** starts and stops. Drag **BPM** to change the tempo.
- **Ctrl+Z** (Cmd+Z on Mac) undoes; **Ctrl+Shift+Z** or **Ctrl+Y** redoes.
  Also in the **Edit** menu. A whole knob drag is one step.

### Saving songs

- **File → Save** (Ctrl+S, Cmd+S on Mac) saves a `.daw` file. **Save as…**
  (Ctrl+Shift+S) picks a new name; **Open…** (Ctrl+O) opens one.
- Samples you loaded are saved inside the file, so songs work on any computer.
- The song you had open reopens next time you start the app.
- A dot in the window title means unsaved changes; daw asks before anything
  would lose them.

A `.daw` file is a zip (like `.docx`) holding `song.json`, which is readable
and editable by hand, plus `samples/*.wav` for any loaded samples.

### Exporting

- **File → Export WAV…** saves the sound, ready to share or post.
- **File → Export MIDI…** saves the notes as a standard MIDI file for other
  DAWs or hardware synths. Each MIDI channel becomes its own track; drums
  go on channel 10, and synths without a channel get a free one.
- Both export the **Song** row, or the current pattern four times if the row
  is empty. Muted tracks are left out.

### Tracks

Each row is a track: its name, **M** (mute), **S** (solo) and 16 steps.
Tracks work like layers: mute turns one off, and solo plays only the
soloed tracks so you can hear a part on its own.

- Click a step to turn it on or off.
- Click a track's name to select it and hear it. The right panel shows its
  settings; melodic tracks also get a **piano roll** below the rows.
- **+ Synth** adds a synth track. **+ Sample…** loads WAV, MP3, FLAC or OGG
  files as sample tracks, or just drop files onto the window.
- Rename a track by editing its name at the top of the right panel.
- **Mix** (right panel): **Volume**, **Pan** (left or right in the stereo
  mix; double-click to centre), and **Reverb** and **Delay**: how much of
  the track goes to the shared effects.

### Effects

Every track can send to one shared reverb and one delay. Turn up a
track's **Reverb** or **Delay** knob, then press **FX** in the top bar to
shape them:

- **Reverb:** **Size** (how long it rings) and **Tone** (muffled to airy).
- **Delay:** **Time** between echoes in 16th notes (3/16, a dotted eighth,
  is a classic) and **Feedback** (how many echoes). Echoes bounce between
  left and right and follow the tempo.

The demo's pad has some reverb on it.

### Groups

Groups are folders of tracks, like layer groups in Photoshop. The demo puts
the four drums in a **Drums** group.

- **Ctrl-click** (Cmd on Mac) track names to pick several, then **Ctrl+G**.
  Or right-click a track name for *Group*, *Move to group* and *Remove from group*.
- **▶/▼** collapses or expands a group; collapsed, it shows all its hits on
  one line.
- The group's **M** / **S** mute or solo all its tracks. Click its name for a
  group volume.

### Showing and hiding

Hiding a part never deletes it or stops it playing.

- **View** menu, or **F5** song and patterns, **F6** tracks, **F7** piano roll,
  **F8** instrument panel.
- **View → Focus** shows just the tracks, or just the piano roll and sound.
- **F9** shows everything again.

### Groove

- **Shift-click** a step for a ghost note (a quiet hit). Quieter steps look dimmer.
- **Right-click** a step for its settings: **Velocity** (Ghost / Normal /
  Accent), **Chance** (plays only sometimes), **Ratchet** (repeats 2–4 times
  in the step, like a roll) and **Nudge** (a little early or late).
- **Swing** (top bar) delays every other 16th note; 66% is a triplet shuffle.
- **Humanize** (right panel) adds small random changes to a track's timing
  and loudness.

All of this goes out over MIDI too, with velocity.

### Piano roll

- Click a cell to add a note; click it again to remove it. Several notes in
  one column make a **chord**.
- **Right-click** the cell after a note to hold that note longer (shown in a
  darker blue). Longer notes let the synth's **Sustain** and **Release** be heard.
- **Right-click** a note for its groove settings; **shift-click** for a ghost note.
- Other tracks' notes show faintly behind, to help line parts up (View menu
  to turn off).

### Play from your computer keyboard

No MIDI keyboard needed: your laptop keys play the selected track.

```
 W E   T Y U   O P        ← black keys
A S D F G H J K L         ← white keys (A = C)
```

- **Z** / **X** move down or up an octave.
- Turn on **⏺ Rec** and press Play: notes you play are written into the
  pattern at the step that's playing. Play first clicks one bar (the
  count-in) so you can come in on time.
- **Click** (top bar) turns on the metronome: a click on every beat, higher
  on beat 1.

### Sample tracks

- **Pitched off:** plays as recorded, like a drum hit.
- **Pitched on:** each note plays the sample higher or lower, like an
  instrument. C at octave 4 is the original pitch.

On Arch, the **+ Sample…** file picker needs `xdg-desktop-portal` and
`xdg-desktop-portal-gtk`. Drag and drop always works.

### Patterns and song

- **A–H** pick which pattern you're editing. Each holds every track's steps.
- **Copy** / **Paste** / **Clear** work on the pattern you're editing.
- **Pattern** mode loops the pattern you're editing.
- **Song** mode plays the **Song** row left to right, one bar per box.
  Click a box to drop in the selected pattern; click it again (or
  right-click) to empty it. Empty boxes in the middle are silent bars.

### Synth

Select a synth track to see its knobs. Drag a knob up or down; double-click
to reset it. Hover for what each one does.

- **Sound:** **Wave** saw → square. **Detune** thickens. **Humanize**.
- **Filter:** **Cutoff** and **Reso** set the tone; **Env amt** and
  **F decay** make each note "pluck" by opening the filter and closing it.
- **Envelope (ADSR):** **Attack** fades in, **Decay** falls to the
  **Sustain** level while the note is held, **Release** fades out after.

Try: Cutoff down, Reso up, Env amt up, Decay short → squelchy acid bass.

### MIDI

Plug gear in over USB (or a USB-MIDI cable), then press **⟳** to rescan.

- **MIDI out:** sends clock and Start/Stop so gear follows the tempo. Each
  track's **MIDI ch** (right panel) sends its notes on that channel; the
  built-in drums use channel 10 (Kick 36, Snare 38, Hat 42, Clap 39).
- **MIDI in:** click it to connect controllers, several at once. Each plays
  the **selected track** (follows your clicks) or a fixed **track** or
  **group**. Route a drum kit to the Drums group and each pad plays its
  matching drum. With **⏺ Rec** on and playing, what you play is written into
  the pattern, chords included.
- **Follow MIDI clock:** the incoming clock sets the tempo, and the other
  device's Start/Stop controls playback.

## Roadmap

1. ~~Drum step sequencer~~
2. ~~MIDI clock and notes for the FM-1 and AIRA Compacts~~
3. ~~Built-in synth with knobs~~
4. ~~Patterns and song arrangement~~
5. ~~Add/remove tracks, per-track volume, sample tracks~~
6. ~~Groove: swing, velocity/ghost notes, humanize, chance, ratchets, nudge; solo~~
7. ~~Track groups, View menu and focus mode~~
8. ~~Chords, other tracks' notes in the piano roll, routed MIDI inputs with recording~~
9. ~~Save and load songs~~
10. ~~Export to WAV and MIDI~~
11. Sample editor: chop, trim, ADSR, fades, reverse
12. ~~Pan, reverb and delay~~ (a full mixer to come)
13. Recording
14. Freesound.org sample browser
15. ~~Web version~~

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
