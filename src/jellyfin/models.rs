use serde::{Deserialize, Serialize};

/// What we send to POST /Users/AuthenticateByName
#[derive(Debug, Serialize)]
pub struct AuthRequest {
    #[serde(rename = "Username")]
    pub username: String,
    #[serde(rename = "Pw")]
    pub password: String,
}

/// What Jellyfin sends back on successful auth
#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    #[serde(rename = "AccessToken")]
    pub access_token: String,
    #[serde(rename = "User")]
    pub user: JellyfinUser,
}

#[derive(Debug, Deserialize)]
pub struct JellyfinUser {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
}

/// A single library/view (Movies, Music, TV Shows, ...)
#[derive(Debug, Deserialize)]
pub struct View {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "CollectionType")]
    pub collection_type: Option<String>,
}

/// Per-user playback state Jellyfin attaches to an item — whether it's
/// been watched, and how far into it you got if not.
#[derive(Debug, Deserialize)]
pub struct UserData {
    #[serde(rename = "Played")]
    pub played: bool,
    /// 0.0–100.0. Present only once there's some playback history.
    #[serde(rename = "PlayedPercentage")]
    pub played_percentage: Option<f64>,
    /// Jellyfin ticks (100ns units) into the item. Present only once
    /// there's some playback history.
    #[serde(rename = "PlaybackPositionTicks")]
    pub playback_position_ticks: Option<i64>,
}

/// One stream inside a media file — video, audio, or subtitle. We only
/// care about the subtitle ones today, but the container mixes all types
/// together so we filter by `stream_type` after deserializing rather than
/// modeling separate types.
#[derive(Debug, Deserialize)]
pub struct MediaStream {
    /// The stream's position within the *whole file* (video + audio +
    /// subtitles all numbered together). NOT the same numbering mpv uses
    /// for `--sid` when given the raw file directly — mpv numbers only
    /// among subtitle-type streams. Kept here for display purposes only;
    /// see app.rs for the actual --sid mapping.
    #[serde(rename = "Index")]
    pub index: i64,
    #[serde(rename = "Type")]
    pub stream_type: String,
    #[serde(rename = "DisplayTitle")]
    pub display_title: Option<String>,
    #[serde(rename = "Language")]
    pub language: Option<String>,
    #[serde(rename = "IsDefault")]
    pub is_default: bool,
}

/// A single item inside a library — could be a movie, an episode, a track,
/// or a subfolder (mixed-content libraries like "Media" nest folders inside
/// folders, hence `is_folder`).
#[derive(Debug, Deserialize)]
pub struct Item {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Type")]
    pub item_type: String,
    #[serde(rename = "IsFolder")]
    pub is_folder: bool,
    /// Absent entirely for folders/collections; present (with real values)
    /// for anything with per-user watch state, which is why this — unlike
    /// the fields above — has to be Optional.
    #[serde(rename = "UserData")]
    pub user_data: Option<UserData>,
    /// Only populated when we explicitly request `Fields=MediaStreams` —
    /// the list endpoints don't include this by default (would be wasteful
    /// for every row), so we fetch it on-demand via get_item() right when
    /// something is about to be played.
    #[serde(rename = "MediaStreams", default)]
    pub media_streams: Option<Vec<MediaStream>>,
}

/// Wrapper Jellyfin uses for basically every "list of items" endpoint
#[derive(Debug, Deserialize)]
pub struct ItemsResponse<T> {
    #[serde(rename = "Items")]
    pub items: Vec<T>,
    #[serde(rename = "TotalRecordCount")]
    pub total_record_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Can't hit Neil's real Jellyfin server from this sandbox, so these
    // pin down the JSON shape we're assuming against realistic payloads
    // instead — run with `cargo test`.

    #[test]
    fn item_with_watch_history_deserializes() {
        let json = r#"{
            "Id": "abc123",
            "Name": "Weapons",
            "Type": "Movie",
            "IsFolder": false,
            "UserData": {
                "Played": false,
                "PlayedPercentage": 42.5,
                "PlaybackPositionTicks": 12345670000
            }
        }"#;

        let item: Item = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(item.name, "Weapons");
        let user_data = item.user_data.expect("UserData should be present");
        assert!(!user_data.played);
        assert_eq!(user_data.played_percentage, Some(42.5));
    }

    #[test]
    fn item_without_watch_history_deserializes() {
        // No playback yet at all — Jellyfin still sends UserData, just
        // with Played:false and the percentage/position fields absent.
        let json = r#"{
            "Id": "def456",
            "Name": "Best Wishes to All",
            "Type": "Movie",
            "IsFolder": false,
            "UserData": { "Played": false }
        }"#;

        let item: Item = serde_json::from_str(json).expect("should deserialize");
        let user_data = item.user_data.expect("UserData should be present");
        assert!(!user_data.played);
        assert_eq!(user_data.played_percentage, None);
    }

    #[test]
    fn folder_without_user_data_field_at_all_deserializes() {
        // Some folder/collection responses omit UserData entirely.
        let json = r#"{
            "Id": "xyz789",
            "Name": "Media",
            "Type": "Folder",
            "IsFolder": true
        }"#;

        let item: Item = serde_json::from_str(json).expect("should deserialize");
        assert!(item.user_data.is_none());
    }

    #[test]
    fn item_with_media_streams_deserializes() {
        let json = r#"{
            "Id": "abc123",
            "Name": "Weapons",
            "Type": "Movie",
            "IsFolder": false,
            "MediaStreams": [
                {"Index": 0, "Type": "Video", "IsDefault": true},
                {"Index": 1, "Type": "Audio", "Language": "eng", "IsDefault": true},
                {"Index": 2, "Type": "Subtitle", "DisplayTitle": "English", "Language": "eng", "IsDefault": true},
                {"Index": 3, "Type": "Subtitle", "DisplayTitle": "Spanish", "Language": "spa", "IsDefault": false}
            ]
        }"#;

        let item: Item = serde_json::from_str(json).expect("should deserialize");
        let streams = item.media_streams.expect("MediaStreams should be present");
        let subtitle_count = streams.iter().filter(|s| s.stream_type == "Subtitle").count();
        assert_eq!(subtitle_count, 2);
    }
}

