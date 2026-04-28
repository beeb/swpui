/// Number of context lines kept on each side of a match.
pub const CONTEXT_LINES: usize = 2;

/// Max bytes of context preserved on each side of a match within a single line.
pub const MAX_CONTEXT_CHARS: usize = 160;

#[derive(Debug, Clone)]
pub struct ContextLine {
    pub line_number: usize,

    pub content: Box<str>,
}

pub(crate) fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

pub(crate) fn floor_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Truncate from the right, keeping at least `min_bytes` from the start.
pub(crate) fn truncate_right(s: &str, min_bytes: usize) -> Box<str> {
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
pub(crate) fn truncate_around_match(
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
