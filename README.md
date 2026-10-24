# daw

A tiny, lightweight DAW in Rust: synth and sample tracks, 8 patterns you can
arrange into a song, and MIDI for hardware like the M-VAVE FM-1 and Roland
AIRA Compacts.

## Setup (once)

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

## Run

```sh
git clone https://github.com/ronbarrantes/daw.git
cd daw
cargo run --release
```

The first build takes a few minutes. After that it's fast.

## Use

Press **F1** (or **Help → How to use**) for the built-in guide.

- **Space** or **▶ Play** starts and stops. Drag **BPM** to change the tempo.

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
  pattern at the step that's playing.

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

- **Sound:** **Wave** saw → square. **Detune** thickens. **Volume**.
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
9. Save and load songs
10. Sample editor: chop, trim, ADSR, fades, reverse
11. Mixer and effects
12. Recording
13. Freesound.org sample browser

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
