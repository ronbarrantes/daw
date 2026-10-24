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

## 4. CI (to set up)

A GitHub Actions workflow (`.github/workflows/ci.yml`) runs on every pull
request and every push to `main`:

- `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`
- on **ubuntu-latest** (installs `libasound2-dev`) and **macos-latest**

A pull request can only merge when CI is green.

## 5. Release (to set up)

Releases are prebuilt downloads, so neither laptop has to compile.

1. On `main`, set `version` in `Cargo.toml` (e.g. `0.2.0`) and commit.
2. Tag it and push the tag:
   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```
3. A release workflow (`.github/workflows/release.yml`) builds for
   - **Linux x86_64** (the Air running Arch)
   - **macOS Apple Silicon** (the M3)

   and attaches both to a GitHub Release with notes made from the merged
   pull requests.

Version numbers stay `0.x` while the app is young: bump the middle number
for new features (`0.2.0` → `0.3.0`) and the last one for fixes
(`0.3.0` → `0.3.1`).
