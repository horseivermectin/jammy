use crate::app::PlaybackRequest;
use anyhow::{bail, Context, Result};
use std::process::Command;

/// Launches mpv for the given request and blocks until the player closes.
///
/// Still deliberately simple: no IPC socket, no pause/seek/stop control
/// from our side, just "start mpv, let it own its own window, wait for it
/// to exit." Playback control from within our TUI (and reporting progress
/// back to Jellyfin as you watch) is a separate piece of work.
pub fn play(request: &PlaybackRequest) -> Result<()> {
    let mut cmd = Command::new("mpv");
    cmd.arg("--fs")
        .arg("--no-terminal")
        .arg("--wayland-app-id=jellyfin-tui-mpv")
        .arg(format!("--title=Jellyfin: {}", request.title));

    if let Some(start) = request.start_seconds {
        cmd.arg(format!("--start={start}"));
    }

    // Always explicit, even when None (--sid=no) — harmless if the file
    // has no subtitle tracks at all, and correct when the user picked
    // "No subtitles" on the selection screen.
    match request.subtitle_index {
        Some(sid) => {
            cmd.arg(format!("--sid={sid}"));
        }
        None => {
            cmd.arg("--sid=no");
        }
    }

    cmd.arg(&request.url);

    let status = cmd
        .status()
        .context("failed to launch mpv — is it installed and on your PATH?")?;

    if !status.success() {
        bail!("mpv exited with a non-zero status: {status}");
    }

    Ok(())
}
