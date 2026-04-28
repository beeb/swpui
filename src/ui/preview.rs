use std::ops::Range;

use rat_widget::scrolled::{Scroll, ScrollArea, ScrollAreaState};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget as _},
};

use super::focused_border_style;
use crate::{
    app::App,
    preview::data::{CONTEXT_LINES, ContextLine, PreviewData, PreviewMatch, PreviewMatchKind},
    replace::{case_aware_replacement, effective_replacement, expand_captures},
    types::{MatchInfo, MatchMode, Pane},
    utils::truncate_match_line,
};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let border_style = focused_border_style(Pane::Preview, app.focused_pane);
    let title = format_title(app, area.width);

    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(border_style)
        .title(title);

    let Some(fm) = app.results.get(app.selected_file()) else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        render_message(frame, inner, "Select a file", Color::DarkGray);
        return;
    };

    let path = fm.path.clone();

    if let Some(err) = app.preview_error.get(&path) {
        let inner = block.inner(area);
        let msg = err.clone();
        frame.render_widget(block, area);
        render_message(frame, inner, &msg, Color::Red);
        return;
    }

    let Some(preview) = app.preview_data.get(&path).cloned() else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = if app.preview_loading {
            "Loading preview\u{2026}"
        } else {
            ""
        };
        render_message(frame, inner, msg, Color::DarkGray);
        return;
    };

    let v_scroll = Scroll::vertical().style(border_style);
    let scroll_area = ScrollArea::new()
        .block(Some(&block))
        .v_scroll(Some(&v_scroll));
    let inner = scroll_area.inner(area, None, Some(&app.preview_scroll));

    let raw_replacement = app.replace_input.text();
    let replacement = effective_replacement(raw_replacement, app.match_mode);
    let is_preview_focused = app.focused_pane == Pane::Preview;
    let inner_height = inner.height as usize;

    // first pass: count total lines and find selected range cheaply
    let mut total_lines = 0;
    let mut selected_range: Range<usize> = 0..0;
    for (match_idx, (info, preview)) in fm.matches.iter().zip(preview.matches.iter()).enumerate() {
        if match_idx > 0 {
            total_lines += 1; // separator
        }
        let match_start = total_lines;
        total_lines += count_match_lines(info, preview, &replacement);
        if is_preview_focused && match_idx == app.selected_match {
            selected_range = match_start..total_lines;
        }
    }

    // compute how far we can scroll within the selected match
    let selected_height = selected_range.end - selected_range.start;
    app.preview_line_offset_max = selected_height.saturating_sub(inner_height);
    app.preview_line_offset = app.preview_line_offset.min(app.preview_line_offset_max);

    // set up scroll state with counted totals so we know the visible offset
    app.preview_scroll.set_page_len(inner_height);
    app.preview_scroll
        .set_max_offset(total_lines.saturating_sub(inner_height));
    app.preview_scroll.scroll_to_range(selected_range);

    // apply the extra line offset for tall matches
    if app.preview_line_offset > 0 {
        app.preview_scroll.scroll_down(app.preview_line_offset);
    }

    let offset = app.preview_scroll.offset;
    let visible_range = offset..offset + inner_height;

    // second pass: build lines, skipping expensive formatting outside visible range
    let lines = build_preview_lines(
        &fm.matches,
        &preview,
        &replacement,
        app.match_mode,
        is_preview_focused,
        app.selected_match,
        inner.width,
        visible_range,
    );

    let offset = app.preview_scroll.offset;

    scroll_area.render(
        area,
        frame.buffer_mut(),
        &mut ScrollAreaState::new()
            .area(area)
            .v_scroll(&mut app.preview_scroll),
    );

    #[expect(clippy::cast_possible_truncation)]
    let scroll_offset = (offset as u16, 0);
    frame.render_widget(Paragraph::new(lines).scroll(scroll_offset), inner);
}

fn format_title(app: &App, area_width: u16) -> String {
    let title_max = area_width.saturating_sub(2) as usize; // border chars
    app.results.get(app.selected_file()).map_or_else(
        || "Preview".to_string(),
        |fm| {
            let rel = fm.path.strip_prefix(&app.root).unwrap_or(&fm.path);
            let path_str = rel.display().to_string();
            let prefix = "Preview: ";
            let full = format!("{prefix}{path_str}");
            if full.len() <= title_max {
                full
            } else {
                // truncate path from the left with ellipsis
                let avail = title_max.saturating_sub(prefix.len() + 1); // 1 for ellipsis
                let start = path_str.len().saturating_sub(avail);
                format!("{prefix}\u{2026}{}", &path_str[start..])
            }
        },
    )
}

