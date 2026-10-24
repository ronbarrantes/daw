# Workflow: push, merge, release

## Branches

- **`main`** always builds and runs. It's what you pull on your laptops.
- **Work branches** hold changes in progress. Claude sessions use
  `claude/<name>`; your own work can use `feature/<name>`.
- Never commit straight to `main`.

## 1. Push (on a work branch)

Each change is a small commit that does one thing. Before every push:

```sh
cargo fmt
cargo clippy --all-targets   # no warnings
cargo test                   # all pass
```

Then `git push -u origin <branch>`.

## 2. Try it

On whichever laptop is handy:

```sh
git fetch
git checkout <branch>
cargo run --release
```

The Air (Arch) is the one that counts for speed and screen size. If it's
fine there, it's fine everywhere.

## 3. Merge

1. Open a pull request from the work branch into `main`.
2. Wait for CI to pass (see below).
3. Merge with **Squash and merge**, so each feature is one commit on `main`.
4. Update your laptops: `git checkout main && git pull`.

After a merge, a Claude session branch restarts from the new `main` for the
next change, so the old merged commits don't come along again.

## 4. CI

`.github/workflows/build.yml` runs on every push (any branch) and every
pull request:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test`, on **ubuntu-latest** and **macos-latest**
- a release build for **Linux x86_64** and **macOS Apple Silicon**, kept as
  downloadable artifacts on the run's page

Only merge a pull request when its checks are green. To have GitHub enforce
that: repo **Settings → Branches → Add rule** for `main`, tick *Require
status checks to pass* and pick the **Check** jobs.

## 5. Release

Releases are prebuilt downloads, so neither laptop has to compile.

- **Every merge to `main`** replaces the **`latest`** pre-release with fresh
  builds for Linux x86_64 (the Air running Arch) and macOS Apple Silicon
  (the M3). Nothing to do; just merge.
- **A version** gets its own permanent release:
  1. On `main`, set `version` in `Cargo.toml` (e.g. `0.2.0`) and commit.
  2. Tag it and push the tag:
     ```sh
     git tag v0.2.0
     git push origin v0.2.0
     ```
  The same workflow builds it and writes release notes from the merged
  pull requests.

Version numbers stay `0.x` while the app is young: bump the middle number
for new features (`0.2.0` → `0.3.0`) and the last one for fixes
(`0.3.0` → `0.3.1`).
