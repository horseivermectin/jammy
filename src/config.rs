use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// What jellyfin-tui itself reads and writes between runs. Deliberately
/// small — just enough to skip re-entering the server URL and logging in
/// every launch. See config.example.toml (written once, alongside this
/// file, on first run) for the broader range of Jellyfin-adjacent settings
/// worth building toward.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: Option<String>,
    pub username: Option<String>,
    /// A saved access token from a previous successful login. This — not
    /// your password — is what lets you skip re-entering credentials.
    /// Jellyfin lets you revoke a token independently from Dashboard >
    /// Devices if this file ever leaks, which a stored plaintext password
    /// wouldn't allow.
    pub access_token: Option<String>,
    pub user_id: Option<String>,
    /// A stable per-install id Jellyfin uses to recognize "this is the
    /// same device" across logins (affects resume points, the Devices
    /// list, etc). Generated once on first run.
    pub device_id: Option<String>,
}

impl Config {
    fn dirs() -> Result<ProjectDirs> {
        ProjectDirs::from("", "", "jellyfin-tui")
            .ok_or_else(|| anyhow!("could not determine a config directory for this platform"))
    }

    fn path() -> Result<PathBuf> {
        Ok(Self::dirs()?.config_dir().join("config.toml"))
    }

    fn example_path() -> Result<PathBuf> {
        Ok(Self::dirs()?.config_dir().join("config.example.toml"))
    }

    /// Loads the config, creating an empty one (plus the reference example
    /// file) the first time this runs.
    pub fn load_or_create() -> Result<Self> {
        let path = Self::path()?;

        let config = if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("reading config at {}", path.display()))?;
            toml::from_str(&text)
                .with_context(|| format!("parsing config at {}", path.display()))?
        } else {
            Self::default()
        };

        Self::write_example_if_missing()?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating config directory {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, text).with_context(|| format!("writing config to {}", path.display()))
    }

    /// Returns the saved device id, generating and persisting a new one the
    /// first time this is called.
    pub fn device_id(&mut self) -> Result<String> {
        if let Some(id) = &self.server.device_id {
            return Ok(id.clone());
        }
        let id = generate_device_id();
        self.server.device_id = Some(id.clone());
        self.save()?;
        Ok(id)
    }

    fn write_example_if_missing() -> Result<()> {
        let path = Self::example_path()?;
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, EXAMPLE_CONFIG)
            .with_context(|| format!("writing example config to {}", path.display()))
    }
}

/// Not cryptographically random — just needs to be stable per install and
/// unlikely to collide with another install running at the same instant,
/// which timestamp-plus-hex easily satisfies without pulling in a `uuid` or
/// `rand` dependency for something this low-stakes.
fn generate_device_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("jellyfin-tui-{nanos:x}")
}

/// A reference file — written once, never overwritten by the app — showing
/// the broader range of settings Jellyfin's own server/client model exposes.
/// Only `[server]` is actually read by jellyfin-tui today; everything below
/// it documents what a future feature could plug into, so it doesn't have
/// to be reverse-engineered from Jellyfin's API docs from scratch.
const EXAMPLE_CONFIG: &str = r#"# jellyfin-tui example config
# Written once for reference; the app never modifies this file.
# Your real, active settings live in config.toml next to this one.

[server]
# Everything in this section IS read by jellyfin-tui today.
url = "http://localhost:8096"
username = "your-jellyfin-username"
# access_token / user_id / device_id are filled in automatically after your
# first successful login — no need to set these by hand.

# --- Nothing below this line is implemented yet ---
# These mirror real Jellyfin server/client settings, so whoever picks up
# the next feature knows what's realistically available to build toward.

[playback]
# ISO 639-2 codes. Jellyfin exposes multiple audio/subtitle tracks per
# item (you've seen this in mpv's own track list) — this would let the
# client auto-select instead of leaving it to mpv's own defaults.
preferred_audio_language = "eng"
preferred_subtitle_language = "eng"
# Burn subtitles into the video vs. mpv rendering them as an overlay track.
burn_in_subtitles = false
# Cap direct-play/transcode bitrate for slower connections (Kbps).
max_bitrate_kbps = 20000
# Extra args passed straight to mpv, for anything not modeled above.
mpv_extra_args = []

[playback.resume]
# Only offer "resume from where you left off" past this many seconds in.
threshold_seconds = 30

[sway]
# app_id tag applied to mpv windows, so your own Sway config can target
# just this app's playback windows, e.g.:
#   for_window [app_id="jellyfin-tui-mpv"] floating enable
mpv_app_id = "jellyfin-tui-mpv"
# Whether to request fullscreen on playback start (current default: yes).
fullscreen_on_play = true
"#;
