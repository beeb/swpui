/// Number of context lines kept on each side of a match.
pub const CONTEXT_LINES: usize = 2;

/// Max bytes of context preserved on each side of a match within a single line.
pub const MAX_CONTEXT_CHARS: usize = 160;

#[derive(Debug, Clone)]
pub struct ContextLine {
    pub line_number: usize,

    pub content: Box<str>,
}

#[must_use]
pub fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

#[must_use]
pub fn floor_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Truncate from the right, keeping at least `min_bytes` from the start.
#[must_use]
pub fn truncate_right(s: &str, min_bytes: usize) -> Box<str> {
    let limit = MAX_CONTEXT_CHARS.max(min_bytes);
    if s.len() <= limit {
        return Box::from(s);
    }
    let mut end = floor_char_boundary(s, limit);
    if end < min_bytes {
        end = ceil_char_boundary(s, min_bytes);
    }
    if end >= s.len() {
        return Box::from(s);
    }
    format!("{}\u{2026}", &s[..end]).into()
}

/// Truncate a match line, keeping `MAX_CONTEXT_CHARS` bytes of context on each side of the
/// match region `[col_start..col_end]`.
///
/// Returns `(truncated_line, new_col_start, new_col_end)`.
#[must_use]
pub fn truncate_around_match(
    line: &str,
    col_start: usize,
    col_end: usize,
) -> (Box<str>, usize, usize) {
    let keep_start = if col_start <= MAX_CONTEXT_CHARS {
        0
    } else {
        ceil_char_boundary(line, col_start - MAX_CONTEXT_CHARS)
    };

    let after_match = line.len() - col_end;
    let keep_end = if after_match <= MAX_CONTEXT_CHARS {
        line.len()
    } else {
        floor_char_boundary(line, col_end + MAX_CONTEXT_CHARS)
    };

    if keep_start == 0 && keep_end == line.len() {
        return (Box::from(line), col_start, col_end);
    }

    (
        Box::from(&line[keep_start..keep_end]),
        col_start - keep_start,
        col_end - keep_start,
    )
}

#[derive(Debug, Clone)]
pub enum PreviewMatchKind {
    SingleLine {
        line_number: usize,
        line_content: Box<str>,
    },
    MultiLine {
        line_number_start: usize,
        line_number_end: usize,
        matched_lines: Box<[Box<str>]>,
    },
}

#[derive(Debug, Clone)]
pub struct PreviewMatch {
    pub match_col_start: usize,
    pub match_col_end: usize,
    pub context_before: Box<[ContextLine]>,
    pub context_after: Box<[ContextLine]>,
    pub kind: PreviewMatchKind,
}

#[derive(Debug, Clone)]
pub struct PreviewData {
    pub matches: Box<[PreviewMatch]>,
    pub size_bytes: usize,
}

/// Build rich preview data for a file given its content and the byte ranges of each match.
///
/// Mirrors the per-match work that previously lived in `search::build_match_info`, but
/// runs on demand on the preview worker thread.
#[must_use]
pub fn build_preview_data(content: &str, byte_ranges: &[(usize, usize)]) -> PreviewData {
    let mut line_starts: Vec<usize> = std::iter::once(0)
        .chain(memchr::memchr_iter(b'\n', content.as_bytes()).map(|i| i + 1))
        .collect();
    if line_starts.last() == Some(&content.len()) {
        line_starts.pop();
    }
    let num_lines = line_starts.len();

    let mut matches = Vec::with_capacity(byte_ranges.len());
    let mut line_idx = 0;
    let mut size_bytes = 0;
    for &(byte_start, byte_end) in byte_ranges {
        while line_starts
            .get(line_idx + 1)
            .is_some_and(|&offset| offset <= byte_start)
        {
            line_idx += 1;
        }
        let pm = build_preview_match(
            content,
            &line_starts,
            num_lines,
            byte_start,
            byte_end,
            line_idx,
        );
        size_bytes += preview_match_size(&pm);
        matches.push(pm);
    }
    PreviewData {
        matches: matches.into_boxed_slice(),
        size_bytes,
    }
}

