mod app;
mod config;
mod jellyfin;
mod mpv;
mod subtitle_prefs;
mod ui;

use anyhow::Result;
use app::{App, Mode};
use config::Config;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use jellyfin::JellyfinClient;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::process::ExitStatus;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = Config::load_or_create()?;
    let device_id = config.device_id()?;

    // Server URL: use the saved one, or ask and remember it.
    let server_url = match &config.server.url {
        Some(url) => url.clone(),
        None => {
            let url = prompt("Jellyfin server URL (e.g. http://localhost:8096)")?
                .trim()
                .trim_end_matches('/')
                .to_string();
            config.server.url = Some(url.clone());
            config.save()?;
            url
        }
    };

    let mut client = JellyfinClient::new(server_url, device_id);

    // Try a saved session first — restore_session() just sets the fields,
    // it doesn't confirm the token still works, so we treat the very first
    // real request (get_views, inside App::new) as the actual validity
    // check and fall back to a fresh login if it fails.
    let have_saved_session = match (&config.server.access_token, &config.server.user_id) {
        (Some(token), Some(user_id)) => {
            client.restore_session(token.clone(), user_id.clone());
            true
        }
        _ => false,
    };

    let mut app = if have_saved_session {
        match App::new(client).await {
            Ok(app) => app,
            Err(_) => {
                // Saved token didn't work (revoked, expired, wrong server
                // since saved, ...) — fall through to a fresh login below.
                let mut client = JellyfinClient::new(
                    config.server.url.clone().unwrap(),
                    config.server.device_id.clone().unwrap(),
                );
                login_and_save(&mut client, &mut config).await?;
                App::new(client).await?
            }
        }
    } else {
        login_and_save(&mut client, &mut config).await?;
        App::new(client).await?
    };

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run(&mut terminal, &mut app).await;

    // Always try to restore the terminal, even if `run` returned an error —
    // otherwise a crash mid-browse leaves the user's shell in raw mode /
    // the alternate screen, which is a genuinely bad experience to debug.
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

/// Prompts for username/password, authenticates, and persists the resulting
/// token (not the password) plus username for next time.
async fn login_and_save(client: &mut JellyfinClient, config: &mut Config) -> Result<()> {
    let username = prompt("Username")?;
    let password = prompt("Password")?;

    client
        .authenticate(username.trim(), password.trim())
        .await?;

    config.server.username = Some(username.trim().to_string());
    config.server.access_token = client.access_token().map(str::to_string);
    config.server.user_id = client.user_id().map(str::to_string);
    config.save()?;

    Ok(())
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Poll with a timeout rather than blocking forever on input — not
        // strictly needed yet (nothing else is happening), but it's the
        // shape this loop will need once mpv/sway events join it later.
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        // Crossterm reports both press and release on some platforms;
        // only act on press or we'd double-handle every keystroke.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if matches!(app.mode, Mode::SelectingSubtitle { .. }) {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.move_subtitle_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_subtitle_selection(1),
                KeyCode::Enter => {
                    if let Some(request) = app.confirm_subtitle_selection() {
                        app.status = match play_and_report(app, &request).await {
                            Ok(()) => None,
                            Err(e) => Some(format!("Playback error: {e:#}")),
                        };
                    }
                }
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('q') => {
                    app.cancel_subtitle_selection();
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
            KeyCode::Backspace | KeyCode::Left => {
                if let Err(e) = app.go_back().await {
                    app.status = Some(format!("Error: {e:#}"));
                }
            }
            KeyCode::Char('m') => {
                if let Err(e) = app.toggle_played().await {
                    app.status = Some(format!("Error: {e:#}"));
                }
            }
            KeyCode::Enter => handle_activate(app, false, false).await,
            KeyCode::Char('R') => handle_activate(app, true, false).await,
            KeyCode::Char('S') => handle_activate(app, false, true).await,
            _ => {}
        }
    }
    Ok(())
}

/// Shared by Enter (resume-aware, remembered subtitle), Shift+R (force
/// restart), and Shift+S (force the subtitle picker open even if a
/// preference is already remembered) — they only differ in which bools
/// get passed to activate().
async fn handle_activate(app: &mut App, force_restart: bool, force_subtitle_picker: bool) {
    match app.activate(force_restart, force_subtitle_picker).await {
        Ok(app::ActivateOutcome::Play(request)) => {
            // mpv owns its own window (it never touches our tty), so no
            // raw-mode juggling is needed here — just pause this loop
            // until it exits, then keep browsing.
            app.status = match play_and_report(app, &request).await {
                Ok(()) => None,
                Err(e) => Some(format!("Playback error: {e:#}")),
            };
        }
        Ok(app::ActivateOutcome::SelectingSubtitle) => {}
        Ok(app::ActivateOutcome::Nothing) => {}
        Err(e) => app.status = Some(format!("Error: {e:#}")),
    }
}

async fn play_and_report(app: &mut App, request: &app::PlaybackRequest) -> Result<()> {
    const REPORT_INTERVAL_SECONDS: u64 = 10;

    let mut player = mpv::Player::start(request)?;
    let mut position_ticks = request.start_seconds.unwrap_or(0) as i64 * 10_000_000;
    let mut is_paused = false;
    let mut seconds_since_report = 0;
    let mut reporting_error = app
        .report_playback_started(&request.item_id, position_ticks)
        .await
        .err();

    let exit_status: ExitStatus = loop {
        if let Some(status) = player.try_wait()? {
            break status;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(state) = player.playback_state() {
            position_ticks = state.position_ticks;
            is_paused = state.is_paused;
        }

        seconds_since_report += 1;
        if seconds_since_report >= REPORT_INTERVAL_SECONDS {
            reporting_error = app
                .report_playback_progress(&request.item_id, position_ticks, is_paused)
                .await
                .err();
            seconds_since_report = 0;
        }
    };

    if let Err(error) = app
        .report_playback_stopped(&request.item_id, position_ticks)
        .await
    {
        reporting_error = Some(error);
    }

    // The server's response is authoritative; re-fetch rather than
    // manufacturing a local percentage from mpv's position.
    app.refresh_after_playback().await?;

    if !exit_status.success() {
        anyhow::bail!("mpv exited with a non-zero status: {exit_status}");
    }
    if let Some(error) = reporting_error {
        return Err(
            error.context("mpv played, but Jellyfin rejected at least one playback-state report")
        );
    }

    Ok(())
}

fn prompt(label: &str) -> io::Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input)
}
