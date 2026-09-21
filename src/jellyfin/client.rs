use crate::jellyfin::models::{
    AuthRequest, AuthResponse, Item, ItemsResponse, PlaybackReport, View,
};
use anyhow::{anyhow, Context, Result};

const CLIENT_NAME: &str = "jellyfin-tui";
const DEVICE_NAME: &str = "rust-tui";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct JellyfinClient {
    http: reqwest::Client,
    server_url: String,
    device_id: String,
    access_token: Option<String>,
    user_id: Option<String>,
}

impl JellyfinClient {
    pub fn new(server_url: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            server_url: server_url.into(),
            device_id: device_id.into(),
            access_token: None,
            user_id: None,
        }
    }

    /// Reuses a token from a previous login instead of authenticating
    /// again. The caller (main.rs) is responsible for confirming it still
    /// works — a token can be revoked server-side at any time — typically
    /// by trying a real request and falling back to a fresh login on 401.
    pub fn restore_session(&mut self, access_token: String, user_id: String) {
        self.access_token = Some(access_token);
        self.user_id = Some(user_id);
    }

    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    /// Jellyfin wants this header on every request so it knows which client
    /// is talking to it. Before login it's the *only* form of identification
    /// (there's no token yet); after login we additionally send X-Emby-Token.
    fn auth_header_value(&self) -> String {
        let device_id = &self.device_id;
        format!(
            r#"MediaBrowser Client="{CLIENT_NAME}", Device="{DEVICE_NAME}", DeviceId="{device_id}", Version="{CLIENT_VERSION}""#
        )
    }

    /// Logs in and stores the access token + user id for subsequent calls.
    pub async fn authenticate(&mut self, username: &str, password: &str) -> Result<()> {
        let url = format!("{}/Users/AuthenticateByName", self.server_url);

        let resp = self
            .http
            .post(&url)
            .header("X-Emby-Authorization", self.auth_header_value())
            .json(&AuthRequest {
                username: username.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "authentication failed: server returned {}",
                resp.status()
            ));
        }

        let auth: AuthResponse = resp.json().await?;
        self.user_id = Some(auth.user.id);
        self.access_token = Some(auth.access_token);
        Ok(())
    }

    /// Fetches the top-level libraries (Movies, TV Shows, Music, ...) for the
    /// logged-in user.
    pub async fn get_views(&self) -> Result<Vec<View>> {
        let token = self
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;

        let url = format!("{}/Users/{}/Views", self.server_url, user_id);

        let resp = self
            .http
            .get(&url)
            .header("X-Emby-Token", token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "fetching views failed: server returned {}",
                resp.status()
            ));
        }

        let body: ItemsResponse<View> = resp.json().await?;
        Ok(body.items)
    }

    /// Fetches what's inside a given library or folder (pass a View's id, or
    /// a folder Item's id to go one level deeper).
    ///
    /// Jellyfin's TV and movie views are logical collections, not physical
    /// folders. At those library roots, asking only for direct children leaks
    /// loose episodes and release directories into the UI. Request the
    /// canonical recursive item type there; below the library root, preserve
    /// the normal Series -> Season -> Episode hierarchy with direct children.
    pub async fn get_items(
        &self,
        parent_id: &str,
        collection_type: Option<&str>,
    ) -> Result<Vec<Item>> {
        let token = self
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;
        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;

        let url = format!("{}/Items", self.server_url);

        let mut query = vec![
            ("ParentId", parent_id),
            ("UserId", user_id.as_str()),
            ("Fields", "UserData"),
        ];
        if let Some(item_type) = logical_library_item_type(collection_type) {
            query.push(("Recursive", "true"));
            query.push(("IncludeItemTypes", item_type));
        }

        let resp = self
            .http
            .get(&url)
            .header("X-Emby-Token", token)
            .query(&query)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "fetching items failed: server returned {}",
                resp.status()
            ));
        }

        let body: ItemsResponse<Item> = resp.json().await?;
        Ok(body.items)
    }

    /// Builds a direct-play stream URL for the given item.
    ///
    /// This is the simplest possible playback path: it skips Jellyfin's
    /// full PlaybackInfo negotiation, which is what lets the server choose
    /// between direct play and transcoding based on what the client can
    /// actually decode. A "real" client calls POST /Items/{id}/PlaybackInfo
    /// first and plays whatever MediaSource comes back, so playback still
    /// works when direct play isn't possible (wrong codec/container, remote
    /// network too slow, etc). Fine for testing direct play on your own
    /// network for now; transcoding fallback is a later addition.
    pub fn stream_url(&self, item_id: &str) -> Result<String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;

        Ok(format!(
            "{}/Videos/{}/stream?static=true&api_key={}",
            self.server_url, item_id, token
        ))
    }

    /// Fetches full details for a single item, including its MediaStreams
    /// (subtitle/audio/video tracks). The list endpoints don't include this
    /// by default — it'd be wasteful to pull for every row in a browse
    /// list — so this is called on-demand right before playback.
    pub async fn get_item(&self, item_id: &str) -> Result<Item> {
        let (token, user_id) = self.require_auth()?;
        let url = format!("{}/Users/{}/Items/{}", self.server_url, user_id, item_id);

        let resp = self
            .http
            .get(&url)
            .header("X-Emby-Token", token)
            .query(&[("Fields", "MediaStreams")])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "fetching item details failed: server returned {}",
                resp.status()
            ));
        }

        Ok(resp.json().await?)
    }

    /// Marks an item watched. Jellyfin's dedicated endpoint for this — not
    /// something we build ourselves out of a generic PATCH.
    pub async fn mark_played(&self, item_id: &str) -> Result<()> {
        let (token, user_id) = self.require_auth()?;
        let url = format!(
            "{}/Users/{}/PlayedItems/{}",
            self.server_url, user_id, item_id
        );

        let resp = self
            .http
            .post(&url)
            .header("X-Emby-Token", token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "marking item played failed: server returned {}",
                resp.status()
            ));
        }
        Ok(())
    }

    /// Marks an item unwatched (same endpoint, DELETE instead of POST).
    pub async fn mark_unplayed(&self, item_id: &str) -> Result<()> {
        let (token, user_id) = self.require_auth()?;
        let url = format!(
            "{}/Users/{}/PlayedItems/{}",
            self.server_url, user_id, item_id
        );

        let resp = self
            .http
            .delete(&url)
            .header("X-Emby-Token", token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "marking item unplayed failed: server returned {}",
                resp.status()
            ));
        }
        Ok(())
    }

    pub async fn report_playback_started(&self, item_id: &str, position_ticks: i64) -> Result<()> {
        self.report_playback("/Sessions/Playing", item_id, position_ticks, false)
            .await
    }

    pub async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: i64,
        is_paused: bool,
    ) -> Result<()> {
        self.report_playback(
            "/Sessions/Playing/Progress",
            item_id,
            position_ticks,
            is_paused,
        )
        .await
    }

    pub async fn report_playback_stopped(&self, item_id: &str, position_ticks: i64) -> Result<()> {
        self.report_playback("/Sessions/Playing/Stopped", item_id, position_ticks, false)
            .await
    }

    async fn report_playback(
        &self,
        endpoint: &str,
        item_id: &str,
        position_ticks: i64,
        is_paused: bool,
    ) -> Result<()> {
        let (token, _) = self.require_auth()?;
        let url = format!("{}{}", self.server_url, endpoint);
        let report = PlaybackReport {
            item_id,
            position_ticks,
            is_paused,
            is_muted: false,
            can_seek: true,
            play_method: "DirectPlay",
        };

        let resp = self
            .http
            .post(&url)
            .header("X-Emby-Token", token)
            .json(&report)
            .send()
            .await
            .with_context(|| format!("reporting playback state to {url}"))?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "reporting playback state to {url} failed: server returned {}",
                resp.status()
            ));
        }
        Ok(())
    }

    /// Small helper: both mark_played/mark_unplayed need the same "are we
    /// logged in" check that get_views/get_items already repeat inline —
    /// consolidating it here since we're adding two more call sites.
    fn require_auth(&self) -> Result<(&str, &str)> {
        let token = self
            .access_token
            .as_deref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;
        let user_id = self
            .user_id
            .as_deref()
            .ok_or_else(|| anyhow!("not authenticated yet — call authenticate() first"))?;
        Ok((token, user_id))
    }
}

/// The semantic item type Jellyfin clients should request at a logical
/// library root. Other collection types (notably home videos) retain their
/// physical folder hierarchy and therefore use the direct-children query.
fn logical_library_item_type(collection_type: Option<&str>) -> Option<&'static str> {
    match collection_type {
        Some("tvshows") => Some("Series"),
        Some("movies") => Some("Movie"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::logical_library_item_type;

    #[test]
    fn logical_library_roots_map_to_canonical_item_types() {
        assert_eq!(logical_library_item_type(Some("tvshows")), Some("Series"));
        assert_eq!(logical_library_item_type(Some("movies")), Some("Movie"));
        assert_eq!(logical_library_item_type(Some("homevideos")), None);
        assert_eq!(logical_library_item_type(None), None);
    }
}
