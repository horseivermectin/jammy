use crate::app::PlaybackRequest;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TICKS_PER_SECOND: f64 = 10_000_000.0;

pub struct PlaybackState {
    pub position_ticks: i64,
    pub is_paused: bool,
}

/// A running mpv process with a private JSON IPC socket. The TUI still
/// waits for mpv to close, but it can now sample playback state while it
/// waits and pass that state back to Jellyfin.
pub struct Player {
    child: Child,
    ipc_path: PathBuf,
}

impl Player {
    pub fn start(request: &PlaybackRequest) -> Result<Self> {
        let ipc_path = unique_ipc_path();
        let mut cmd = Command::new("mpv");
        cmd.arg("--fs")
            .arg("--no-terminal")
            .arg("--wayland-app-id=jellyfin-tui-mpv")
            .arg(format!("--title=Jellyfin: {}", request.title))
            .arg(format!("--input-ipc-server={}", ipc_path.display()));

        if let Some(start) = request.start_seconds {
            cmd.arg(format!("--start={start}"));
        }

        // Always explicit, even when None (--sid=no) — harmless if the file
        // has no subtitle tracks, and correct when the user picked off.
        match request.subtitle_index {
            Some(sid) => {
                cmd.arg(format!("--sid={sid}"));
            }
            None => {
                cmd.arg("--sid=no");
            }
        }

        cmd.arg(&request.url);
        let child = cmd
            .spawn()
            .context("failed to launch mpv — is it installed and on your PATH?")?;

        Ok(Self { child, ipc_path })
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child.try_wait().context("checking mpv process status")
    }

    /// Samples mpv's current position. `None` is normal during startup and
    /// shutdown, when the process exists but its socket is not accepting
    /// requests. Playback itself should not fail because telemetry missed a
    /// sample.
    pub fn playback_state(&self) -> Option<PlaybackState> {
        let seconds = query_property(&self.ipc_path, "time-pos")?.as_f64()?;
        let position_ticks = seconds_to_ticks(seconds);
        let is_paused = query_property(&self.ipc_path, "pause")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        Some(PlaybackState {
            position_ticks,
            is_paused,
        })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // mpv normally removes its own socket. This covers failed starts and
        // abnormal exits without leaving junk in the temporary directory.
        let _ = fs::remove_file(&self.ipc_path);
    }
}

fn unique_ipc_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "jellyfin-tui-{}-{nanos:x}.sock",
        std::process::id()
    ))
}

fn query_property(socket_path: &Path, property: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(socket_path).ok()?;
    let timeout = Some(Duration::from_millis(500));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;

    let request = json!({ "command": ["get_property", property] });
    writeln!(stream, "{request}").ok()?;

    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).ok()?;
    let response: Value = serde_json::from_str(&response).ok()?;
    (response.get("error")?.as_str()? == "success")
        .then(|| response.get("data").cloned())
        .flatten()
}

fn seconds_to_ticks(seconds: f64) -> i64 {
    (seconds.max(0.0) * TICKS_PER_SECOND) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_mpv_seconds_to_jellyfin_ticks() {
        assert_eq!(seconds_to_ticks(0.0), 0);
        assert_eq!(seconds_to_ticks(30.5), 305_000_000);
        assert_eq!(seconds_to_ticks(-1.0), 0);
    }
}
