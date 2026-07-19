use crate::app::{App, Mode};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.size());

    let breadcrumb = app.breadcrumb();

    match &mut app.mode {
        Mode::Browsing => draw_browser(
            frame,
            chunks[0],
            &app.entries,
            &mut app.list_state,
            &breadcrumb,
        ),
        Mode::SelectingSubtitle {
            options,
            list_state,
            ..
        } => {
            let items: Vec<ListItem> = options
                .iter()
                .map(|o| ListItem::new(o.label.clone()))
                .collect();
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Choose subtitles"),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, chunks[0], list_state);
        }
    }

    let help = match &app.mode {
        Mode::Browsing => app.status.clone().unwrap_or_else(|| {
            "\u{2191}/\u{2193} or j/k: move   Enter: play   R: restart   S: subtitles   m: watched   Backspace: back   q: quit"
                .to_string()
        }),
        Mode::SelectingSubtitle { .. } => {
            "\u{2191}/\u{2193} or j/k: move   Enter: confirm   Esc/Backspace: cancel".to_string()
        }
    };
    frame.render_widget(Paragraph::new(Line::from(help)), chunks[1]);
}

fn draw_browser(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    entries: &[crate::app::Entry],
    list_state: &mut ratatui::widgets::ListState,
    breadcrumb: &str,
) {
    let items: Vec<ListItem> = entries
        .iter()
        .map(|entry| {
            // Twemoji (or whatever emoji font is installed) renders this
            // automatically via font substitution — we just need to emit
            // the plain Unicode character, no image handling on our end.
            let watched_marker = if entry.is_folder {
                "  ".to_string()
            } else if entry.played {
                "\u{2705}".to_string() // checkmark
            } else {
                match entry.played_percentage {
                    Some(pct) if pct > 0.0 => format!("{pct:>2.0}%"),
                    _ => "  ".to_string(),
                }
            };

            let folder_marker = if entry.is_folder { "/" } else { " " };
            ListItem::new(format!("{watched_marker} {}{folder_marker}", entry.name))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(breadcrumb.to_string()),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, list_state);
}
