use std::ops::Range;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::{
    preview::data::{CONTEXT_LINES, ContextLine, PreviewData, PreviewMatch, PreviewMatchKind},
    replace::Replacement,
    types::{MatchInfo, MatchMode},
    utils::TruncatedLine,
};

/// Left-side gutter for every preview line.
struct Gutter {
    line_nb_width: usize,
}

impl Gutter {
    fn new(data: &PreviewData) -> Self {
        let mut max_ln = 1usize;
        for p in &data.matches {
            for c in p.context_before.iter().chain(p.context_after.iter()) {
                max_ln = max_ln.max(c.line_number);
            }
            match &p.kind {
                PreviewMatchKind::SingleLine { line_number, .. } => {
                    max_ln = max_ln.max(*line_number);
                }
                PreviewMatchKind::MultiLine {
                    line_number_start,
                    matched_lines,
                } => {
                    let last = line_number_start + matched_lines.len().saturating_sub(1);
                    max_ln = max_ln.max(last);
                }
            }
        }
        Self {
            line_nb_width: max_ln.to_string().len(),
        }
    }

    /// Gutter for a context line: blank indicator, blue line number.
    fn context_spans(&self, line_number: usize, is_skipped: bool) -> Vec<Span<'static>> {
        let width = self.line_nb_width;
        let style = if is_skipped {
            Style::default().dim()
        } else {
            Style::default().fg(Color::Blue).dim()
        };
        vec![
            Span::raw(" "),
            Span::styled(format!("{line_number:>width$} "), style),
        ]
    }

    /// Gutter for a matched line.
    fn match_spans(
        &self,
        line_number: Option<usize>,
        is_selected: bool,
        is_skipped: bool,
    ) -> Vec<Span<'static>> {
        let line_nb_style = if is_skipped {
            Style::default().dim()
        } else {
            Style::default().fg(if is_selected {
                Color::Cyan
            } else {
                Color::Blue
            })
        };
        let (ind, ind_style) = if is_selected {
            (
                ">",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (" ", Style::default())
        };
        let width = self.line_nb_width;
        let number_part = match line_number {
            Some(n) => format!("{n:>width$} "),
            None => format!("{:>width$} ", ""),
        };
        vec![
            Span::styled(ind.to_string(), ind_style),
            Span::styled(number_part, line_nb_style),
        ]
    }

    /// Effective `max_width` to pass to [`TruncatedLine::new`] so that the gutter and content together fit
    /// within `inner_width`.
    fn content_max_width(&self, inner_width: u16) -> usize {
        (inner_width as usize).saturating_sub(self.line_nb_width + 1)
    }
}

/// One rendered preview line that belongs to a match (matched, removed, added, or skipped).
struct MatchLine {
    spans: Vec<Span<'static>>,
    is_selected: bool,
}

impl MatchLine {
    fn new(
        gutter: &Gutter,
        line_number: Option<usize>,
        is_selected: bool,
        is_skipped: bool,
    ) -> Self {
        Self {
            spans: gutter.match_spans(line_number, is_selected, is_skipped),
            is_selected,
        }
    }

    fn push(&mut self, span: Span<'static>) {
        self.spans.push(span);
    }
}

impl From<MatchLine> for Line<'static> {
    fn from(mut ml: MatchLine) -> Self {
        if ml.is_selected {
            for span in &mut ml.spans {
                span.style = span.style.add_modifier(Modifier::BOLD);
            }
        }
        Line::from(ml.spans)
    }
}

pub struct PreviewBuilder<'a> {
    matches: &'a [MatchInfo],
    data: &'a PreviewData,
    replacement: &'a str,
    mode: MatchMode,
    selected: Option<usize>,
    inner_width: u16,
    gutter: Gutter,
}

impl<'a> PreviewBuilder<'a> {
    pub fn new(
        matches: &'a [MatchInfo],
        data: &'a PreviewData,
        replacement: &'a str,
        mode: MatchMode,
        is_preview_focused: bool,
        selected_match: usize,
        inner_width: u16,
    ) -> Self {
        let gutter = Gutter::new(data);
        let selected = is_preview_focused.then_some(selected_match);
        Self {
            matches,
            data,
            replacement,
            mode,
            selected,
            inner_width,
            gutter,
        }
    }

