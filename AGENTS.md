# AGENTS.md

Guidance for AI coding agents working in this repository. (Humans: this
doubles as the contributor guide.)

## Project overview

**jammy** (cargo package: `jellyfin-tui`) is a terminal-UI client for the
[Jellyfin](https://jellyfin.org) media server. It browses libraries in the
terminal, plays media by spawning mpv, and reports mpv's playback position
to Jellyfin. Single binary crate, no workspace, ~1.6k lines of Rust.

Stack: `tokio` + `reqwest` (rustls) for the Jellyfin REST API, `ratatui` +
`crossterm` for the UI, `serde`/`serde_json` for API models, `anyhow` for
errors, `directories` + `toml` for config. There is no CI yet — local
checks are the gate.

## Setup

- Rust toolchain — read **Toolchain constraints** below before touching
  dependencies.
- mpv on `PATH` for any manual playback testing.
- A Jellyfin server for manual/integration testing. Unit tests must **not**
  require one (or any network).
- Sandbox note: if the checkout lives on a `noexec` mount, builds fail with
  "Permission denied" running build scripts — point
  `CARGO_TARGET_DIR` at a normal filesystem (e.g. `/tmp/target`).

## Commands

```sh
cargo build
cargo test                      # unit tests, no network needed
cargo clippy --all-targets      # keep warning-free
cargo fmt                       # run before committing
```

Current baseline (last verified): build + clippy clean apart from two known
`dead_code` warnings on deserialization-only model fields
(`JellyfinUser::name`, `ItemsResponse::total_record_count`); 7/7 tests pass;
`cargo fmt --check` is clean.

## Toolchain constraints (important)

`Cargo.toml` deliberately pins `reqwest = "0.11"` (rustls) and several
transitive crates (`idna_adapter = "=1.1.0"`, `indexmap = "=2.2.6"`,
`unicode-segmentation = "=1.11.0"`) so the project builds with **rustc
1.75** — a constraint of the environment it was first developed in, not a
principled MSRV.

- Do **not** bump these pins as a side effect of unrelated work.
- If a task requires modernizing them, verify the build against the oldest
  toolchain you intend to support, say so explicitly in the PR, and add a
  `rust-version` field to `Cargo.toml` when the floor changes.

## Codebase map

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Entry point: config load, login/session restore, terminal setup/teardown, the event loop, **all key handling** |
| `src/app.rs` | Application state and navigation/playback decisions: the `Entry` flattening, the `Mode` state machine, resume threshold, subtitle-track resolution |
| `src/jellyfin/client.rs` | `JellyfinClient`: auth, `/Users/{id}/Views`, `/Items`, item details, played/unplayed, stream-URL building |
| `src/jellyfin/models.rs` | serde models of Jellyfin JSON — only fields the app actually reads — plus the deserialization tests |
| `src/ui.rs` | Rendering only: draws from `App` state, owns no behavior |
| `src/mpv.rs` | Spawns mpv, owns its private Unix IPC socket, and samples playback position/pause state |
| `src/config.rs` | `config.toml` load/save, device-id generation, the aspirational `config.example.toml` text |
| `src/subtitle_prefs.rs` | Per-item subtitle memory (JSON in the XDG data dir) |

Key handling stays in `main.rs`; state transitions stay in `app.rs`;
`ui.rs` stays behavior-free. Keep it that way.

## Conventions

- **Errors:** `anyhow` throughout. Attach `.context()` at I/O boundaries
  with the path/URL involved. User-facing failures surface via
  `app.status`, never via panic in the event loop.
- **Comments explain *why*, not *what*.** Several are load-bearing domain
  facts — keep them accurate when you change behavior:
  - mpv's `--sid` numbers *subtitle-type* streams from 1; Jellyfin's
    `MediaStream.Index` is container-wide. `app.rs` maps ordinal → sid at
    play time; `subtitle_prefs` persists the *absolute* index because it's
    the stable one.
- Jellyfin ticks are 100ns units; `MIN_RESUME_TICKS` is 10s.
- mpv reports `time-pos` in seconds; `mpv.rs` converts it to Jellyfin's
  100ns ticks. `main.rs` posts progress every 10s and once on exit.
- Jellyfin may discard a valid reported position outside its server-side
  `MinResumePct`/`MaxResumePct` window. Tests below the minimum can advance
  `PlayCount`/`LastPlayedDate` while leaving `PlaybackPositionTicks` at zero.
  - `restore_session()` does not validate the token; `main.rs` treats the
    first real request (`get_views` in `App::new`) as the validity check
    and falls back to a fresh login.
- **State machines as enums**, not parallel `Option`/`bool` fields — see
  `Mode`. Illegal states should remain unrepresentable.
- **Terminal hygiene:** raw mode / alternate screen must be restored on
  *every* exit path, including errors. If you add code that can panic
  inside the TUI, install a panic hook that restores the terminal first.
- **Secrets:** never log the access token or password. The token currently
  appears in mpv's command line (`api_key=` in the stream URL) — a known
  limitation; don't make it worse (e.g. don't start echoing URLs).
- **Dependencies:** the tree is intentionally small. New deps need a stated
  reason in the PR.

## Testing guidance

- New Jellyfin response handling → add a deserialization test with a
  realistic payload; `models.rs` tests are the pattern (they pin assumed
  JSON shapes precisely because no real server is reachable in tests).
- Logic that needs no network (sid mapping, resume-threshold filtering,
  remembered-preference resolution) should be **extracted into pure
  functions** and unit-tested — much of it is currently embedded in
  `App` methods and untestable.
- Manual smoke test before a PR: browse into a library → play an item for
  at least 10s → quit mpv → confirm the percentage updates → play it again
  and confirm resume → use the subtitle picker → toggle watched → quit and
  relaunch (session restore) → revoke token server-side and relaunch
  (re-login path).

## Commit / PR guidance

- Small, focused commits that describe behavior changes.
- Keep `Cargo.lock` changes tied to the dependency change that caused them
  (the lockfile is committed on purpose — this is a binary).
- Update `README.md` (features, keybindings, limitations) and the codebase
  map above whenever you change what they describe.