fn build_preview_match(
    content: &str,
    line_starts: &[usize],
    num_lines: usize,
    byte_start: usize,
    byte_end: usize,
    line_idx: usize,
) -> PreviewMatch {
    let get_line = |idx: usize| -> &str {
        let start = line_starts[idx];
        let end = line_starts.get(idx + 1).map_or(content.len(), |&s| s - 1);
        content[start..end].trim_end_matches('\n')
    };

    let line_number = line_idx + 1;

    let context_before: Box<[ContextLine]> = (line_idx.saturating_sub(CONTEXT_LINES)..line_idx)
        .map(|i| ContextLine {
            line_number: i + 1,
            content: truncate_right(get_line(i), 0),
        })
        .collect();

    let line_idx_end = if byte_end - byte_start > 1024 {
        line_starts.partition_point(|&s| s < byte_end) - 1
    } else {
        line_starts[line_idx + 1..]
            .iter()
            .position(|&s| s >= byte_end)
            .map_or(num_lines - 1, |pos| line_idx + pos)
    };

    let context_after: Box<[ContextLine]> = ((line_idx_end + 1)
        ..=(line_idx_end + CONTEXT_LINES).min(num_lines.saturating_sub(1)))
        .map(|i| ContextLine {
            line_number: i + 1,
            content: truncate_right(get_line(i), 0),
        })
        .collect();

    let line_start_byte = line_starts[line_idx];
    let last_line_byte = line_starts[line_idx_end];
    let last_line_str = get_line(line_idx_end);
    let mut match_col_start = byte_start - line_start_byte;
    let mut match_col_end = (byte_end - last_line_byte).min(last_line_str.len());

    let kind = if line_idx_end == line_idx {
        let (line_content, new_start, new_end) =
            truncate_around_match(last_line_str, match_col_start, match_col_end);
        match_col_start = new_start;
        match_col_end = new_end;
        PreviewMatchKind::SingleLine {
            line_number,
            line_content,
        }
    } else {
        PreviewMatchKind::MultiLine {
            line_number_start: line_idx + 1,
            line_number_end: line_idx_end + 1,
            matched_lines: (line_idx..=line_idx_end)
                .map(|i| {
                    let line = get_line(i);
                    if i == line_idx {
                        truncate_right(line, match_col_start + MAX_CONTEXT_CHARS)
                    } else if i == line_idx_end {
                        truncate_right(line, match_col_end + MAX_CONTEXT_CHARS)
                    } else {
                        truncate_right(line, 0)
                    }
                })
                .collect(),
        }
    };

    PreviewMatch {
        match_col_start,
        match_col_end,
        context_before,
        context_after,
        kind,
    }
}

fn preview_match_size(m: &PreviewMatch) -> usize {
    let ctx_size = |c: &[ContextLine]| -> usize {
        size_of_val(c) + c.iter().map(|cl| cl.content.len()).sum::<usize>()
    };
    let kind_size = match &m.kind {
        PreviewMatchKind::SingleLine { line_content, .. } => line_content.len(),
        PreviewMatchKind::MultiLine { matched_lines, .. } => {
            matched_lines.len() * size_of::<Box<str>>()
                + matched_lines.iter().map(|s| s.len()).sum::<usize>()
        }
    };
    size_of::<PreviewMatch>() + ctx_size(&m.context_before) + ctx_size(&m.context_after) + kind_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_preview_single_line() {
        let content = "hello world\n";
        let data = build_preview_data(content, &[(0, 5)]);
        assert_eq!(data.matches.len(), 1);
        let m = &data.matches[0];
        assert_eq!(m.match_col_start, 0);
        assert_eq!(m.match_col_end, 5);
        assert!(matches!(
            m.kind,
            PreviewMatchKind::SingleLine { line_number: 1, .. }
        ));
    }

    #[test]
    fn build_preview_context_before_and_after() {
        let content = "a\nb\nc\nmatch\nd\ne\nf\n";
        let pos = content.find("match").unwrap_or_else(|| unreachable!());
        let data = build_preview_data(content, &[(pos, pos + 5)]);
        let m = &data.matches[0];
        assert_eq!(m.context_before.len(), CONTEXT_LINES);
        assert_eq!(&*m.context_before[0].content, "b");
        assert_eq!(&*m.context_before[1].content, "c");
        assert_eq!(m.context_after.len(), CONTEXT_LINES);
        assert_eq!(&*m.context_after[0].content, "d");
    }

    #[test]
    fn build_preview_multiline_match() {
        let content = "foo\nbar\nbaz\n";
        let data = build_preview_data(content, &[(0, 7)]);
        let m = &data.matches[0];
        let PreviewMatchKind::MultiLine {
            line_number_start,
            line_number_end,
            matched_lines,
        } = &m.kind
        else {
            panic!("expected MultiLine");
        };
        assert_eq!(*line_number_start, 1);
        assert_eq!(*line_number_end, 2);
        assert_eq!(matched_lines.len(), 2);
    }

    #[test]
    fn build_preview_truncates_long_line_around_match() {
        let prefix = "a".repeat(200);
        let suffix = "b".repeat(200);
        let content = format!("{prefix}NEEDLE{suffix}\n");
        let pos = content.find("NEEDLE").unwrap_or_else(|| unreachable!());
        let data = build_preview_data(&content, &[(pos, pos + 6)]);
        let m = &data.matches[0];
        let PreviewMatchKind::SingleLine { line_content, .. } = &m.kind else {
            panic!("expected SingleLine");
        };
        let before = &line_content[..m.match_col_start];
        let after = &line_content[m.match_col_end..];
        assert_eq!(before.len(), MAX_CONTEXT_CHARS);
        assert_eq!(after.len(), MAX_CONTEXT_CHARS);
    }

    #[test]
    fn build_preview_size_bytes_nonzero() {
        let content = "hello world\n";
        let data = build_preview_data(content, &[(0, 5)]);
        assert!(data.size_bytes > 0);
    }

    #[test]
    fn build_preview_empty_byte_ranges() {
        let data = build_preview_data("hello\n", &[]);
        assert_eq!(data.matches.len(), 0);
        assert_eq!(data.size_bytes, 0);
    }
}