    /// Total preview line count and the [start, end) range covered by the selected match.
    ///
    /// The selected range is `0..0` when the preview is unfocused.
    pub fn layout(&self) -> (usize, Range<usize>) {
        let mut total_lines = 0;
        let mut selected_range: Range<usize> = 0..0;
        for (match_idx, (info, preview)) in self
            .matches
            .iter()
            .zip(self.data.matches.iter())
            .enumerate()
        {
            if match_idx > 0 {
                total_lines += 1; // separator
            }
            let match_start = total_lines;
            total_lines += preview.line_count(info, self.replacement);
            if self.selected == Some(match_idx) {
                selected_range = match_start..total_lines;
            }
        }
        (total_lines, selected_range)
    }

    /// Generate the set of lines for the preview.
    pub fn build(&self, visible_range: Range<usize>) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> =
            Vec::with_capacity(self.matches.len() * CONTEXT_LINES * 2 + 3);
        let separator: String = "\u{2500}".repeat(self.inner_width as usize);

        for (match_idx, (info, preview)) in self
            .matches
            .iter()
            .zip(self.data.matches.iter())
            .enumerate()
        {
            let is_selected = self.selected == Some(match_idx);

            if match_idx > 0 {
                lines.push(Line::styled(separator.clone(), Style::default().dim()));
            }

            // skip expensive formatting for matches entirely outside the visible range
            if lines.len() >= visible_range.end {
                let n = preview.line_count(info, self.replacement);
                lines.extend((0..n).map(|_| Line::default()));
            } else {
                lines.extend(self.context_lines(&preview.context_before, info.skip));

                let (ctx, range) = preview.match_context();
                let effective_replacement =
                    Replacement::new(info, &ctx, range, self.replacement, self.mode).compute();
                match &preview.kind {
                    PreviewMatchKind::SingleLine { .. } => {
                        lines.push(self.match_line(
                            info,
                            preview,
                            &effective_replacement,
                            is_selected,
                        ));
                    }
                    PreviewMatchKind::MultiLine { matched_lines, .. } => {
                        lines.extend(self.multiline_match_lines(
                            info,
                            preview,
                            matched_lines,
                            &effective_replacement,
                            is_selected,
                        ));
                    }
                }

                lines.extend(self.context_lines(&preview.context_after, info.skip));
            }
        }

