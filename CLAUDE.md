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

Keep [docs/ROADMAP.md](docs/ROADMAP.md) and the guides (README, F1 help)
up to date in every PR; see WORKFLOW.md.

Ron reads in short bursts: keep replies short and plain.

## Code

- `src/audio.rs`: the engine. The audio callback must never lock, block or
  allocate; the UI publishes `Arc<Project>` snapshots and sends `Event`s.
- `src/project.rs`: the song data. The units: pattern (one bar of steps),
  chain (patterns bar by bar), block (a chain placed on the timeline; a
  `Clip` in code is one placed copy), song. Timeline positions are in
  steps (16ths).
- `src/timeline.rs`: the Jam view's Chain row and pattern library, and the
  Arrange view's timeline and block drawer. A block is a chain or a MIDI
  block (free notes for one track, `Block::midi`, times in `PPS`ths of a
  step); `src/notes.rs` is the MIDI block's note editor.
- `src/main.rs`: the UI; `src/widgets.rs` small widgets; `src/help.rs` the
  F1 guide (keep it in step with the README).
- The web version is the same app built for `wasm32-unknown-unknown`:
  `src/web.rs` has the browser-only parts (downloads, uploads, autosave),
  `web/` the page and build script. Keep both builds working; there are no
  threads or file paths on the web.
- `packaging/`: `make_icon.py` draws `assets/icon*.png` and `icon.ico`
  (commit them; `build.rs` puts the `.ico` into the Windows `daw.exe`);
  `macos/Info.plist` for the app bundle; `linux/` has the installer and
  `.desktop` entry. `.github/workflows/build.yml` assembles them.
- Before pushing: `cargo fmt`, `cargo clippy --all-targets` with no
  warnings, `cargo test`, and `cargo clippy --target wasm32-unknown-unknown`
  for the web version.
- Stick to glyphs egui's default font has; arrows like → and ▼ don't render.
