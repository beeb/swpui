use ratatui::buffer::CellWidth;
use unicode_segmentation::UnicodeSegmentation;

use crate::{search::FileMatches, types::MatchInfo};

pub struct TruncatedLine<'a> {
    pub before: &'a str,
    pub matched: &'a str,
    pub replacement: Option<&'a str>,
    pub after: &'a str,
    pub left_ellipsis: bool,
    pub right_ellipsis: bool,
}

impl<'a> TruncatedLine<'a> {
    pub fn new(
        before: &'a str,
        matched: &'a str,
        replacement: Option<&'a str>,
        after: &'a str,
        max_width: u16,
    ) -> TruncatedLine<'a> {
        let budget = max_width.saturating_sub(1); // account for leading space/margin

        let before_w = before.cell_width();
        let matched_w = matched.cell_width();
        let replacement_w = replacement.map_or(0, CellWidth::cell_width);
        let after_w = after.cell_width();
        let total_w = before_w + matched_w + replacement_w + after_w;

        // everything fits
        if total_w <= budget {
            return TruncatedLine {
                before,
                matched,
                replacement,
                after,
                left_ellipsis: false,
                right_ellipsis: false,
            };
        }

        let center_w = matched_w + replacement_w;

        // match+replacement fits, trim before/after
        if center_w <= budget {
            let remaining = budget - center_w;

            // if only one side overflows, give the other its full width
            let (before_budget, after_budget) = if before_w <= remaining && after_w <= remaining {
                // both fit individually but not together: split equally
                (remaining.div_ceil(2), remaining / 2)
            } else if before_w <= remaining {
                // only before fits individually
                (before_w, remaining - before_w)
            } else if after_w <= remaining {
                // only after fits individually
                (remaining - after_w, after_w)
            } else {
                // neither fits individually: split equally
                (remaining.div_ceil(2), remaining / 2)
            };

            let (before, left_ellipsis) = trim_start_to_width(before, before_budget, true);
            let (after, right_ellipsis) = trim_end_to_width(after, after_budget, true);

            return TruncatedLine {
                before,
                matched,
                replacement,
                after,
                left_ellipsis,
                right_ellipsis,
            };
        }

        // match+replacement exceeds budget
        let left_ellipsis = true;
        let avail = budget.saturating_sub(1); // reserve 1 col for left ellipsis

        let (matched, replacement, right_ellipsis) = if let Some(repl) = replacement {
            let repl_w = repl.cell_width();
            if repl_w <= avail {
                // replacement fits, give remaining to match (from the right end)
                let match_avail = avail - repl_w;
                (
                    trim_start_to_width(matched, match_avail, false).0,
                    Some(repl),
                    false,
                )
            } else {
                // replacement overflows: truncate from the right, no room for match
                let repl_avail = avail.saturating_sub(1); // reserve 1 col for right ellipsis
                ("", Some(trim_end_to_width(repl, repl_avail, false).0), true)
            }
        } else {
            // truncate match from the right end (no replacement)
            (trim_start_to_width(matched, avail, false).0, None, false)
        };

        TruncatedLine {
            before: "",
            matched,
            replacement,
            after: "",
            left_ellipsis,
            right_ellipsis,
        }
    }
}

/// Logging helper to calculate the size in memory the search results.
pub fn results_mem_bytes(results: &[FileMatches]) -> usize {
    size_of_val(results) + results.iter().map(file_matches_mem_bytes).sum::<usize>()
}

/// Trim a string from the left to fit within `max_cols` display columns.
///
/// If `ellipsis` is true, reserves 1 column for an ellipsis character.
///
/// Returns `(visible_slice, was_trimmed)`.
#[must_use]
pub fn trim_start_to_width(s: &str, max_cols: u16, ellipsis: bool) -> (&str, bool) {
    let w = s.cell_width();
    if w <= max_cols {
        return (s, false);
    }
    let target = max_cols.saturating_sub(u16::from(ellipsis)); // reserve 1 col for ellipsis if requested
    let mut cols: u16 = 0;
    let mut byte_start = s.len();
    for (idx, g) in s.grapheme_indices(true).rev() {
        let next = cols.saturating_add(g.cell_width());
        if next > target {
            break;
        }
        cols = next;
        byte_start = idx;
    }
    (&s[byte_start..], true)
}

