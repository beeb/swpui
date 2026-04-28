use std::{
    fs,
    io::{BufReader, Read},
    path::{Component, Path},
};

use sha2::{Digest as _, Sha256};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr};

use crate::types::{FileMatches, MatchInfo};

pub struct TruncatedLine<'a> {
    pub before: &'a str,
    pub matched: &'a str,
    pub replacement: Option<&'a str>,
    pub after: &'a str,
    pub left_ellipsis: bool,
    pub right_ellipsis: bool,
}

/// Format a file path entry for display, progressively abbreviating to fit `max_width`
/// display columns.
///
/// 1. Full relative path
/// 2. Directory segments abbreviated to 3 characters
/// 2. Directory segments abbreviated to 2 characters
/// 3. Directory segments abbreviated to 1 character
/// 4. Ellipsis in the basename
/// 5. Right-aligned truncation
#[must_use]
pub fn format_file_entry(rel: impl AsRef<Path>, suffix: &str, max_width: usize) -> String {
    let rel = rel.as_ref();
    let mut dirs: Vec<String> = rel
        .components()
        .filter_map(|c| {
            if let Component::Normal(s) = c {
                s.to_str().map(String::from)
            } else {
                None
            }
        })
        .collect();

    let Some(filename) = dirs.pop() else {
        return suffix.to_string();
    };

    // full path
    let path = join_path(&dirs, &filename);
    if fits(&path, suffix, max_width) {
        return with_suffix(&path, suffix);
    }

    // abbreviate directory segments to 3 characters
    for dir in &mut dirs {
        truncate_chars(dir, 3);
    }
    let path = join_path(&dirs, &filename);
    if fits(&path, suffix, max_width) {
        return with_suffix(&path, suffix);
    }

    // abbreviate directory segments to 2 characters
    for dir in &mut dirs {
        truncate_chars(dir, 2);
    }
    let path = join_path(&dirs, &filename);
    if fits(&path, suffix, max_width) {
        return with_suffix(&path, suffix);
    }

    // abbreviate directory segments to 1 character
    for dir in &mut dirs {
        truncate_chars(dir, 1);
    }
    let path = join_path(&dirs, &filename);
    if fits(&path, suffix, max_width) {
        return with_suffix(&path, suffix);
    }

    // ellipsis in the file stem
    let (stem, ext) = split_stem_ext(&filename);
    let dir_prefix = dir_prefix_string(&dirs);
    let stem_chars: Vec<char> = stem.chars().collect();

    if stem_chars.len() > 3 {
        // overhead = dir_prefix + ellipsis + ext + suffix
        let overhead = dir_prefix.width() + 1 + ext.width() + suffix.width();
        let stem_width: usize = stem_chars.iter().filter_map(|c| c.width()).sum();
        if max_width > overhead && max_width - overhead < stem_width {
            let avail_cols = max_width - overhead;
            let (start_n, start_w) =
                chars_within_width(stem_chars.iter().copied(), avail_cols.div_ceil(2));
            let (end_n, _) =
                chars_within_width(stem_chars.iter().rev().copied(), avail_cols - start_w);
            if start_n >= 1 && end_n >= 1 {
                let start: String = stem_chars[..start_n].iter().collect();
                let end: String = stem_chars[stem_chars.len() - end_n..].iter().collect();
                return format!("{dir_prefix}{start}\u{2026}{end}{ext}{suffix}");
            }
        }
    }

    // right-aligned truncation of the most compact form
    let compact_stem = if stem_chars.len() > 3 {
        format!(
            "{}\u{2026}{}",
            stem_chars[0],
            stem_chars[stem_chars.len() - 1]
        )
    } else {
        stem.to_string()
    };
    let compact = format!("{dir_prefix}{compact_stem}{ext}{suffix}");
    let compact_w = compact.width();

    if compact_w <= max_width {
        return compact;
    }

    truncate_left(&compact, compact_w - max_width)
}

