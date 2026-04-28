use std::ops::Range;

use rat_widget::scrolled::{Scroll, ScrollArea, ScrollAreaState};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Paragraph, StatefulWidget as _},
};

use super::focused_border_style;
use crate::{
    app::App,
    preview::data::CONTEXT_LINES,
    replace::{case_aware_replacement, effective_replacement, expand_captures},
    types::{FileMatches, MatchInfo, MatchKind, MatchMode, Pane},
    utils::truncate_match_line,
};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let border_style = focused_border_style(Pane::Preview, app.focused_pane);

    let title_max = area.width.saturating_sub(2) as usize; // border chars
    let title = app.results.get(app.selected_file()).map_or_else(
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
    );

    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(border_style)
        .title(title);

    let Some(fm) = app.results.get(app.selected_file()) else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("Select a file").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
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
    for (match_idx, m) in fm.matches.iter().enumerate() {
        if match_idx > 0 {
            total_lines += 1; // separator
        }
        let match_start = total_lines;
        total_lines += count_match_lines(m, &replacement);
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
    let (lines, _) = build_preview_lines(
        fm,
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

fn build_match_line(m: &MatchInfo, replacement: &str, inner_width: u16) -> Line<'static> {
    let MatchKind::SingleLine { line_content, .. } = &m.kind else {
        // multiline matches are rendered by build_preview_lines directly
        return Line::default();
    };
    let before = &line_content[..m.match_col_start];
    let matched = m.matched_text();
    let after = &line_content[m.match_col_end..];
    let dark_gray = Style::default().fg(Color::DarkGray);

    if m.skip {
        let t = truncate_match_line(before, &matched, None, after, inner_width as usize);
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
            &matched,
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
        let t = truncate_match_line(before, &matched, None, after, inner_width as usize);
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

fn build_match_header(m: &MatchInfo, is_selected: bool) -> Line<'static> {
    let style = if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let text = match &m.kind {
        MatchKind::SingleLine { line_number, .. } => format!(" line {line_number}:"),
        MatchKind::MultiLine {
            line_number_start,
            line_number_end,
            ..
        } => format!(" lines {line_number_start}-{line_number_end}:"),
    };
    Line::from(Span::styled(text, style))
}

fn build_context_lines(
    ctx: &[crate::preview::data::ContextLine],
) -> impl Iterator<Item = Line<'static>> + '_ {
    ctx.iter().map(|c| {
        Line::from(Span::styled(
            format!(" {}", c.content),
            Style::default().fg(Color::DarkGray),
        ))
    })
}

fn build_multiline_match_lines(
    m: &MatchInfo,
    matched_lines: &[Box<str>],
    effective_replacement: &str,
) -> Vec<Line<'static>> {
    let removed_style = Style::default().fg(Color::Red);
    let added_style = Style::default().fg(Color::Green);
    let skipped_style = Style::default().fg(Color::DarkGray);
    let prefix = &matched_lines[0][..m.match_col_start];
    let suffix = matched_lines.last().map_or("", |l| &l[m.match_col_end..]);

    let mut out = Vec::new();
    if m.skip {
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
    m: &MatchInfo,
    matched_lines: &[Box<str>],
    replacement: &str,
) -> usize {
    if m.skip {
        return matched_lines.len();
    }
    let mut n = matched_lines.len();
    let has_prefix = m.match_col_start > 0;
    let has_suffix = matched_lines
        .last()
        .is_some_and(|l| m.match_col_end < l.len());
    if !replacement.is_empty() || has_prefix || has_suffix {
        // if replacement is empty, we still show a `+` line with the prefix and/or suffix if any
        // so we need to add 1 to the total
        n += replacement.split('\n').count();
    }
    n
}

/// Count how many preview lines a single match entry produces (header + context + match lines).
fn count_match_lines(m: &MatchInfo, replacement: &str) -> usize {
    // header
    let mut n = 1;
    n += m.context_before.len();
    match &m.kind {
        MatchKind::SingleLine { .. } => n += 1,
        MatchKind::MultiLine { matched_lines, .. } => {
            n += count_multiline_match_lines(m, matched_lines, replacement);
        }
    }
    n += m.context_after.len();
    n
}

fn build_preview_lines(
    fm: &FileMatches,
    replacement: &str,
    mode: MatchMode,
    is_preview_focused: bool,
    selected_match: usize,
    inner_width: u16,
    visible_range: Range<usize>,
) -> (Vec<Line<'static>>, Range<usize>) {
    let mut lines: Vec<Line<'static>> =
        Vec::with_capacity(fm.matches.len() * CONTEXT_LINES * 2 + 3);
    let mut selected_range: Range<usize> = 0..0;
    let separator: String = "─".repeat(inner_width as usize);

    for (match_idx, m) in fm.matches.iter().enumerate() {
        let is_selected = is_preview_focused && match_idx == selected_match;

        if match_idx > 0 {
            lines.push(Line::styled(
                separator.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        let match_start = lines.len();

        // skip expensive formatting for lines entirely outside the visible range
        if lines.len() >= visible_range.end {
            let n = count_match_lines(m, replacement);
            lines.extend((0..n).map(|_| Line::default()));
        } else {
            lines.push(build_match_header(m, is_selected));
            lines.extend(build_context_lines(&m.context_before));

            let matched_text = m.matched_text();
            let expanded = expand_captures(replacement, &m.captures);
            let effective_replacement = if mode == MatchMode::CaseAware {
                case_aware_replacement(&matched_text, &expanded)
            } else {
                expanded
            };
            match &m.kind {
                MatchKind::SingleLine { .. } => {
                    lines.push(build_match_line(m, &effective_replacement, inner_width));
                }
                MatchKind::MultiLine { matched_lines, .. } => {
                    lines.extend(build_multiline_match_lines(
                        m,
                        matched_lines,
                        &effective_replacement,
                    ));
                }
            }

            lines.extend(build_context_lines(&m.context_after));
        }

        if is_selected {
            selected_range = match_start..lines.len();
        }
    }

    (lines, selected_range)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        preview::data::ContextLine,
        types::{FileMatches, MatchInfo, MatchKind, MatchMode},
    };

    use super::*;

    fn make_multiline_match(
        matched_lines: &[&str],
        match_col_start: usize,
        match_col_end: usize,
    ) -> MatchInfo {
        let boxed_lines: Box<[Box<str>]> = matched_lines.iter().map(|s| Box::from(*s)).collect();
        MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 10,
            match_col_start,
            match_col_end,
            context_before: Box::new([]),
            context_after: Box::new([]),
            skip: false,
            kind: MatchKind::MultiLine {
                line_number_start: 1,
                line_number_end: boxed_lines.len(),
                matched_lines: boxed_lines,
            },
            captures: Box::new([]),
        }
    }

    fn make_single_line_match(line_content: &str, col_start: usize, col_end: usize) -> MatchInfo {
        MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 10,
            match_col_start: col_start,
            match_col_end: col_end,
            context_before: Box::new([]),
            context_after: Box::new([]),
            skip: false,
            kind: MatchKind::SingleLine {
                line_number: 1,
                line_content: Box::from(line_content),
            },
            captures: Box::new([]),
        }
    }

    #[test]
    fn count_single_line_match() {
        // header + match line = 2
        let m = make_single_line_match("hello world", 0, 5);
        assert_eq!(count_match_lines(&m, ""), 2);
    }

    #[test]
    fn count_single_line_with_context() {
        let ctx = vec![
            ContextLine {
                line_number: 1,
                content: Box::from("before"),
            },
            ContextLine {
                line_number: 2,
                content: Box::from("before2"),
            },
        ];
        let mut m = make_single_line_match("hello world", 0, 5);
        m.context_before = ctx.into();
        m.context_after = vec![ContextLine {
            line_number: 4,
            content: Box::from("after"),
        }]
        .into();
        // header(1) + context_before(2) + match(1) + context_after(1) = 5
        assert_eq!(count_match_lines(&m, ""), 5);
    }

    #[test]
    fn count_multiline_skipped() {
        let mut m = make_multiline_match(&["foo", "bar", "baz"], 0, 3);
        m.skip = true;
        // header(1) + 3 matched lines
        assert_eq!(count_match_lines(&m, "repl"), 4);
    }

    #[test]
    fn count_multiline_with_replacement() {
        let m = make_multiline_match(&["  foo", "bar"], 2, 3);
        // header(1) + 2 removed + 1 replacement (single line, prefix non-empty) = 4
        assert_eq!(count_match_lines(&m, "baz"), 4);
        // multi-line replacement: "a\nb\nc" = 3 lines
        assert_eq!(count_match_lines(&m, "a\nb\nc"), 6);
    }

    #[test]
    fn count_multiline_empty_replacement_no_affixes() {
        // match spans entire lines, so prefix and suffix are empty
        let m = make_multiline_match(&["foo", "bar"], 0, 3);
        // header(1) + 2 removed, no replacement lines (empty repl + empty prefix + empty suffix)
        assert_eq!(count_match_lines(&m, ""), 3);
    }

    #[test]
    fn multiline_single_replacement_line() {
        let fm = FileMatches {
            path: PathBuf::from("test.rs"),
            matches: vec![make_multiline_match(
                &["    foo", "bar"],
                4, // match starts (first line)
                3, // match ends (last line)
            )],
            content_hash: [0; 32],
        };
        let (lines, _) = build_preview_lines(
            &fm,
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
        assert!(plus_lines[0].spans[0].content.starts_with("+ "));
        assert!(plus_lines[0].spans[0].content.contains("    replacement"));
    }
}
