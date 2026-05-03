use rat_widget::{text::HasScreenCursor as _, text_input::TextInput};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize as _},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, StatefulWidget as _},
};

use crate::{
    app::App,
    types::{MatchMode, Pane},
};

mod file_list;
mod preview;

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();

    // main layout: content area + error/status bar
    let [content_area, status_area, hints_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(app.status_message.is_some().into()),
        Constraint::Length(1),
    ])
    .areas(area);

    // split content into left and right columns
    // shrink the file list when the preview pane is focused
    let left_size = if app.focused_pane == Pane::Preview {
        let target = content_area.width / 5;
        Constraint::Length(target.max(10))
    } else {
        let target = content_area.width / 2;
        Constraint::Length(target.min(50))
    };
    let [left, right] = Layout::horizontal([left_size, Constraint::Fill(1)]).areas(content_area);

    // left column: input area + file list
    let [input_area, file_area] =
        Layout::vertical([Constraint::Length(6), Constraint::Fill(1)]).areas(left);

    render_input_area(app, frame, input_area);
    file_list::render(app, frame, file_area);
    preview::render(app, frame, right);
    render_status_bar(app, frame, status_area, hints_area);

    if app.confirm_apply_all {
        render_confirm_modal(frame, area);
    }
}

fn focused_border_style(pane: Pane, current: Pane) -> Style {
    if pane == current {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().dim()
    }
}

fn render_input_area(app: &mut App, frame: &mut Frame, area: Rect) {
    let [search_area, replace_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(3)]).areas(area);

    let hidden_label = if app.include_hidden {
        " [+hidden]"
    } else {
        " [-hidden]"
    };
    let mode_label = match app.match_mode {
        MatchMode::CaseAware => format!("Search (case-aware){hidden_label}"),
        MatchMode::Literal => format!("Search (literal){hidden_label}"),
        MatchMode::Regex => format!("Search (regex){hidden_label}"),
        MatchMode::RegexMultiline => format!("Search (regex multiline){hidden_label}"),
    };

    // search input
    app.search_input
        .focus
        .set(app.focused_pane == Pane::SearchInput);
    let search_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(focused_border_style(Pane::SearchInput, app.focused_pane))
        .title(mode_label);
    TextInput::new()
        .style(Style::default())
        .focus_style(Style::default())
        .invalid_style(Style::default().fg(Color::Red))
        .block(search_block)
        .render(search_area, frame.buffer_mut(), &mut app.search_input);
    if let Some((cx, cy)) = app.search_input.screen_cursor() {
        frame.set_cursor_position((cx, cy));
    }

    // replace input
    app.replace_input
        .focus
        .set(app.focused_pane == Pane::ReplaceInput);
    let replace_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(focused_border_style(Pane::ReplaceInput, app.focused_pane))
        .title("Replace");
    TextInput::new()
        .style(Style::default())
        .focus_style(Style::default())
        .block(replace_block)
        .render(replace_area, frame.buffer_mut(), &mut app.replace_input);
    if let Some((cx, cy)) = app.replace_input.screen_cursor() {
        frame.set_cursor_position((cx, cy));
    }
}

fn render_confirm_modal(frame: &mut Frame, area: Rect) {
    let width = 30u16.min(area.width);
    let height = 4u16.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal_area);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(Color::Yellow))
        .title("Confirm");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    frame.render_widget(
        Paragraph::new("Apply all replacements?\ny / n")
            .alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

fn render_status_bar(app: &App, frame: &mut Frame, status_area: Rect, hints_area: Rect) {
    if let Some(msg) = &app.status_message {
        let msg = Line::from(Span::styled(msg.as_str(), Style::default().fg(Color::Red)));
        frame.render_widget(msg, status_area);
    }
    let hints = match app.focused_pane {
        Pane::SearchInput | Pane::ReplaceInput => {
            "C-r/A-r: mode | C-d/A-d: hidden | esc: file list | tab/S-tab: cycle | q/C-c: quit"
        }
        Pane::FileList => {
            "s: skip file | f: apply file | a: apply all | j/k: navigate | l/enter: preview | tab/S-tab: cycle | q/C-c: quit"
        }
        Pane::Preview => {
            "space: skip | enter: apply match | s: skip file | f: apply file | j/k: navigate | h/esc: back | tab/S-tab: cycle | q/C-c: quit"
        }
    };
    let version = concat!("v", env!("CARGO_PKG_VERSION"));
    // reserve an extra column so the version doesn't sit flush against the hints
    #[expect(clippy::cast_possible_truncation)]
    let version_width = (version.len() + 1) as u16;
    let [hints_area, version_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(version_width)])
            .areas(hints_area);
    frame.render_widget(Line::from(hints.blue()), hints_area);
    frame.render_widget(Line::from(version.dim()).right_aligned(), version_area);
}
