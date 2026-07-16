use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Remembers which subtitle track (if any) you picked for each item, so a
/// foreign-language film only needs choosing once. Keyed by Jellyfin item
/// id; value is the subtitle stream's *absolute* MediaStreams index (not
/// mpv's --sid ordinal, which we recompute fresh each time from whatever
/// the current stream list looks like — the absolute index is the stable
/// part). `Some(None)` in the map means "remembered as off"; the key being
/// absent entirely means "nothing recorded yet," so the picker still
/// appears on first play.
#[derive(Debug, Default)]
pub struct SubtitlePrefs {
    choices: HashMap<String, Option<i64>>,
}

impl SubtitlePrefs {
    fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "jellyfin-tui")
            .ok_or_else(|| anyhow!("could not determine a data directory for this platform"))?;
        Ok(dirs.data_dir().join("subtitle_prefs.json"))
    }

    /// Falls back to an empty (no preferences remembered yet) set rather
    /// than failing the whole app if this file is missing or corrupt — a
    /// broken local cache shouldn't block startup.
    pub fn load() -> Self {
        Self::try_load().unwrap_or_default()
    }

    fn try_load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading subtitle preferences at {}", path.display()))?;
        let choices = serde_json::from_str(&text)
            .with_context(|| format!("parsing subtitle preferences at {}", path.display()))?;
        Ok(Self { choices })
    }

    fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(&self.choices)?;
        fs::write(&path, text)
            .with_context(|| format!("writing subtitle preferences to {}", path.display()))
    }

    /// `None` = nothing recorded for this item yet (show the picker).
    /// `Some(None)` = remembered as off. `Some(Some(idx))` = remembered
    /// stream, by absolute index.
    pub fn get(&self, item_id: &str) -> Option<Option<i64>> {
        self.choices.get(item_id).copied()
    }

    /// Records a choice and persists it immediately — this is the only
    /// place preferences get written, so every pick from the selection
    /// screen (whether it appeared automatically or was forced open) ends
    /// up remembered, overwriting whatever was there before.
    pub fn set(&mut self, item_id: &str, absolute_index: Option<i64>) -> Result<()> {
        self.choices.insert(item_id.to_string(), absolute_index);
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_reflects_a_saved_choice_without_touching_disk() {
        // Doesn't exercise the real file path (that's a manual/integration
        // concern given XDG dirs), just the in-memory get/set contract
        // confirm_subtitle_selection() actually relies on.
        let mut prefs = SubtitlePrefs::default();
        assert_eq!(prefs.get("item-1"), None); // nothing recorded yet
        prefs.choices.insert("item-1".to_string(), Some(3));
        assert_eq!(prefs.get("item-1"), Some(Some(3)));
        prefs.choices.insert("item-1".to_string(), None); // remembered "off"
        assert_eq!(prefs.get("item-1"), Some(None));
    }
}
