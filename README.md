# daw

A tiny, lightweight DAW in Rust. Right now: a 4-track, 16-step drum machine.

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

## Roadmap

1. ~~Drum step sequencer~~
2. MIDI clock and notes for the FM-1 and AIRA Compacts
3. Built-in synth
4. Recording
5. Freesound.org sample browser