        lines
    }

    fn context_lines<'b>(
        &'b self,
        ctx: &'b [ContextLine],
        is_skipped: bool,
    ) -> impl Iterator<Item = Line<'static>> + 'b {
        ctx.iter().map(move |c| {
            let mut spans = self.gutter.context_spans(c.line_number, is_skipped);
            spans.push(Span::styled(c.content.to_string(), Style::default().dim()));
            Line::from(spans)
        })
    }

    fn match_line(
        &self,
        info: &MatchInfo,
        preview: &PreviewMatch,
        replacement: &str,
        is_selected: bool,
    ) -> Line<'static> {
        let PreviewMatchKind::SingleLine {
            line_content,
            line_number,
        } = &preview.kind
        else {
            // multiline matches go through `multiline_match_lines`
            return Line::default();
        };
        let before = &line_content[..preview.match_col_start];
        let matched = &line_content[preview.match_col_start..preview.match_col_end];
        let after = &line_content[preview.match_col_end..];
        let dim = Style::default().dim();
        let content_max = self.gutter.content_max_width(self.inner_width);

        let mut ml = MatchLine::new(&self.gutter, Some(*line_number), is_selected, info.skip);

        if info.skip {
            let t = TruncatedLine::new(before, matched, None, after, content_max);
            if t.left_ellipsis {
                ml.push(Span::styled("\u{2026}", dim));
            }
            ml.push(Span::styled(
                format!("{}{}{}", t.before, t.matched, t.after),
                dim,
            ));
            if t.right_ellipsis {
                ml.push(Span::styled("\u{2026}", dim));
            }
        } else if !replacement.is_empty() {
            let t = TruncatedLine::new(before, matched, Some(replacement), after, content_max);
            if t.left_ellipsis {
                ml.push(Span::raw("\u{2026}"));
            }
            ml.push(Span::raw(t.before.to_string()));
            ml.push(Span::styled(
                t.matched.to_string(),
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::CROSSED_OUT),
            ));
            if let Some(repl) = t.replacement {
                ml.push(Span::styled(
                    repl.to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            ml.push(Span::raw(t.after.to_string()));
            if t.right_ellipsis {
                ml.push(Span::raw("\u{2026}"));
            }
        } else {
            let t = TruncatedLine::new(before, matched, None, after, content_max);
            if t.left_ellipsis {
                ml.push(Span::raw("\u{2026}"));
            }
            ml.push(Span::raw(t.before.to_string()));
            ml.push(Span::styled(
                t.matched.to_string(),
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD | Modifier::CROSSED_OUT),
            ));
            ml.push(Span::raw(t.after.to_string()));
            if t.right_ellipsis {
                ml.push(Span::raw("\u{2026}"));
            }
        }

        ml.into()
    }

    fn multiline_match_lines(
        &self,
        info: &MatchInfo,
        preview: &PreviewMatch,
        matched_lines: &[Box<str>],
        effective_replacement: &str,
        is_selected: bool,
    ) -> Vec<Line<'static>> {
        let removed_style = Style::default().fg(Color::Red);
        let added_style = Style::default().fg(Color::Green);
        let skipped_style = Style::default().dim();
        let prefix = &matched_lines[0][..preview.match_col_start];
        let suffix = matched_lines
            .last()
            .map_or("", |l| &l[preview.match_col_end..]);
        let line_number_start = match &preview.kind {
            PreviewMatchKind::MultiLine {
                line_number_start, ..
            } => *line_number_start,
            PreviewMatchKind::SingleLine { .. } => return Vec::new(),
        };

        let make_line =
            |line_number: Option<usize>, marker: &str, content: String, style: Style| {
                let mut ml = MatchLine::new(&self.gutter, line_number, is_selected, info.skip);
                ml.push(Span::styled(format!("{marker} {content}"), style));
                ml.into()
            };

        let mut out = Vec::new();
        if info.skip {
            for (i, line) in matched_lines.iter().enumerate() {
                out.push(make_line(
                    Some(line_number_start + i),
                    "~",
                    line.to_string(),
                    skipped_style,
                ));
            }
        } else {
            for (i, line) in matched_lines.iter().enumerate() {
                out.push(make_line(
                    Some(line_number_start + i),
                    "-",
                    line.to_string(),
                    removed_style,
                ));
            }
            if !effective_replacement.is_empty() || !prefix.is_empty() || !suffix.is_empty() {
                // strip trailing `\r` from each piece so a CRLF replacement renders
                // cleanly (without leaking a control char into the displayed span)
                let repl_lines: Vec<&str> = effective_replacement
                    .split('\n')
                    .map(|s| s.trim_end_matches('\r'))
                    .collect();
                let last_idx = repl_lines.len() - 1;
                if last_idx == 0 {
                    out.push(make_line(
                        None,
                        "+",
                        format!("{prefix}{}{suffix}", repl_lines[0]),
                        added_style,
                    ));
                } else {
                    out.push(make_line(
                        None,
                        "+",
                        format!("{prefix}{}", repl_lines[0]),
                        added_style,
                    ));
                    for mid in &repl_lines[1..last_idx] {
                        out.push(make_line(None, "+", (*mid).to_string(), added_style));
                    }
                    out.push(make_line(
                        None,
                        "+",
                        format!("{}{suffix}", repl_lines[last_idx]),
                        added_style,
                    ));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::preview::data::{PreviewData, PreviewMatch, PreviewMatchKind};
    use crate::types::{ByteRange, MatchInfo, MatchMode};

    use super::*;

    fn make_info() -> MatchInfo {
        MatchInfo {
            byte_range: ByteRange::new(0, 5),
            skip: false,
            captures: Box::new([]),
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
                matched_lines: boxed_lines,
            },
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

    fn preview_lines(preview: &PreviewMatch) -> Box<[Box<str>]> {
        match &preview.kind {
            PreviewMatchKind::MultiLine { matched_lines, .. } => matched_lines.clone(),
            PreviewMatchKind::SingleLine { .. } => unreachable!(),
        }
    }

    fn test_builder<'a>(
        matches: &'a [MatchInfo],
        data: &'a PreviewData,
        replacement: &'a str,
        mode: MatchMode,
    ) -> PreviewBuilder<'a> {
        PreviewBuilder::new(matches, data, replacement, mode, false, 0, 80)
    }

    #[test]
    fn multiline_replacement_strips_carriage_return_from_rendered_spans() {
        // a CRLF replacement (e.g. typed as `\r\n` in RegexMultiline mode and expanded)
        // must not leak the trailing `\r` into the rendered diff spans
        let info = make_info();
        let preview = make_preview_multiline(&["  foo", "bar"], 2, 3);
        let matches = vec![info.clone()];
        let data = PreviewData {
            matches: vec![preview.clone()].into(),
            size: 0,
        };
        let builder = test_builder(&matches, &data, "", MatchMode::Literal);
        let lines = builder.multiline_match_lines(
            &info,
            &preview,
            &preview_lines(&preview),
            "a\r\nb",
            false,
        );
        for line in &lines {
            for span in &line.spans {
                assert!(
                    !span.content.contains('\r'),
                    "rendered span leaked a '\\r': {:?}",
                    span.content
                );
            }
        }
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn multiline_single_replacement_line() {
        let matches = vec![make_info()];
        let data = PreviewData {
            matches: vec![make_preview_multiline(&["    foo", "bar"], 4, 3)].into(),
            size: 0,
        };
        let builder = test_builder(&matches, &data, "replacement", MatchMode::Literal);
        let lines = builder.build(0..usize::MAX);
        // first + line should carry the prefix spaces
        let plus_lines: Vec<_> = lines
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.content.starts_with("+ ")))
            .collect();
        assert_eq!(plus_lines.len(), 1);
        let combined: String = plus_lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(combined.contains("    replacement"));
    }

    #[test]
    fn case_aware_substring_in_camel_identifier_renders_camel_replacement() {
        // regression: a CaseAware match of "foo" (cols 0..3) inside the camelCase identifier
        // "fooBar" must render the replacement "new_thing" as "newThing" the same way the
        // file write does. Before the shared-pipeline refactor the preview rendered
        // "new_thing" because it skipped word-boundary expansion.
        let matches = vec![MatchInfo {
            byte_range: ByteRange::new(0, 3),
            skip: false,
            captures: Box::new([]),
        }];
        let data = PreviewData {
            matches: vec![make_preview_single("fooBar", 0, 3)].into(),
            size: 0,
        };
        let builder = test_builder(&matches, &data, "new_thing", MatchMode::CaseAware);
        let lines = builder.build(0..usize::MAX);
        let rendered: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            rendered.contains("newThing"),
            "expected preview to render camelCase replacement; got {rendered:?}"
        );
        assert!(
            !rendered.contains("new_thing"),
            "preview leaked the un-cased replacement; got {rendered:?}"
        );
    }

    #[test]
    fn selected_match_line_has_indicator_and_bold() {
        let matches = vec![make_info()];
        let data = PreviewData {
            matches: vec![make_preview_single("hello world", 0, 5)].into(),
            size: 0,
        };
        let builder = PreviewBuilder::new(&matches, &data, "", MatchMode::Literal, true, 0, 80);
        let lines = builder.build(0..usize::MAX);
        // single match, no context: just the match line
        assert_eq!(lines.len(), 1);
        let match_line = &lines[0];
        assert_eq!(match_line.spans[0].content.as_ref(), ">");
        // every span on the matched line must carry BOLD because Paragraph renders spans
        // with their own style rather than inheriting Line::style
        for span in &match_line.spans {
            assert!(
                span.style.add_modifier.contains(Modifier::BOLD),
                "expected BOLD on span {:?}",
                span.content
            );
        }
    }

    #[test]
    fn unselected_match_line_has_blank_indicator() {
        let matches = vec![make_info()];
        let data = PreviewData {
            matches: vec![make_preview_single("hello world", 0, 5)].into(),
            size: 0,
        };
        let builder = test_builder(&matches, &data, "", MatchMode::Literal);
        let lines = builder.build(0..usize::MAX);
        let match_line = &lines[0];
        assert_eq!(match_line.spans[0].content.as_ref(), " ");
        // before/after spans (Span::raw) should not carry BOLD when the match isn't selected
        let before_span = &match_line.spans[2];
        assert!(!before_span.style.add_modifier.contains(Modifier::BOLD));
    }
}
