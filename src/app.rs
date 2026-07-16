use crate::jellyfin::JellyfinClient;
use crate::subtitle_prefs::SubtitlePrefs;
use anyhow::Result;
use ratatui::widgets::ListState;

/// Jellyfin reports resume position down to the second, but resuming
/// something you watched 8 seconds of isn't useful — below this many
/// ticks (100ns units; 10s = 10 * 10_000_000), treat it as unwatched.
const MIN_RESUME_TICKS: i64 = 100_000_000;

/// One thing the user can pick in the browser: a library, a folder, or a
/// playable item. Jellyfin's "Views" (libraries) and "Items" are different
/// API types with different fields, but from the browser's point of view
/// they're the same shape — something with a name you can open — so we
/// flatten both into this one type rather than juggling two.
pub struct Entry {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    pub kind_label: String,
    /// Watch state. Always `false`/`None` for folders — Jellyfin doesn't
    /// really track "watched" on a folder the way it does on a video.
    pub played: bool,
    pub played_percentage: Option<f64>,
    pub playback_position_ticks: Option<i64>,
}

/// Where we currently are in the browsing tree.
/// `parent_id: None` means "the root" — show top-level libraries via
/// get_views() instead of get_items().
struct Level {
    title: String,
    parent_id: Option<String>,
}

/// One choice on the subtitle-selection screen.
pub struct SubtitleOption {
    pub label: String,
    /// What to pass mpv's `--sid`. `None` means "no subtitles"
    /// (`--sid=no`) — always the first option in the list.
    pub subtitle_index: Option<i64>,
    /// The stream's stable Jellyfin index, for persisting the choice.
    /// `None` for the synthetic "No subtitles" option.
    absolute_index: Option<i64>,
    is_default_choice: bool,
}

/// Everything needed to actually launch mpv, gathered by activate() before
/// handing control back to main.rs.
pub struct PlaybackRequest {
    pub url: String,
    pub title: String,
    pub start_seconds: Option<u64>,
    pub subtitle_index: Option<i64>,
}

/// What the browsing screen is currently doing. Kept as an enum rather
/// than a handful of separate bools/Options, so it's structurally
/// impossible to be "browsing" and "picking a subtitle" at once.
pub enum Mode {
    Browsing,
    SelectingSubtitle {
        options: Vec<SubtitleOption>,
        list_state: ListState,
        // Already resolved before we got here — just waiting on the
        // subtitle choice to build the final PlaybackRequest.
        url: String,
        title: String,
        start_seconds: Option<u64>,
        item_id: String,
    },
}

/// What activate() actually accomplished — since it can mean three
/// different things depending on what was selected.
pub enum ActivateOutcome {
    /// Nothing to do (list was empty, or we just descended into a folder —
    /// app state already updated, nothing for main.rs to act on).
    Nothing,
    /// Entered subtitle-selection mode; main.rs just keeps looping, the
    /// next draw() will show it.
    SelectingSubtitle,
    /// Ready to hand straight to mpv (no subtitle tracks to choose from).
    Play(PlaybackRequest),
}

pub struct App {
    client: JellyfinClient,
    stack: Vec<Level>,
    pub entries: Vec<Entry>,
    pub list_state: ListState,
    pub status: Option<String>,
    pub mode: Mode,
    subtitle_prefs: SubtitlePrefs,
}

impl App {
    pub async fn new(client: JellyfinClient) -> Result<Self> {
        let mut app = Self {
            client,
            stack: vec![Level {
                title: "Libraries".to_string(),
                parent_id: None,
            }],
            entries: Vec::new(),
            list_state: ListState::default(),
            status: None,
            mode: Mode::Browsing,
            subtitle_prefs: SubtitlePrefs::load(),
        };
        app.refresh(false).await?;
        Ok(app)
    }

