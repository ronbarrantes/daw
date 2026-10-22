# daw

A tiny, lightweight DAW in Rust. Right now: a 16-step drum machine and melody sequencer with a built-in
synth, that talks MIDI.

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

### Melody

- Click a cell to put a note on that step; click it again to remove it.
  One note per step.
- Click a note name to hear it.
- **Octave** shifts the whole melody.
- **Built-in synth** plays it here. Turn it off to hear only your MIDI gear.
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
3. Built-in synth (basic version done; knobs to come)
4. Recording
5. Freesound.org sample browser