/// Render a centered message in the frame.
fn render_message(frame: &mut Frame, inner: Rect, msg: &str, color: Color) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let line_y = inner.y + inner.height.saturating_sub(1) / 2;
    let line_rect = Rect {
        x: inner.x,
        y: line_y,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(msg)
            .style(Style::default().fg(color))
            .alignment(Alignment::Center),
        line_rect,
    );
}

fn build_match_line(
    info: &MatchInfo,
    preview: &PreviewMatch,
    replacement: &str,
    inner_width: u16,
) -> Line<'static> {
    let PreviewMatchKind::SingleLine { line_content, .. } = &preview.kind else {
        // multiline matches are rendered by build_preview_lines directly
        return Line::default();
    };
    let before = &line_content[..preview.match_col_start];
    let matched = &line_content[preview.match_col_start..preview.match_col_end];
    let after = &line_content[preview.match_col_end..];
    let dark_gray = Style::default().fg(Color::DarkGray);

    if info.skip {
        let t = truncate_match_line(before, matched, None, after, inner_width as usize);
        let mut spans = Vec::with_capacity(7);
        spans.push(Span::raw(" "));
        if t.left_ellipsis {
            spans.push(Span::styled("\u{2026}", dark_gray));
        }
        spans.push(Span::styled(t.before.to_string(), dark_gray));
        spans.push(Span::styled(
            t.matched.to_string(),
            dark_gray.add_modifier(Modifier::CROSSED_OUT),
        ));
        spans.push(Span::styled(t.after.to_string(), dark_gray));
        if t.right_ellipsis {
            spans.push(Span::styled("\u{2026}", dark_gray));
        }
        spans.push(Span::styled(" [skipped]", dark_gray));
        Line::from(spans)
    } else if !replacement.is_empty() {
        let t = truncate_match_line(
            before,
            matched,
            Some(replacement),
            after,
            inner_width as usize,
        );
        let mut spans = Vec::with_capacity(7);
        spans.push(Span::raw(" "));
        if t.left_ellipsis {
            spans.push(Span::raw("\u{2026}"));
        }
        spans.push(Span::raw(t.before.to_string()));
        spans.push(Span::styled(
            t.matched.to_string(),
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::CROSSED_OUT),
        ));
        if let Some(repl) = t.replacement {
            spans.push(Span::styled(
                repl.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::raw(t.after.to_string()));
        if t.right_ellipsis {
            spans.push(Span::raw("\u{2026}"));
        }
        Line::from(spans)
    } else {
        let t = truncate_match_line(before, matched, None, after, inner_width as usize);
        let mut spans = Vec::with_capacity(5);
        spans.push(Span::raw(" "));
        if t.left_ellipsis {
            spans.push(Span::raw("\u{2026}"));
        }
        spans.push(Span::raw(t.before.to_string()));
        spans.push(Span::styled(
            t.matched.to_string(),
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD | Modifier::CROSSED_OUT),
        ));
        spans.push(Span::raw(t.after.to_string()));
        if t.right_ellipsis {
            spans.push(Span::raw("\u{2026}"));
        }
        Line::from(spans)
    }
}

fn build_match_header(preview: &PreviewMatch, is_selected: bool) -> Line<'static> {
    let style = if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let text = match &preview.kind {
        PreviewMatchKind::SingleLine { line_number, .. } => format!(" line {line_number}:"),
        PreviewMatchKind::MultiLine {
            line_number_start,
            line_number_end,
            ..
        } => format!(" lines {line_number_start}-{line_number_end}:"),
    };
    Line::from(Span::styled(text, style))
}

fn build_context_lines(ctx: &[ContextLine]) -> impl Iterator<Item = Line<'static>> + '_ {
    ctx.iter().map(|c| {
        Line::from(Span::styled(
            format!(" {}", c.content),
            Style::default().fg(Color::DarkGray),
        ))
    })
}

