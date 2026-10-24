# daw

A tiny, lightweight DAW in Rust: drums, a melody track and a built-in synth,
8 patterns you can arrange into a song, and MIDI for hardware like the M-VAVE
FM-1 and Roland AIRA Compacts.

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

- **Space** or **▶ Play** starts and stops.
- Click a pad to turn a step on or off.
- Click a drum name to hear it.
- Drag **BPM** to change the tempo.

### Patterns and song

- **A–H** pick which pattern you're editing. Each has its own drums and melody.
- **Copy** / **Paste** / **Clear** work on the pattern you're editing.
- **Pattern** mode loops the pattern you're editing.
- **Song** mode plays the **Song** row left to right, one bar per box.
  Click a box to drop in the selected pattern; click it again (or
  right-click) to empty it. Empty boxes in the middle are silent bars.

### Synth

The knobs on the right shape the built-in synth. Drag a knob up or down;
double-click to reset it. Hover for what each one does.

- **Wave** saw → square. **Detune** thickens. **Volume**.
- **Cutoff** and **Reso** set the filter; **Env amt** and **Decay** make each
  note "pluck" by opening the filter and closing it again.
- **Attack** / **Release** fade notes in and out.

Try: Cutoff down, Reso up, Env amt up, Decay short → squelchy acid bass.

### Melody

- Click a cell to put a note on that step; click it again to remove it.
  One note per step.
- Click a note name to hear it.
- **Octave** shifts the whole melody.
- Synth **On** (right panel) plays it here. Turn it off to hear only your MIDI gear.
- **MIDI ch** is the channel the melody is sent on (match it on the FM-1).

### MIDI

Plug gear in over USB (or a USB-MIDI cable), then press **⟳** to rescan.

- **MIDI out:** sends clock, Start/Stop and, with **Send notes** on, the
  drum hits on channel 10 (Kick 36, Snare 38, Hat 42, Clap 39). Other gear
  follows the tempo; drum machines like the T-8 can play the pattern.
- **MIDI in:** notes 36/38/42/39 from any channel play the drums.
- **Follow MIDI clock:** the incoming clock sets the tempo, and the other
  device's Start/Stop controls playback.

## Roadmap

1. ~~Drum step sequencer~~
2. ~~MIDI clock and notes for the FM-1 and AIRA Compacts~~
3. ~~Built-in synth with knobs~~
4. ~~Patterns and song arrangement~~
5. Add/remove tracks, per-track volume
6. Save and load songs
7. Recording
8. Freesound.org sample browser

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