#[must_use]
pub fn truncate_match_line<'a>(
    before: &'a str,
    matched: &'a str,
    replacement: Option<&'a str>,
    after: &'a str,
    max_width: usize,
) -> TruncatedLine<'a> {
    let budget = max_width.saturating_sub(1); // account for leading space

    let before_w = before.width();
    let matched_w = matched.width();
    let replacement_w = replacement.map_or(0, UnicodeWidthStr::width);
    let after_w = after.width();
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
        let (before_budget, after_budget) = if before_w + after_w <= remaining {
            // both fit (shouldn't reach here due to step 1, but handle gracefully)
            (before_w, after_w)
        } else if before_w <= remaining && after_w <= remaining {
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

        let (before, left_ellipsis) = trim_start_to_width(before, before_budget);
        let (after, right_ellipsis) = trim_end_to_width(after, after_budget);

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
        let repl_w = repl.width();
        if repl_w <= avail {
            // replacement fits, give remaining to match (from the right end)
            let match_avail = avail - repl_w;
            (slice_end(matched, match_avail), Some(repl), false)
        } else {
            // replacement overflows: truncate from the right, no room for match
            let repl_avail = avail.saturating_sub(1); // reserve 1 col for right ellipsis
            ("", Some(slice_start(repl, repl_avail)), true)
        }
    } else {
        // truncate match from the right end (no replacement)
        (slice_end(matched, avail), None, false)
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

/// Hash the contents of a Reader with SHA256
pub fn hash_content<R: Read>(content: &mut R) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut buf = [0; 1024];
    while let Ok(size) = content.read(&mut buf) {
        if size == 0 {
            break;
        }
        hasher.update(&buf[0..size]);
    }
    hasher.finalize().into()
}

pub fn hash_file(path: impl AsRef<Path>) -> anyhow::Result<[u8; 32]> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    Ok(hash_content(&mut reader))
}

pub fn is_file_stale(path: impl AsRef<Path>, original_hash: [u8; 32]) -> anyhow::Result<bool> {
    Ok(hash_file(path)? != original_hash)
}

/// Logging helper to calculate the size in memory the search results.
pub fn results_mem_bytes(results: &[FileMatches]) -> usize {
    size_of_val(results) + results.iter().map(file_matches_mem_bytes).sum::<usize>()
}

fn chars_within_width(chars: impl Iterator<Item = char>, max_cols: usize) -> (usize, usize) {
    let mut total = 0;
    let count = chars
        .take_while(|c| {
            let w = c.width().unwrap_or_default();
            if total + w > max_cols {
                return false;
            }
            total += w;
            true
        })
        .count();
    (count, total)
}

fn fits(path: &str, suffix: &str, max_width: usize) -> bool {
    path.width() + suffix.width() <= max_width
}

fn with_suffix(path: &str, suffix: &str) -> String {
    format!("{path}{suffix}")
}

fn truncate_chars(s: &mut String, max_chars: usize) {
    if let Some((idx, _)) = s.char_indices().nth(max_chars) {
        s.truncate(idx);
    }
}

fn join_path(dirs: &[String], filename: &str) -> String {
    if dirs.is_empty() {
        filename.to_string()
    } else {
        format!("{}/{filename}", dirs.join("/"))
    }
}

fn dir_prefix_string(dirs: &[String]) -> String {
    if dirs.is_empty() {
        String::new()
    } else {
        format!("{}/", dirs.join("/"))
    }
}

fn split_stem_ext(filename: &str) -> (&str, &str) {
    match filename.rfind('.') {
        Some(pos) if pos > 0 => (&filename[..pos], &filename[pos..]),
        _ => (filename, ""),
    }
}

fn truncate_left(s: &str, excess_cols: usize) -> String {
    let mut removed = 0;
    let mut byte_offset = 0;
    for ch in s.chars() {
        if removed >= excess_cols {
            break;
        }
        removed += ch.width().unwrap_or(0);
        byte_offset += ch.len_utf8();
    }
    s[byte_offset..].to_owned()
}

/// Trim a string from the left to fit within `max_cols` display columns.
/// If trimmed, reserves 1 column for an ellipsis character.
/// Returns `(visible_slice, was_trimmed)`.
fn trim_start_to_width(s: &str, max_cols: usize) -> (&str, bool) {
    let w = s.width();
    if w <= max_cols {
        return (s, false);
    }
    let target = max_cols.saturating_sub(1); // reserve 1 col for ellipsis
    let mut cols = 0;
    let mut byte_start = s.len();
    for (idx, ch) in s.char_indices().rev() {
        let cw = ch.width().unwrap_or(0);
        if cols + cw > target {
            break;
        }
        cols += cw;
        byte_start = idx;
    }
    (&s[byte_start..], true)
}