fn build_multiline_match_lines(
    info: &MatchInfo,
    preview: &PreviewMatch,
    matched_lines: &[Box<str>],
    effective_replacement: &str,
) -> Vec<Line<'static>> {
    let removed_style = Style::default().fg(Color::Red);
    let added_style = Style::default().fg(Color::Green);
    let skipped_style = Style::default().fg(Color::DarkGray);
    let prefix = &matched_lines[0][..preview.match_col_start];
    let suffix = matched_lines
        .last()
        .map_or("", |l| &l[preview.match_col_end..]);

    let mut out = Vec::new();
    if info.skip {
        for line in matched_lines {
            out.push(Line::from(Span::styled(format!("~ {line}"), skipped_style)));
        }
    } else {
        for line in matched_lines {
            out.push(Line::from(Span::styled(format!("- {line}"), removed_style)));
        }
        if !effective_replacement.is_empty() || !prefix.is_empty() || !suffix.is_empty() {
            let repl_lines: Vec<&str> = effective_replacement.split('\n').collect();
            let last_idx = repl_lines.len() - 1;
            if last_idx == 0 {
                out.push(Line::from(Span::styled(
                    format!("+ {prefix}{}{suffix}", repl_lines[0]),
                    added_style,
                )));
            } else {
                out.push(Line::from(Span::styled(
                    format!("+ {prefix}{}", repl_lines[0]),
                    added_style,
                )));
                for mid in &repl_lines[1..last_idx] {
                    out.push(Line::from(Span::styled(format!("+ {mid}"), added_style)));
                }
                out.push(Line::from(Span::styled(
                    format!("+ {}{suffix}", repl_lines[last_idx]),
                    added_style,
                )));
            }
        }
    }
    out
}

/// Count how many preview lines a multiline match produces without building the preview.
fn count_multiline_match_lines(
    info: &MatchInfo,
    preview: &PreviewMatch,
    matched_lines: &[Box<str>],
    replacement: &str,
) -> usize {
    if info.skip {
        return matched_lines.len();
    }
    let mut n = matched_lines.len();
    let has_prefix = preview.match_col_start > 0;
    let has_suffix = matched_lines
        .last()
        .is_some_and(|l| preview.match_col_end < l.len());
    if !replacement.is_empty() || has_prefix || has_suffix {
        // if replacement is empty, we still show a `+` line with the prefix and/or suffix if any
        // so we need to add 1 to the total
        n += replacement.split('\n').count();
    }
    n
}

/// Count how many preview lines a single match entry produces (header + context + match lines).
fn count_match_lines(info: &MatchInfo, preview: &PreviewMatch, replacement: &str) -> usize {
    // header
    let mut n = 1;
    n += preview.context_before.len();
    match &preview.kind {
        PreviewMatchKind::SingleLine { .. } => n += 1,
        PreviewMatchKind::MultiLine { matched_lines, .. } => {
            n += count_multiline_match_lines(info, preview, matched_lines, replacement);
        }
    }
    n += preview.context_after.len();
    n
}