/// Trim a string from the right to fit within `max_cols` display columns.
///
/// If `ellipsis` is true, reserves 1 column for an ellipsis character.
///
/// Returns `(visible_slice, was_trimmed)`.
#[must_use]
pub fn trim_end_to_width(s: &str, max_cols: u16, ellipsis: bool) -> (&str, bool) {
    let w = s.cell_width();
    if w <= max_cols {
        return (s, false);
    }
    let target = max_cols.saturating_sub(u16::from(ellipsis)); // reserve 1 col for ellipsis if requested
    let mut cols: u16 = 0;
    let mut byte_end = 0;
    for (idx, g) in s.grapheme_indices(true) {
        let next = cols.saturating_add(g.cell_width());
        if next > target {
            break;
        }
        cols = next;
        byte_end = idx + g.len();
    }
    (&s[..byte_end], true)
}

fn file_matches_mem_bytes(fm: &FileMatches) -> usize {
    fm.path.capacity()
        + fm.matches.capacity() * size_of::<MatchInfo>()
        + fm.matches
            .iter()
            .map(|m| m.captures.iter().map(|c| c.len()).sum::<usize>())
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_everything_fits() {
        let result = TruncatedLine::new("before ", "match", None, " after", 20);
        assert_eq!(result.before, "before ");
        assert_eq!(result.matched, "match");
        assert!(result.replacement.is_none());
        assert_eq!(result.after, " after");
        assert!(!result.left_ellipsis);
        assert!(!result.right_ellipsis);
    }

    #[test]
    fn truncate_everything_fits_with_replacement() {
        let result = TruncatedLine::new("ab", "cd", Some("XY"), "ef", 20);
        assert_eq!(result.before, "ab");
        assert_eq!(result.matched, "cd");
        assert_eq!(result.replacement, Some("XY"));
        assert_eq!(result.after, "ef");
        assert!(!result.left_ellipsis);
        assert!(!result.right_ellipsis);
    }

    #[test]
    fn truncate_exact_fit() {
        // "before " (7) + "match" (5) + " after" (6) = 18, budget = 19 - 1 = 18
        let result = TruncatedLine::new("before ", "match", None, " after", 19);
        assert_eq!(result.before, "before ");
        assert!(!result.left_ellipsis);
        assert!(!result.right_ellipsis);
    }

    #[test]
    fn truncate_trims_before_and_after() {
        // before=10 + match=2 + after=10 = 22, budget = 15 - 1 = 14
        // center=2, remaining=12, both overflow -> split equally: 6/6
        // before: 6 - 1 ellipsis = 5 cols from right -> "56789"
        // after: 6 - 1 ellipsis = 5 cols from left -> "abcde"
        let result = TruncatedLine::new("0123456789", "XX", None, "abcdefghij", 15);
        assert!(result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.matched, "XX");
        assert_eq!(result.before, "56789");
        assert_eq!(result.after, "abcde");
    }

    #[test]
    fn truncate_trims_only_before() {
        // before=10 + match=2 + after=2 = 14, budget = 10 - 1 = 9
        // center=2, remaining=7, after fits (2<=7), before overflows
        // after_budget=2, before_budget=5, trimmed: 5 - 1 ellipsis = 4 -> "6789"
        let result = TruncatedLine::new("0123456789", "XX", None, "ab", 10);
        assert!(result.left_ellipsis);
        assert!(!result.right_ellipsis);
        assert_eq!(result.before, "6789");
        assert_eq!(result.after, "ab");
    }

    #[test]
    fn truncate_trims_only_after() {
        // before=2 + match=2 + after=10 = 14, budget = 10 - 1 = 9
        // center=2, remaining=7, before fits (2<=7), after overflows
        // before_budget=2, after_budget=5, trimmed: 5 - 1 ellipsis = 4 -> "0123"
        let result = TruncatedLine::new("ab", "XX", None, "0123456789", 10);
        assert!(!result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.before, "ab");
        assert_eq!(result.after, "0123");
    }

    #[test]
    fn truncate_odd_remaining_favors_before() {
        // before=10 + match=1 + after=10 = 21, budget = 13 - 1 = 12
        // center=1, remaining=11, both overflow -> split: ceil(11/2)=6 / 5
        // before: 6 - 1 = 5 -> "56789"
        // after: 5 - 1 = 4 -> "abcd"
        let result = TruncatedLine::new("0123456789", "X", None, "abcdefghij", 13);
        assert!(result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.before, "56789");
        assert_eq!(result.after, "abcd");
    }

    #[test]
    fn truncate_with_replacement_trims_surroundings() {
        // before=10 + match=2 + repl=2 + after=10 = 24, budget = 15 - 1 = 14
        // center=4, remaining=10, both overflow -> split: 5/5
        // before: 5 - 1 = 4 -> "6789"
        // after: 5 - 1 = 4 -> "abcd"
        let result = TruncatedLine::new("0123456789", "MM", Some("RR"), "abcdefghij", 15);
        assert!(result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.matched, "MM");
        assert_eq!(result.replacement, Some("RR"));
        assert_eq!(result.before, "6789");
        assert_eq!(result.after, "abcd");
    }

    #[test]
    fn truncate_oversized_match_no_replacement() {
        // match="ABCDEFGHIJKLMNO" (15), budget = 10 - 1 = 9
        // center overflows. No replacement.
        // left_ellipsis=true, avail = 9 - 1 = 8, take 8 from end -> "HIJKLMNO"
        let result = TruncatedLine::new("x", "ABCDEFGHIJKLMNO", None, "y", 10);
        assert!(result.left_ellipsis);
        assert!(!result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "HIJKLMNO");
        assert_eq!(result.after, "");
    }

    #[test]
    fn truncate_oversized_replacement_priority() {
        // match=5 + repl=9 = 14, budget = 12 - 1 = 11
        // center overflows. left_ellipsis=true, avail = 11 - 1 = 10
        // repl (9) fits in avail (10), match gets 10 - 9 = 1 col from end -> "M"
        let result = TruncatedLine::new("x", "MMMMM", Some("RRRRRRRRR"), "y", 12);
        assert!(result.left_ellipsis);
        assert!(!result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "M");
        assert_eq!(result.replacement, Some("RRRRRRRRR"));
        assert_eq!(result.after, "");
    }

    #[test]
    fn truncate_replacement_itself_overflows() {
        // match=3 + repl=14 = 17, budget = 10 - 1 = 9
        // center overflows. left_ellipsis=true, avail = 9 - 1 = 8
        // repl (14) > avail (8): right_ellipsis=true, repl gets 8 - 1 = 7 cols -> "RRRRRRR"
        // match gets 0
        let result = TruncatedLine::new("x", "MMM", Some("RRRRRRRRRRRRRR"), "y", 10);
        assert!(result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "");
        assert_eq!(result.replacement, Some("RRRRRRR"));
        assert_eq!(result.after, "");
    }

    #[test]
    fn trim_end_context_dependent() {
        // U+FE0F makes the preceding '#' render as a 2-cell emoji, so "#\u{FE0F}" is 2 cells,
        // not the 1 a per-char sum would give. With a 2-col budget only "#\u{FE0F}" fits.
        let (out, trimmed) = trim_end_to_width("#\u{FE0F}ab", 2, false);
        assert_eq!(out, "#\u{FE0F}");
        assert!(trimmed);
    }

    #[test]
    fn trim_start_context_dependent() {
        let (out, trimmed) = trim_start_to_width("ab#\u{FE0F}", 2, false);
        assert_eq!(out, "#\u{FE0F}");
        assert!(trimmed);
    }

    #[test]
    fn truncate_match_only_barely_overflows() {
        // match="ABCDE" (5), budget = 5 - 1 = 4
        // center overflows. left_ellipsis=true, avail = 4 - 1 = 3 -> "CDE"
        let result = TruncatedLine::new("x", "ABCDE", None, "y", 5);
        assert!(result.left_ellipsis);
        assert!(!result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "CDE");
        assert_eq!(result.after, "");
    }
}
