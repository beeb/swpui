use rat_widget::{text::HasScreenCursor as _, text_input::TextInput};
use ratatui::{
    Frame,
    buffer::CellWidth as _,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize as _},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, StatefulWidget as _},
};

use crate::{
    app::App,
    replace::effective_replacement,
    types::{Pane, PaneAreas},
    ui::preview::Preview,
    utils::trim_end_to_width,
};

mod file_list;
pub mod preview;

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();

    // main layout: content area + error/status bar
    let [content_area, status_area, options_area, hints_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(app.status_message.is_some().into()),
        Constraint::Length(1),
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
    let [search_area, replace_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(3)]).areas(input_area);

    let visible = |shown: bool, area: Rect| if shown { area } else { Rect::default() };
    app.pane_areas = PaneAreas {
        search_input: visible(!app.filter_view, search_area),
        replace_input: visible(!app.filter_view, replace_area),
        include_input: visible(app.filter_view, search_area),
        exclude_input: visible(app.filter_view, replace_area),
        file_list: file_area,
        preview: right,
    };

    render_input_area(app, frame, search_area, replace_area);
    file_list::render(app, frame, file_area);
    render_preview(app, frame, right);
    render_status_bar(app, frame, status_area, hints_area);
    render_options_strip(app, frame, options_area);

    if app.confirm_apply_all {
        render_confirm_modal(frame, area);
    }
    if app.options_open {
        render_options_modal(app, frame, area);
    }
}

fn render_preview(app: &mut App, frame: &mut Frame, area: Rect) {
    let replacement = effective_replacement(app.replace_input.text(), app.options.match_mode);
    let file = app.results.get(app.selected_file());
    Preview {
        file,
        replacement: &replacement,
        mode: app.options.match_mode,
        focused: app.focused_pane == Pane::Preview,
        border_style: focused_border_style(Pane::Preview, app.focused_pane),
    }
    .render(area, frame.buffer_mut(), &mut app.preview);
}

fn focused_border_style(pane: Pane, current: Pane) -> Style {
    if pane == current {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().dim()
    }
}

fn render_input_area(app: &mut App, frame: &mut Frame, top_area: Rect, bottom_area: Rect) {
    if app.filter_view {
        render_filter_inputs(app, frame, top_area, bottom_area);
    } else {
        render_search_inputs(app, frame, top_area, bottom_area);
    }
}

fn render_filter_inputs(app: &mut App, frame: &mut Frame, include_area: Rect, exclude_area: Rect) {
    // include input
    app.include_input
        .focus
        .set(app.focused_pane == Pane::IncludeInput);
    let include_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(focused_border_style(Pane::IncludeInput, app.focused_pane))
        .title(format!(
            "\u{2500}[{}]\u{2500}Include globs",
            Pane::IncludeInput.digit()
        ));
    TextInput::new()
        .style(Style::default())
        .focus_style(Style::default())
        .invalid_style(Style::default().fg(Color::Red))
        .block(include_block)
        .render(include_area, frame.buffer_mut(), &mut app.include_input);
    if let Some((cx, cy)) = app.include_input.screen_cursor() {
        frame.set_cursor_position((cx, cy));
    }

    // exclude input
    app.exclude_input
        .focus
        .set(app.focused_pane == Pane::ExcludeInput);
    let exclude_block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(focused_border_style(Pane::ExcludeInput, app.focused_pane))
        .title(format!(
            "\u{2500}[{}]\u{2500}Exclude globs",
            Pane::ExcludeInput.digit()
        ));
    TextInput::new()
        .style(Style::default())
        .focus_style(Style::default())
        .invalid_style(Style::default().fg(Color::Red))
        .block(exclude_block)
        .render(exclude_area, frame.buffer_mut(), &mut app.exclude_input);
    if let Some((cx, cy)) = app.exclude_input.screen_cursor() {
        frame.set_cursor_position((cx, cy));
    }
}

fn render_search_inputs(app: &mut App, frame: &mut Frame, search_area: Rect, replace_area: Rect) {
    let mode_label = format!(
        "\u{2500}[{}]\u{2500}Search ({})",
        Pane::SearchInput.digit(),
        app.options.match_mode
    );

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
        .title(format!(
            "\u{2500}[{}]\u{2500}Replace",
            Pane::ReplaceInput.digit()
        ));
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
    let body = vec![
        Line::from("Apply all replacements?").centered(),
        Line::from(vec![
            Span::styled("y/enter", Style::default().fg(Color::Green)),
            Span::raw(" : "),
            Span::styled("n/esc", Style::default().fg(Color::Red)),
        ])
        .centered(),
    ];
    frame.render_widget(Paragraph::new(body), inner);
}

