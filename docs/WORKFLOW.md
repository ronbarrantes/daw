# Workflow: request, PR, merge, release

## How we work

Ron works on this away from his main computer, in short bursts, and gives
ideas rather than code. So:

1. **Ron asks for something.** A question is not a request. Answer it,
   and if it points to something worth building, offer it and wait for a
   clear yes before starting. "Oh, that's a good idea" or "that would be
   great to have" is not a yes. Ron often keeps talking and the idea
   grows, so waiting lets the whole thing land in one pass (e.g. one PR
   recolouring the knobs, chips and sliders together, not three PRs).
2. **Claude builds it in as few pull requests as makes sense.** Before
   starting, check whether nearby parts want the same change, and ask
   Ron if the scope is unclear. Split only when the pieces really are
   separate.
3. **Claude merges each PR into `main` once CI is green**, without waiting
   for approval. The exception is a big breaking change (e.g. old saved
   songs stop loading, or a feature is removed or reworked): ask first.
4. **Every merged PR bumps the version and gets a tagged release** (see
   *Versions* below), so there's always a numbered download.

Ron gets new builds from the Releases page or the one-line `curl` commands
in the README. Neither laptop has to compile.

This flow isn't fixed. When something in it stops fitting, update this file
in the same PR as the change that needs it.

## Branches

- **`main`** always builds and runs.
- **Work branches** hold changes in progress. Claude sessions use
  `claude/<name>`; hand-made work can use `feature/<name>`.
- Never commit straight to `main`.
- After a squash merge, the work branch restarts from the new `main` (a
  force-push to the work branch is fine), so merged commits don't come
  along again.

## Keep the docs in step

Every PR that changes what the app does also updates, in the same PR:

- **[ROADMAP.md](ROADMAP.md):** tick off what's done (with its version)
  and drop or reword anything that's no longer the plan.
- **The guides:** the README's *Use* section and the F1 guide
  (`src/help.rs`) say how to use the new thing, in plain words.

## Before every push

```sh
cargo fmt
cargo clippy --all-targets   # no warnings
cargo test                   # all pass
```

## Each pull request

1. Bump `version` in `Cargo.toml` as part of the PR (see *Versions*), and
   let `cargo build` update `Cargo.lock`.
2. Push and open a PR into `main` with a short description of what changed
   and how it was tested.
3. Wait for the **Build** workflow to pass on the PR.
4. **Squash and merge**, so each PR is one commit on `main`. The Build
   workflow then tags and releases the new version by itself; no manual
   tags needed (Claude's sandbox can't push tags anyway).

## CI

`.github/workflows/build.yml` runs on every push (any branch) and every
pull request:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test`, on **ubuntu-latest**, **macos-latest** and **windows-latest**
- a release build for **Linux x86_64**, **macOS Apple Silicon** and
  **Windows x86_64**, kept as
  downloadable artifacts on the run's page
- the **web version** (`cargo clippy` for `wasm32-unknown-unknown`, then
  `web/build.sh`); on `main` it's pushed to the `web-build` branch, which
  GitHub Pages serves (Settings → Pages → Deploy from a branch →
  `web-build`, `/ (root)`). Vercel could serve the same branch with no
  build step.

To have GitHub enforce green checks: repo **Settings → Branches → Add rule**
for `main`, tick *Require status checks to pass* and pick the **Check** jobs.

## Releases

- **Every push to `main`** replaces the **`latest`** pre-release with fresh
  builds for Linux x86_64 (the Air running Arch) and macOS Apple Silicon
  (the M3).
- **Every new version** in `Cargo.toml` that reaches `main` gets its own
  permanent `vX.Y.Z` release, tagged by the workflow, with notes written
  from the merged pull requests. A PR that doesn't bump the version only
  updates `latest`.
- A `v*` tag pushed by hand also gets a release.

## Versions

Version numbers stay `0.x` while the app is young:

- **New feature** → bump the middle number (`0.2.0` → `0.3.0`).
- **Fix or small tweak** → bump the last number (`0.3.0` → `0.3.1`).
- Several PRs from one request each get their own bump.