fn matched_text_from_preview(preview: &PreviewMatch) -> std::borrow::Cow<'_, str> {
    match &preview.kind {
        PreviewMatchKind::SingleLine { line_content, .. } => std::borrow::Cow::Borrowed(
            &line_content[preview.match_col_start..preview.match_col_end],
        ),
        PreviewMatchKind::MultiLine { matched_lines, .. } => {
            let last = matched_lines.len() - 1;
            let mut parts = Vec::with_capacity(matched_lines.len());
            for (i, line) in matched_lines.iter().enumerate() {
                if i == 0 {
                    parts.push(&line[preview.match_col_start..]);
                } else if i == last {
                    parts.push(&line[..preview.match_col_end]);
                } else {
                    parts.push(line);
                }
            }
            std::borrow::Cow::Owned(parts.join("\n"))
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn build_preview_lines(
    matches: &[MatchInfo],
    data: &PreviewData,
    replacement: &str,
    mode: MatchMode,
    is_preview_focused: bool,
    selected_match: usize,
    inner_width: u16,
    visible_range: Range<usize>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(matches.len() * CONTEXT_LINES * 2 + 3);
    let separator: String = "\u{2500}".repeat(inner_width as usize);

    for (match_idx, (info, preview)) in matches.iter().zip(data.matches.iter()).enumerate() {
        let is_selected = is_preview_focused && match_idx == selected_match;

        if match_idx > 0 {
            lines.push(Line::styled(
                separator.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // skip expensive formatting for lines entirely outside the visible range
        if lines.len() >= visible_range.end {
            let n = count_match_lines(info, preview, replacement);
            lines.extend((0..n).map(|_| Line::default()));
        } else {
            lines.push(build_match_header(preview, is_selected));
            lines.extend(build_context_lines(&preview.context_before));

            let matched_text = matched_text_from_preview(preview);
            let expanded = expand_captures(replacement, &info.captures);
            let effective_replacement = if mode == MatchMode::CaseAware {
                case_aware_replacement(&matched_text, &expanded)
            } else {
                expanded
            };
            match &preview.kind {
                PreviewMatchKind::SingleLine { .. } => {
                    lines.push(build_match_line(
                        info,
                        preview,
                        &effective_replacement,
                        inner_width,
                    ));
                }
                PreviewMatchKind::MultiLine { matched_lines, .. } => {
                    lines.extend(build_multiline_match_lines(
                        info,
                        preview,
                        matched_lines,
                        &effective_replacement,
                    ));
                }
            }

            lines.extend(build_context_lines(&preview.context_after));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use crate::preview::data::{ContextLine, PreviewData, PreviewMatch, PreviewMatchKind};
    use crate::types::{MatchInfo, MatchMode};

    use super::*;

    fn make_info() -> MatchInfo {
        MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 5,
            skip: false,
            captures: Box::new([]),
        }
    }

    fn make_preview_single(line: &str, col_start: usize, col_end: usize) -> PreviewMatch {
        PreviewMatch {
            match_col_start: col_start,
            match_col_end: col_end,
            context_before: Box::new([]),
            context_after: Box::new([]),
            kind: PreviewMatchKind::SingleLine {
                line_number: 1,
                line_content: Box::from(line),
            },
        }
    }

    fn make_preview_multiline(lines: &[&str], col_start: usize, col_end: usize) -> PreviewMatch {
        let boxed_lines: Box<[Box<str>]> = lines.iter().map(|s| Box::from(*s)).collect();
        PreviewMatch {
            match_col_start: col_start,
            match_col_end: col_end,
            context_before: Box::new([]),
            context_after: Box::new([]),
            kind: PreviewMatchKind::MultiLine {
                line_number_start: 1,
                line_number_end: boxed_lines.len(),
                matched_lines: boxed_lines,
            },
        }
    }

    #[test]
    fn count_single_line_match() {
        // header + match line = 2
        let info = make_info();
        let preview = make_preview_single("hello world", 0, 5);
        assert_eq!(count_match_lines(&info, &preview, ""), 2);
    }

    #[test]
    fn count_single_line_with_context() {
        let info = make_info();
        let mut preview = make_preview_single("hello world", 0, 5);
        preview.context_before = vec![
            ContextLine {
                line_number: 1,
                content: Box::from("before"),
            },
            ContextLine {
                line_number: 2,
                content: Box::from("before2"),
            },
        ]
        .into();
        preview.context_after = vec![ContextLine {
            line_number: 4,
            content: Box::from("after"),
        }]
        .into();
        // header(1) + context_before(2) + match(1) + context_after(1) = 5
        assert_eq!(count_match_lines(&info, &preview, ""), 5);
    }

    #[test]
    fn count_multiline_skipped() {
        let mut info = make_info();
        info.skip = true;
        let preview = make_preview_multiline(&["foo", "bar", "baz"], 0, 3);
        // header(1) + 3 matched lines
        assert_eq!(count_match_lines(&info, &preview, "repl"), 4);
    }

    #[test]
    fn count_multiline_with_replacement() {
        let info = make_info();
        let preview = make_preview_multiline(&["  foo", "bar"], 2, 3);
        // header(1) + 2 removed + 1 replacement (single line, prefix non-empty) = 4
        assert_eq!(count_match_lines(&info, &preview, "baz"), 4);
        // multi-line replacement: "a\nb\nc" = 3 lines
        assert_eq!(count_match_lines(&info, &preview, "a\nb\nc"), 6);
    }

    #[test]
    fn count_multiline_empty_replacement_no_affixes() {
        let info = make_info();
        // match spans entire lines, so prefix and suffix are empty
        let preview = make_preview_multiline(&["foo", "bar"], 0, 3);
        // header(1) + 2 removed, no replacement lines (empty repl + empty prefix + empty suffix)
        assert_eq!(count_match_lines(&info, &preview, ""), 3);
    }

    #[test]
    fn multiline_single_replacement_line() {
        let matches = vec![make_info()];
        let preview = PreviewData {
            matches: vec![make_preview_multiline(&["    foo", "bar"], 4, 3)].into(),
            size_bytes: 0,
        };
        let lines = build_preview_lines(
            &matches,
            &preview,
            "replacement",
            MatchMode::Literal,
            false,
            0,
            80,
            0..usize::MAX,
        );
        // first + line should carry the prefix spaces
        let plus_lines: Vec<_> = lines
            .iter()
            .filter(|l| l.spans.first().is_some_and(|s| s.content.starts_with("+ ")))
            .collect();
        assert_eq!(plus_lines.len(), 1);
        assert!(plus_lines[0].spans[0].content.contains("    replacement"));
    }
}