    pub fn breadcrumb(&self) -> String {
        self.stack
            .iter()
            .map(|l| l.title.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Re-fetches entries for whatever level is currently on top of the
    /// stack.
    ///
    /// `keep_selection`: true preserves the current index (used when the
    /// underlying data changed but we're still looking at the *same* list —
    /// e.g. after toggling watched status); false resets to the top (used
    /// when we've actually moved to a different level via activate()/
    /// go_back()).
    async fn refresh(&mut self, keep_selection: bool) -> Result<()> {
        let level = self.stack.last().expect("stack is never empty");
        let previous_selection = self.list_state.selected();

        self.entries = match &level.parent_id {
            None => {
                let views = self.client.get_views().await?;
                views
                    .into_iter()
                    .map(|v| Entry {
                        id: v.id,
                        name: v.name,
                        is_folder: true,
                        kind_label: v.collection_type.unwrap_or_else(|| "unknown type".to_string()),
                        played: false,
                        played_percentage: None,
                        playback_position_ticks: None,
                    })
                    .collect()
            }
            Some(parent_id) => {
                let items = self.client.get_items(parent_id).await?;
                items
                    .into_iter()
                    .map(|it| {
                        let played = it.user_data.as_ref().map(|u| u.played).unwrap_or(false);
                        let played_percentage =
                            it.user_data.as_ref().and_then(|u| u.played_percentage);
                        let playback_position_ticks = it
                            .user_data
                            .as_ref()
                            .and_then(|u| u.playback_position_ticks);
                        Entry {
                            id: it.id,
                            name: it.name,
                            is_folder: it.is_folder,
                            kind_label: it.item_type,
                            played,
                            played_percentage,
                            playback_position_ticks,
                        }
                    })
                    .collect()
            }
        };

        let selection = if keep_selection {
            previous_selection.filter(|&idx| idx < self.entries.len())
        } else {
            None
        }
        .or(if self.entries.is_empty() { None } else { Some(0) });

        self.list_state.select(selection);
        Ok(())
    }

    /// Moves the selection by `delta` (positive = down, negative = up),
    /// wrapping around at either end.
    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        let current = self.list_state.selected().unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len);
        self.list_state.select(Some(next as usize));
    }

    /// Enter is context-dependent: descending into a folder updates our own
    /// state and returns Nothing; picking a playable item resolves a resume
    /// position (unless `force_restart`) and its subtitle tracks. If a
    /// subtitle preference is already remembered for this item and
    /// `force_subtitle_picker` isn't set, it's applied silently; otherwise
    /// this enters subtitle-selection mode, or returns a ready-to-play
    /// PlaybackRequest directly if there's nothing to choose between.
    pub async fn activate(
        &mut self,
        force_restart: bool,
        force_subtitle_picker: bool,
    ) -> Result<ActivateOutcome> {
        let Some(idx) = self.list_state.selected() else {
            return Ok(ActivateOutcome::Nothing);
        };
        let entry = &self.entries[idx];

        if entry.is_folder {
            let title = entry.name.clone();
            let id = entry.id.clone();
            self.stack.push(Level {
                title,
                parent_id: Some(id),
            });
            self.refresh(false).await?;
            return Ok(ActivateOutcome::Nothing);
        }

        let url = self.client.stream_url(&entry.id)?;
        let title = entry.name.clone();
        let item_id = entry.id.clone();

        let start_seconds = if force_restart {
            None
        } else {
            entry
                .playback_position_ticks
                .filter(|&ticks| ticks > MIN_RESUME_TICKS)
                .map(|ticks| (ticks / 10_000_000) as u64)
        };

        let item = self.client.get_item(&item_id).await?;
        let subtitle_streams: Vec<_> = item
            .media_streams
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.stream_type == "Subtitle")
            .collect();

        if subtitle_streams.is_empty() {
            return Ok(ActivateOutcome::Play(PlaybackRequest {
                url,
                title,
                start_seconds,
                subtitle_index: None,
            }));
        }

        if !force_subtitle_picker {
            if let Some(remembered) = self.subtitle_prefs.get(&item_id) {
                match remembered {
                    // Remembered as off.
                    None => {
                        return Ok(ActivateOutcome::Play(PlaybackRequest {
                            url,
                            title,
                            start_seconds,
                            subtitle_index: None,
                        }));
                    }
                    // Remembered a specific stream — find its current
                    // ordinal position among subtitle streams. If it's no
                    // longer there (file got rescanned, tracks changed),
                    // fall through to the picker instead of guessing.
                    Some(absolute_index) => {
                        if let Some(pos) = subtitle_streams
                            .iter()
                            .position(|s| s.index == absolute_index)
                        {
                            return Ok(ActivateOutcome::Play(PlaybackRequest {
                                url,
                                title,
                                start_seconds,
                                subtitle_index: Some((pos + 1) as i64),
                            }));
                        }
                    }
                }
            }
        }