fn render_options_modal(app: &App, frame: &mut Frame, area: Rect) {
    let hidden = if app.options.include_hidden {
        "included"
    } else {
        "excluded"
    };
    let gitignored = if app.options.include_gitignored {
        "included"
    } else {
        "excluded"
    };

    let match_mode = app.options.match_mode.to_string();
    let rows: [(&str, &str, &str); 3] = [
        ("r", "Search mode  ", match_mode.as_str()),
        ("h", "Hidden files ", hidden),
        ("g", ".gitignore   ", gitignored),
    ];

    let row_width = rows
        .iter()
        .map(|(k, name, val)| k.len() + name.len() + val.len())
        .max()
        .unwrap_or(0)
        + 4;
    let close_hint = "esc/C-o/A-o: close";
    let inner_width_usize = row_width.max(close_hint.len());
    #[expect(clippy::cast_possible_truncation)]
    let inner_width = inner_width_usize as u16;
    let width = (inner_width + 4).min(area.width);
    #[expect(clippy::cast_possible_truncation)]
    let height = (rows.len() as u16 + 4).min(area.height); // rows + blank + hint + 2 borders
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal_area);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(Color::Cyan))
        .title("Options");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut body: Vec<Line> = rows
        .iter()
        .map(|(key, name, val)| {
            Line::from(vec![
                Span::styled(format!(" {key} "), Style::default().fg(Color::Blue)),
                Span::raw(format!(" {name} ")),
                Span::styled(*val, Style::default().fg(Color::Cyan)),
            ])
        })
        .collect();
    body.push(Line::raw(""));
    body.push(Line::from(Span::styled(close_hint, Style::default().fg(Color::Blue))).centered());
    frame.render_widget(Paragraph::new(body), inner);
}

fn render_options_strip(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![
        Span::raw("hidden: ").dim(),
        toggle_span(app.options.include_hidden),
        Span::raw(" | .gitignore: ").dim(),
        toggle_span(app.options.include_gitignored),
    ];
    let globs = &app.options.globs;
    if !globs.include.is_empty() {
        spans.push(Span::raw(" | include: ").dim());
        spans.push(Span::raw(globs.include.join(", ")).yellow());
    }
    if !globs.exclude.is_empty() {
        spans.push(Span::raw(" | exclude: ").dim());
        spans.push(Span::raw(globs.exclude.join(", ")).yellow());
    }
    frame.render_widget(trim_line(Line::from(spans), area.width), area);
}

fn toggle_span(active: bool) -> Span<'static> {
    if active {
        Span::raw("incl").green()
    } else {
        Span::raw("excl").red()
    }
}

/// Truncate a line to `width` columns, appending a dim ellipsis when content is cut off.
fn trim_line(line: Line<'static>, width: u16) -> Line<'static> {
    let total: u16 = line
        .spans
        .iter()
        .map(|s| s.content.as_ref().cell_width())
        .sum();
    if total <= width {
        return line;
    }
    let budget = width.saturating_sub(1); // reserve 1 col for the ellipsis
    let mut spans = Vec::new();
    let mut used: u16 = 0;
    for span in line.spans {
        let (kept, trimmed) = trim_end_to_width(&span.content, budget - used, false);
        used += kept.cell_width();
        if !kept.is_empty() {
            // re-apply style to new span
            spans.push(Span::styled(kept.to_owned(), span.style));
        }
        if trimmed {
            break;
        }
    }
    spans.push(Span::raw("\u{2026}").dim());
    Line::from(spans)
}

fn render_status_bar(app: &App, frame: &mut Frame, status_area: Rect, hints_area: Rect) {
    if let Some(msg) = &app.status_message {
        let msg = Line::from(Span::styled(msg.as_str(), Style::default().fg(Color::Red)));
        frame.render_widget(msg, status_area);
    }
    let hints = match app.focused_pane {
        Pane::SearchInput | Pane::ReplaceInput | Pane::IncludeInput | Pane::ExcludeInput => {
            "C-r/A-r: mode | C-g/A-g: globs | C-o/A-o: options | esc: file list | tab/S-tab: cycle | q/C-c: quit"
        }
        Pane::FileList => {
            "space/s: skip file | f: apply file | a: apply all | j/k: navigate | l/enter: preview | C-g/A-g: globs | C-o/A-o: options | tab/S-tab: cycle | q/C-c: quit"
        }
        Pane::Preview => {
            "space: skip | enter: apply match | s: skip file | f: apply file | j/k: navigate | h/esc: back | C-g/A-g: globs | C-o/A-o: options | tab/S-tab: cycle | q/C-c: quit"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elide_line_fits() {
        let line = trim_line(Line::from("short"), 10);
        assert_eq!(line.to_string(), "short");
    }

    #[test]
    fn elide_line_truncates_with_ellipsis() {
        let line = Line::from(vec![Span::raw("hello "), Span::raw("world")]);
        let elided = trim_line(line, 8);
        assert_eq!(elided.to_string(), "hello w\u{2026}");
    }

    #[test]
    fn elide_line_exact_fit() {
        let line = Line::from("exactly8");
        let elided = trim_line(line, 8);
        assert_eq!(elided.to_string(), "exactly8");
    }
}
