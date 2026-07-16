# jammy

A terminal UI (TUI) client for [Jellyfin](https://jellyfin.org), written in Rust.
Browse your media libraries in the terminal; play them in mpv.

**Status: early development.** The core loop — log in, browse, play, resume,
pick subtitles, toggle watched — works. See [Limitations](#limitations) before
making it your daily driver.

## Features

- Browse libraries and folders (movies, series → seasons → episodes) as a
  keyboard-driven list, with a breadcrumb trail
- Playback through [mpv](https://mpv.io): fullscreen, in its own window
  (tagged with Wayland app-id `jellyfin-tui-mpv` so your compositor can
  target it)
- Resume where you left off — `Enter` resumes from the server's saved
  position, `Shift+R` restarts from the beginning
- Subtitle picker (`Shift+S`) with the container default pre-selected; your
  choice is remembered per item, so a foreign-language film only needs
  picking once
- Watched-state at a glance: ✅ for watched, a percentage for in-progress,
  and `m` to toggle manually
- One-time login: server URL and a revocable access token persist between
  runs — your password is never stored

## Requirements

- A Jellyfin server you have an account on
- mpv installed and on your `PATH`
- A Rust toolchain (see [Toolchain notes](#toolchain-notes))
- Linux is the development target; other platforms are untested (the code
  avoids Linux-only APIs, so reports/patches are welcome)

## Installation

```sh
git clone https://github.com/horseivermectin/jammy
cd jammy
cargo build --release     # binary lands at target/release/jellyfin-tui
```

Or install it onto your PATH:

```sh
cargo install --path .
```

## Usage

```sh
jellyfin-tui
```

On first run you'll be prompted for:

1. **Server URL** (e.g. `http://localhost:8096`)
2. **Username**
3. **Password**

After a successful login the password is discarded and a revocable access
token is saved instead. Later launches go straight to your libraries; if the
saved token stops working (revoked, expired), you'll simply be asked to log
in again.

> **Note:** if you point jammy at a server over plain `http://` on a network
> you don't control, your credentials and token travel unencrypted. Prefer
> an `https://` URL for anything beyond localhost/LAN.

### Keybindings

**Browsing**

| Key                  | Action                                             |
| -------------------- | -------------------------------------------------- |
| `↑`/`↓` or `j`/`k`   | Move selection (wraps around)                      |
| `Enter`              | Open folder / play item (resuming if possible)     |
| `Shift+R`            | Play from the beginning, ignoring resume position  |
| `Shift+S`            | Play, forcing the subtitle picker open             |
| `m`                  | Toggle watched / unwatched                         |
| `Backspace` / `←`    | Back up one level                                  |
| `q`                  | Quit                                               |

**Subtitle picker**

| Key                            | Action  |
| ------------------------------ | ------- |
| `↑`/`↓` or `j`/`k`             | Move    |
| `Enter`                        | Confirm |
| `Esc` / `Backspace` / `←` / `q`| Cancel  |

## Configuration

Config lives in your XDG config directory — on Linux,
`~/.config/jellyfin-tui/config.toml`:

```toml
[server]
url = "http://localhost:8096"
username = "your-jellyfin-username"
# The fields below are managed by the app; no need to edit by hand.
access_token = "..."
user_id = "..."
device_id = "..."
```

- **The access token is a secret** — anyone holding it can act as you on
  your server. If the file ever leaks, revoke the token from the Jellyfin
  dashboard (Dashboard → Devices) instead of changing your password.
- To switch servers or users, delete `config.toml` (or clear
  `access_token`/`user_id`) and restart.
- A `config.example.toml` is written alongside the real config on first
  run. It sketches planned `[playback]`, `[playback.resume]` and `[sway]`
  settings — those sections are **aspirational**; only `[server]` is read
  today.

Remembered subtitle choices live separately in the XDG data directory — on
Linux, `~/.local/share/jellyfin-tui/subtitle_prefs.json`, keyed by Jellyfin
item id. Deleting it just means the picker comes back on next play.

## Limitations

- **Direct play only.** jammy skips Jellyfin's `/PlaybackInfo` negotiation
  and streams the original file (`static=true`). If a file can't be played
  as-is (unsupported codec/container, insufficient bandwidth), playback
  fails where official clients would fall back to transcoding.
- **No progress reporting yet.** jammy doesn't tell the server where you
  stopped, so resume positions and watched state only reflect what *other*
  clients reported. Finishing something doesn't auto-mark it watched —
  press `m`.
- **mpv owns the screen while playing.** The TUI pauses until mpv exits;
  there's no pause/seek/stop control from within jammy yet.
- **Text-only UI** — no posters or thumbnails.
- Only video playback is exercised; non-video libraries browse fine but
  play at your own risk.

## Roadmap

Roughly in priority order:

- Report playback progress to Jellyfin (`/Sessions/Playing/*`), driven by
  mpv IPC (`--input-ipc-server`) — this unlocks real resume points and
  automatic watched-marking
- mpv control from the TUI (pause/seek/stop) instead of blocking on exit
- `/PlaybackInfo` negotiation with transcoding fallback
- Implement the `[playback]` / `[sway]` config sections sketched in
  `config.example.toml`
- Modernize the pinned dependency set (see below)

## Toolchain notes

`Cargo.toml` deliberately pins `reqwest` 0.11 (with rustls), `ratatui`
0.26, and a few transitive crates (`idna_adapter`, `indexmap`,
`unicode-segmentation`) so the project builds with an older rustc (1.75).
On a current stable toolchain everything builds as-is. Unpinning and
modernizing is planned future work — don't "fix" the pins casually.

## Contributing

Issues and PRs are welcome. See [AGENTS.md](AGENTS.md) for the codebase
map, build/test commands, and conventions (it's written for AI coding
agents, but everything in it applies to humans too).

## License

No license chosen yet — all rights reserved for now.