        let mut options = vec![SubtitleOption {
            label: "No subtitles".to_string(),
            subtitle_index: None,
            absolute_index: None,
            is_default_choice: false,
        }];

        // mpv numbers --sid among subtitle-type streams only (1, 2, 3, ...)
        // when handed the raw file directly — not Jellyfin's absolute
        // container-wide stream Index — so the ordinal position here,
        // not stream.index, is what we pass to mpv.
        for (i, stream) in subtitle_streams.iter().enumerate() {
            let mpv_sid = (i + 1) as i64;
            let label = stream
                .display_title
                .clone()
                .or_else(|| stream.language.clone())
                .unwrap_or_else(|| format!("Track {}", stream.index));
            options.push(SubtitleOption {
                label,
                subtitle_index: Some(mpv_sid),
                absolute_index: Some(stream.index),
                is_default_choice: stream.is_default,
            });
        }

        let default_selected = options.iter().position(|o| o.is_default_choice).unwrap_or(0);
        let mut list_state = ListState::default();
        list_state.select(Some(default_selected));

        self.mode = Mode::SelectingSubtitle {
            options,
            list_state,
            url,
            title,
            start_seconds,
            item_id,
        };
        Ok(ActivateOutcome::SelectingSubtitle)
    }

    /// Moves the cursor on the subtitle-selection screen. No-op outside
    /// that mode.
    pub fn move_subtitle_selection(&mut self, delta: isize) {
        if let Mode::SelectingSubtitle {
            options, list_state, ..
        } = &mut self.mode
        {
            if options.is_empty() {
                return;
            }
            let len = options.len() as isize;
            let current = list_state.selected().unwrap_or(0) as isize;
            let next = (current + delta).rem_euclid(len);
            list_state.select(Some(next as usize));
        }
    }

    /// Confirms the highlighted subtitle choice, persists it for next time,
    /// builds the final PlaybackRequest, and returns to Browsing mode.
    /// Returns None if we somehow weren't in subtitle-selection mode.
    pub fn confirm_subtitle_selection(&mut self) -> Option<PlaybackRequest> {
        let mode = std::mem::replace(&mut self.mode, Mode::Browsing);
        match mode {
            Mode::SelectingSubtitle {
                options,
                list_state,
                url,
                title,
                start_seconds,
                item_id,
            } => {
                let idx = list_state.selected().unwrap_or(0);
                let chosen = options.get(idx);
                let subtitle_index = chosen.and_then(|o| o.subtitle_index);
                let absolute_index = chosen.and_then(|o| o.absolute_index);

                if let Err(e) = self.subtitle_prefs.set(&item_id, absolute_index) {
                    self.status = Some(format!("Couldn't save subtitle preference: {e:#}"));
                }

                Some(PlaybackRequest {
                    url,
                    title,
                    start_seconds,
                    subtitle_index,
                })
            }
            other => {
                self.mode = other;
                None
            }
        }
    }

    /// Backs out of subtitle-selection without playing anything.
    pub fn cancel_subtitle_selection(&mut self) {
        self.mode = Mode::Browsing;
    }

    pub async fn go_back(&mut self) -> Result<()> {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.refresh(false).await?;
        }
        Ok(())
    }

    /// Toggles watched/unwatched on the selected item. No-ops (with a
    /// status message) on folders, since Jellyfin doesn't track that at
    /// the folder level the way it does for an actual video.
    pub async fn toggle_played(&mut self) -> Result<()> {
        let Some(idx) = self.list_state.selected() else {
            return Ok(());
        };
        let entry = &self.entries[idx];

        if entry.is_folder {
            self.status = Some("Can't mark a folder watched/unwatched".to_string());
            return Ok(());
        }

        let name = entry.name.clone();
        let id = entry.id.clone();
        let now_played = !entry.played;

        if now_played {
            self.client.mark_played(&id).await?;
        } else {
            self.client.mark_unplayed(&id).await?;
        }

        // Re-fetch so we're showing the server's actual state rather than
        // just assuming our request landed — but stay on the same row
        // rather than jumping back to the top of the list.
        self.refresh(true).await?;

        self.status = Some(format!(
            "Marked \"{name}\" as {}",
            if now_played { "watched" } else { "unwatched" }
        ));
        Ok(())
    }
}
