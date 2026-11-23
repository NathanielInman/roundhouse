# Roundhouse

`docs/design.md` is the spec; the README is the tour. Read the spec section
for whatever you change.

## Crates

- `bar-anchor` is no longer in this repo: it's the shared
  [bar-anchor](https://github.com/NathanielInman/bar-anchor) crate, pinned by tag in the root `Cargo.toml`.
  Change it there and move the tag; its `Anchor` JSON must stay
  identical across the apps.
- `rh-core`: collectors (`collect/`), the snapshot model, lamp rules, config,
  the socket protocol and the waybar reader. No UI.
- `rh`: the daemon, CLI and waybar module (one binary, `rh`).
- `rh-board`: the Linux GTK4 layer-shell drawer. `anim.rs` and `stage.rs`
  came from Switchyard's board.

## Build and test

`cargo build --workspace --locked`, `cargo test --workspace --locked` and
`cargo clippy --workspace --all-targets`. The drawer needs gtk4-layer-shell;
`.github/scripts/gtk4-layer-shell.sh` is how CI gets it.

Parsers are pure functions over text and are tested against real output
(`/proc/stat`, mountinfo, waybar CSS and JSONC, compositor replies). Add a
test with a real sample when you touch one.

`crates/rh/tests/daemon.rs` drives a real daemon in a throwaway directory,
with `ROUNDHOUSE_BOARD=/bin/true` so no drawer opens. The lamp's JSON is a
contract with other machines' daemons and scripts:
`crates/rh-core/testdata/lamp-contract.json` holds every field they may
read, and a test fails if one disappears or changes type. Add new fields to
the fixture; never remove one.

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

Installing over `~/.local/bin/rh` upgrades the running daemon, drawer and
bar modules on their own within a tick (docs/design.md, Upgrades);
`rh restart` does it at once.

## Rules

- No subprocesses in the daemon's sampling path. Read `/proc`, `/sys`, a
  socket, or a library loaded at runtime.
- Nothing may stall a sample: network filesystems and other machines get a
  deadline.
- Never touch an automount point that is not mounted.

## When behavior changes

Update in the same commit:

- `docs/design.md` for a new config key, protocol message, collector or lamp
  rule.
- The README for anything a new user meets (CLI, keys, install, config).

Commits are conventional, scoped by area: `anchor`, `core`, `collect`,
`lamp`, `cli`, `daemon`, `board`, `docs`, `ci`.

## Platforms

The suite targets Linux, Windows and macOS. `platforms` in
`docs/assets/suite.toml` lists where this app builds today, and `planned`
what a port still owes; CI and the pre-push hook hold it to `platforms`.

- Turn the hook on once per clone: `git config core.hooksPath .githooks`.
  Before a push it runs clippy for every platform listed (needs
  `rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin` and, for
  crates with C code, `mingw-w64-gcc`). A failure there is a broken build on
  that platform: fix it, don't push past it with `--no-verify`.
- Code for one platform goes behind `#[cfg(...)]`, helpers included: a
  function only Linux calls is dead code on Windows, and `-D warnings` fails
  the build there.
- A test that needs something one platform has (a Linux-only binary, Unix
  socket behavior, a POSIX shell, a path layout) is gated with `#[cfg(...)]`
  or skips elsewhere.
