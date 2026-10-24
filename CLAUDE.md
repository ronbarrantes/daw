# daw

A lightweight DAW in Rust (eframe/egui UI, cpal audio, midir MIDI). It has to
run well on a 2012 11" MacBook Air running Arch (1366×768, dual-core) and on
an Apple Silicon Mac.

## How to work here

Follow [docs/WORKFLOW.md](docs/WORKFLOW.md). In short: turn each request
into one or more PRs, bump the version in each, and merge to `main` yourself
once CI is green; the Build workflow then tags and publishes the release.
Only ask Ron first for big breaking changes, and for anything about naming
or branding (the app's name, file extension, icon): talk it through and wait
for a clear "do it".

Ron reads in short bursts: keep replies short and plain.

## Code

- `src/audio.rs`: the engine. The audio callback must never lock, block or
  allocate; the UI publishes `Arc<Project>` snapshots and sends `Event`s.
- `src/project.rs`: the song data (tracks, lanes, patterns, groups).
- `src/main.rs`: the UI; `src/widgets.rs` small widgets; `src/help.rs` the
  F1 guide (keep it in step with the README).
- `packaging/`: `make_icon.py` draws `assets/icon*.png` (commit the PNGs);
  `macos/Info.plist` for the app bundle; `linux/` has the installer and
  `.desktop` entry. `.github/workflows/build.yml` assembles them.
- Before pushing: `cargo fmt`, `cargo clippy --all-targets` with no
  warnings, `cargo test`.
- Stick to glyphs egui's default font has; arrows like → and ▼ don't render.
