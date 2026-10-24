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

- **Space** or **▶ Play** starts and stops. Drag **BPM** to change the tempo.

### Tracks

Each row is a track: its name, **M** (mute), and 16 steps.

- Click a step to turn it on or off.
- Click a track's name to select it and hear it. The right panel shows its
  settings; melodic tracks also get a **piano roll** below the rows.
- **+ Synth** adds a synth track. **+ Sample…** loads WAV, MP3, FLAC or OGG
  files as sample tracks, or just drop files onto the window.
- Rename a track by editing its name at the top of the right panel.

### Piano roll

- Click a cell to add a note; click it again to remove it.
- **Right-click** the cell after a note to hold that note longer (shown in a
  darker blue). Longer notes let the synth's **Sustain** and **Release** be heard.

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
- **MIDI in:** a note plays the one-shot track that sends the same note.
- **Follow MIDI clock:** the incoming clock sets the tempo, and the other
  device's Start/Stop controls playback.

## Roadmap

1. ~~Drum step sequencer~~
2. ~~MIDI clock and notes for the FM-1 and AIRA Compacts~~
3. ~~Built-in synth with knobs~~
4. ~~Patterns and song arrangement~~
5. ~~Add/remove tracks, per-track volume, sample tracks~~
6. Clip editing: trim, ADSR, fades, reverse
7. Save and load songs
8. Mixer and effects
9. Recording
10. Freesound.org sample browser

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