/// Trim a string from the right to fit within `max_cols` display columns.
/// If trimmed, reserves 1 column for an ellipsis character.
/// Returns `(visible_slice, was_trimmed)`.
fn trim_end_to_width(s: &str, max_cols: usize) -> (&str, bool) {
    let w = s.width();
    if w <= max_cols {
        return (s, false);
    }
    let target = max_cols.saturating_sub(1); // reserve 1 col for ellipsis
    let (char_count, _) = chars_within_width(s.chars(), target);
    let byte_end = s.char_indices().nth(char_count).map_or(s.len(), |(i, _)| i);
    (&s[..byte_end], true)
}

/// Take up to `max_cols` display columns from the end of a string (no ellipsis reservation).
fn slice_end(s: &str, max_cols: usize) -> &str {
    let mut cols = 0;
    let mut byte_start = s.len();
    for (idx, ch) in s.char_indices().rev() {
        let cw = ch.width().unwrap_or(0);
        if cols + cw > max_cols {
            break;
        }
        cols += cw;
        byte_start = idx;
    }
    &s[byte_start..]
}

/// Take up to `max_cols` display columns from the start of a string (no ellipsis reservation).
fn slice_start(s: &str, max_cols: usize) -> &str {
    let (char_count, _) = chars_within_width(s.chars(), max_cols);
    let byte_end = s.char_indices().nth(char_count).map_or(s.len(), |(i, _)| i);
    &s[..byte_end]
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
    fn full_path_when_it_fits() {
        let path = Path::new("src/main.rs");
        assert_eq!(format_file_entry(path, " (3/5)", 50), "src/main.rs (3/5)");
    }

    #[test]
    fn dirs_abbreviated_to_three_chars() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        assert_eq!(
            format_file_entry(path, " (1/2)", 30),
            "src/com/wid/MyFile.rs (1/2)"
        );
    }

    #[test]
    fn dirs_abbreviated_to_two_chars() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        assert_eq!(
            format_file_entry(path, " (1/2)", 26),
            "sr/co/wi/MyFile.rs (1/2)"
        );
    }

    #[test]
    fn dirs_abbreviated_to_one_char() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        assert_eq!(
            format_file_entry(path, " (1/2)", 21),
            "s/c/w/MyFile.rs (1/2)"
        );
    }

    #[test]
    fn ellipsis_in_stem_minimal() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        // overhead = "s/c/w/"(6) + ellipsis(1) + ".rs"(3) + " (1/2)"(6) = 16
        // avail = 18 - 16 = 2 → start=1, end=1 → "M…e"
        assert_eq!(
            format_file_entry(path, " (1/2)", 18),
            "s/c/w/M\u{2026}e.rs (1/2)"
        );
    }

    #[test]
    fn ellipsis_in_stem_with_more_chars() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        // overhead = 16, avail = 20 - 16 = 4 → start=2, end=2 → "My…le"
        assert_eq!(
            format_file_entry(path, " (1/2)", 20),
            "s/c/w/My\u{2026}le.rs (1/2)"
        );
    }

    #[test]
    fn right_aligned_truncation() {
        let path = Path::new("src/components/widgets/MyFile.rs");
        // compact: "s/c/w/M…e.rs (1/2)" = 18 cols, truncate 3
        assert_eq!(
            format_file_entry(path, " (1/2)", 15),
            "/w/M\u{2026}e.rs (1/2)"
        );
    }

    #[test]
    fn no_directories() {
        let path = Path::new("README.md");
        assert_eq!(format_file_entry(path, " (2/3)", 20), "README.md (2/3)");
    }

    #[test]
    fn no_directories_with_ellipsis() {
        let path = Path::new("README.md");
        // overhead = 0 + 1 + 3 + 6 = 10, avail = 13 - 10 = 3 → start=2, end=1 → "RE…E"
        assert_eq!(
            format_file_entry(path, " (2/3)", 13),
            "RE\u{2026}E.md (2/3)"
        );
    }

    #[test]
    fn no_extension() {
        let path = Path::new("src/Makefile");
        assert_eq!(format_file_entry(path, " (1/1)", 17), "sr/Makefile (1/1)");
    }

    #[test]
    fn no_extension_with_ellipsis() {
        let path = Path::new("src/Makefile");
        // dir_prefix="s/"(2), stem="Makefile"(8), ext=""(0), suffix=" (1/1)"(6)
        // overhead = 2 + 1 + 0 + 6 = 9, avail = 13 - 9 = 4 → start=2, end=2 → "Ma…le"
        assert_eq!(
            format_file_entry(path, " (1/1)", 13),
            "s/Ma\u{2026}le (1/1)"
        );
    }

    #[test]
    fn short_stem_skips_ellipsis() {
        let path = Path::new("src/a.rs");
        // stem="a" (1 char, <= 3), ellipsis skipped
        // compact: "s/a.rs (1/1)" = 12, truncate 2
        assert_eq!(format_file_entry(path, " (1/1)", 10), "a.rs (1/1)");
    }

    #[test]
    fn short_dirs_not_over_abbreviated() {
        let path = Path::new("a/b/MyFile.rs");
        assert_eq!(format_file_entry(path, " (1/1)", 25), "a/b/MyFile.rs (1/1)");
    }

    #[test]
    fn very_narrow_truncation() {
        let path = Path::new("src/components/MyFile.rs");
        assert_eq!(format_file_entry(path, " (1/2)", 5), "(1/2)");
    }

    #[test]
    fn empty_path_returns_suffix() {
        let path = Path::new("");
        assert_eq!(format_file_entry(path, " (0/0)", 20), " (0/0)");
    }

    #[test]
    fn dotfile_treated_as_extensionless() {
        let path = Path::new(".gitignore");
        assert_eq!(format_file_entry(path, " (1/1)", 20), ".gitignore (1/1)");
    }

    #[test]
    fn truncate_everything_fits() {
        let result = truncate_match_line("before ", "match", None, " after", 20);
        assert_eq!(result.before, "before ");
        assert_eq!(result.matched, "match");
        assert!(result.replacement.is_none());
        assert_eq!(result.after, " after");
        assert!(!result.left_ellipsis);
        assert!(!result.right_ellipsis);
    }

    #[test]
    fn truncate_everything_fits_with_replacement() {
        let result = truncate_match_line("ab", "cd", Some("XY"), "ef", 20);
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
        let result = truncate_match_line("before ", "match", None, " after", 19);
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
        let result = truncate_match_line("0123456789", "XX", None, "abcdefghij", 15);
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
        let result = truncate_match_line("0123456789", "XX", None, "ab", 10);
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
        let result = truncate_match_line("ab", "XX", None, "0123456789", 10);
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
        let result = truncate_match_line("0123456789", "X", None, "abcdefghij", 13);
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
        let result = truncate_match_line("0123456789", "MM", Some("RR"), "abcdefghij", 15);
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
        let result = truncate_match_line("x", "ABCDEFGHIJKLMNO", None, "y", 10);
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
        let result = truncate_match_line("x", "MMMMM", Some("RRRRRRRRR"), "y", 12);
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
        let result = truncate_match_line("x", "MMM", Some("RRRRRRRRRRRRRR"), "y", 10);
        assert!(result.left_ellipsis);
        assert!(result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "");
        assert_eq!(result.replacement, Some("RRRRRRR"));
        assert_eq!(result.after, "");
    }

    #[test]
    fn truncate_match_only_barely_overflows() {
        // match="ABCDE" (5), budget = 5 - 1 = 4
        // center overflows. left_ellipsis=true, avail = 4 - 1 = 3 -> "CDE"
        let result = truncate_match_line("x", "ABCDE", None, "y", 5);
        assert!(result.left_ellipsis);
        assert!(!result.right_ellipsis);
        assert_eq!(result.before, "");
        assert_eq!(result.matched, "CDE");
        assert_eq!(result.after, "");
    }

    #[test]
    fn stale_file_detected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "original content").unwrap();
        let hash = hash_file(&path).unwrap();

        // modify the file externally
        fs::write(&path, "modified content").unwrap();

        assert!(is_file_stale(&path, hash).unwrap());
    }

    #[test]
    fn fresh_file_not_stale() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "original content").unwrap();
        let hash = hash_file(&path).unwrap();

        assert!(!is_file_stale(&path, hash).unwrap());
    }
}
