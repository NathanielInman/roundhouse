# Roundhouse

`docs/design.md` is the spec; the README is the tour. Read the spec section
for whatever you change.

## Crates

- `bar-anchor`: Hyprland anchoring and panel placement, no GTK. Its `Anchor`
  JSON must stay identical to Switchyard's.
- `rh-core`: collectors (`collect/`), the snapshot model, lamp rules, config,
  the socket protocol, the waybar reader and the Switchyard client. No UI.
- `rh`: the daemon, CLI and waybar module (one binary, `rh`).
- `rh-board`: the Linux GTK4 layer-shell drawer. `anim.rs` and `stage.rs`
  came from Switchyard's board.

## Build and test

`cargo build --workspace --locked`, `cargo test --workspace --locked` and
`cargo clippy --workspace --all-targets`. The drawer needs gtk4-layer-shell;
`.github/scripts/gtk4-layer-shell.sh` is how CI gets it.

Parsers are pure functions over text and are tested against real output
(`/proc/stat`, mountinfo, waybar CSS and JSONC, Switchyard replies). Add a
test with a real sample when you touch one.

`crates/rh/tests/daemon.rs` drives a real daemon in a throwaway directory,
with `ROUNDHOUSE_BOARD=/bin/true` so no drawer opens. The lamp's JSON is a
sibling contract: `testdata/lamp-contract.json` holds every field Switchyard
may read, and the test fails if one disappears or changes type. Add new
fields to the fixture; never remove one.

## Release

Tag `vX.Y.Z` after bumping `workspace.package.version` (and `Cargo.lock`) in
a `chore(release)` commit. `.github/workflows/release.yml` builds x86_64 and
aarch64 tarballs, publishes the release, and renders
`packaging/aur/PKGBUILD` with `packaging/render.py`, committing it as
`$GITHUB_ACTOR`, never a bot.

## Isolation

The machine you work on may run an installed Roundhouse. Point a dev build
somewhere else with `ROUNDHOUSE_RUNTIME_DIR` and `ROUNDHOUSE_STATE_DIR` (and
`ROUNDHOUSE_CONFIG_DIR` for a test config).

## Rules

- No subprocesses in the daemon's sampling path. Read `/proc`, `/sys`, a
  socket, or a library loaded at runtime.
- Nothing may stall a sample: network filesystems and siblings get a
  deadline.
- A sibling (Switchyard) is optional. Code that talks to it treats absence as
  "feature hidden", never as an error.
- Never touch an automount point that is not mounted.

## When behavior changes

Update in the same commit:

- `docs/design.md` for a new config key, protocol message, collector or lamp
  rule.
- The README for anything a new user meets (CLI, keys, install, config).

Commits are conventional, scoped by area: `anchor`, `core`, `collect`,
`lamp`, `cli`, `daemon`, `board`, `docs`, `ci`.
