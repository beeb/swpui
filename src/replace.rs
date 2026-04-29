use std::{borrow::Cow, cmp::Reverse, io::Write as _, ops::Range, path::Path};

use convert_case::{Case, Casing as _};

use crate::types::{MatchInfo, MatchMode};

/// Cases to detect, ordered from least specific to most specific.
const CASES: [Case<'static>; 6] = [
    Case::Flat,
    Case::Snake,
    Case::Camel,
    Case::Kebab,
    Case::Pascal,
    Case::UpperSnake,
];

/// Adjust replacement text according to casing of the match (expanded to word boundaries).
#[must_use]
pub fn case_aware_replacement<'a>(match_word: &str, replacement: &'a str) -> Cow<'a, str> {
    if match_word.is_empty() || replacement.is_empty() {
        return Cow::Borrowed(replacement);
    }

    let Some(matched_case) = detect_case(match_word) else {
        return Cow::Borrowed(replacement);
    };

    // detect the replacement's case so that convert_case parses word boundaries correctly
    // before converting to the matched case
    let repl_case = detect_case(replacement);

    // if the matched text is `Flat` but the replacement is in a more specific lowercase case
    // (snake, kebab, camel), respect the replacement's case as-is
    if matched_case == Case::Flat
        && repl_case.is_some_and(|c| matches!(c, Case::Snake | Case::Kebab | Case::Camel))
    {
        return Cow::Borrowed(replacement);
    }

    let converted = if let Some(from_case) = repl_case {
        replacement.from_case(from_case).to_case(matched_case)
    } else {
        replacement.to_case(matched_case)
    };
    if converted == replacement {
        return Cow::Borrowed(replacement);
    }
    Cow::Owned(converted)
}

#[must_use]
pub fn apply_replacements(
    content: impl Into<String>,
    matches: &[MatchInfo],
    replacement: &str,
    mode: MatchMode,
) -> String {
    let mut active: Vec<&MatchInfo> = matches.iter().filter(|m| !m.skip).collect();
    let mut result = content.into();
    if active.is_empty() {
        return result;
    }

    // sort by byte offset in descending order so we can replace from the end
    active.sort_unstable_by_key(|m| Reverse(m.byte_offset_start));

    for m in active {
        let expanded = expand_captures(replacement, &m.captures);
        let match_range = m.byte_offset_start..m.byte_offset_end;
        let repl = if mode == MatchMode::CaseAware {
            let word_range = expand_to_word(&result, match_range.clone());
            case_aware_replacement(&result[word_range], &expanded)
        } else {
            expanded
        };
        result.replace_range(match_range, &repl);
    }
    result
}

#[must_use]
pub fn has_overlapping_matches(matches: &[MatchInfo]) -> bool {
    let mut active: Vec<&MatchInfo> = matches.iter().filter(|m| !m.skip).collect();
    active.sort_unstable_by_key(|m| m.byte_offset_start);
    active
        .array_windows()
        .any(|[w0, w1]| w0.byte_offset_end > w1.byte_offset_start)
}

pub fn write_file(path: impl AsRef<Path>, content: &str) -> anyhow::Result<()> {
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(content.as_bytes())?;
    tmp.persist(path)?;
    Ok(())
}

/// Detect the case of a string by trying each case from least to most specific.
fn detect_case(s: &str) -> Option<Case<'static>> {
    CASES.iter().copied().find(|&case| s == s.to_case(case))
}

/// Expand the byte range to cover the contiguous identifier surrounding the match.
///
/// Walks left and right by alphanumeric characters, `_`, and `-`. Leading and trailing
/// `_` or `-` in the *context* (not the matched bytes themselves) are trimmed so that
/// separators only count when they sit between alphanumeric characters.
fn expand_to_word(content: &str, range: Range<usize>) -> Range<usize> {
    let Range { start, end } = range;
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';

    // walk left from `start`, taking extending chars right-to-left
    let new_start = content[..start]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(start, |(i, _)| i);

    // walk right from `end`, taking extending chars left-to-right
    let new_end = content[end..]
        .char_indices()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(end, |(i, c)| end + i + c.len_utf8());

    // trim leading `_`/`-` from the expansion (up to original match)
    let new_start = content[new_start..start]
        .char_indices()
        .find(|&(_, c)| c.is_alphanumeric())
        .map_or(start, |(i, _)| new_start + i);

    // trim trailing `_`/`-` from the expansion (after the original match)
    let new_end = content[end..new_end]
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_alphanumeric())
        .map_or(end, |(i, c)| end + i + c.len_utf8());

    new_start..new_end
}

/// Return the effective replacement string, expanding escape sequences when in `RegexMultiline` mode.
#[must_use]
pub fn effective_replacement(raw: &str, mode: MatchMode) -> Cow<'_, str> {
    if mode == MatchMode::RegexMultiline {
        expand_escape_sequences(raw)
    } else {
        Cow::Borrowed(raw)
    }
}

/// Expand capture group references (`$0`-`$9`) in a replacement template.
///
/// `$$` produces a literal `$`. References to non-participating groups produce an empty string.
/// Returns a borrowed slice when no `$` is present.
#[must_use]
pub fn expand_captures<'a>(template: &'a str, captures: &[Box<str>]) -> Cow<'a, str> {
    if !template.contains('$') || captures.is_empty() {
        return Cow::Borrowed(template);
    }
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some('$') => {
                    chars.next();
                    result.push('$');
                }
                Some(&d) if d.is_ascii_digit() => {
                    chars.next();
                    let idx = (d as u8 - b'0') as usize;
                    if let Some(cap) = captures.get(idx) {
                        result.push_str(cap);
                    }
                }
                _ => result.push('$'),
            }
        } else {
            result.push(c);
        }
    }
    Cow::Owned(result)
}

/// Expand escape sequences in a string (`\n`, `\r`, `\t`, `\\`).
/// Unknown escape sequences are preserved as-is (e.g. `\x` stays `\x`).
/// Returns a borrowed slice when no escape sequences are present.
#[must_use]
pub fn expand_escape_sequences(s: &str) -> Cow<'_, str> {
    if !s.contains('\\') {
        return Cow::Borrowed(s);
    }
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') | None => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
            }
        } else {
            result.push(c);
        }
    }
    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn make_match(start: usize, end: usize) -> MatchInfo {
        MatchInfo {
            byte_offset_start: start,
            byte_offset_end: end,
            skip: false,
            captures: Box::new([]),
        }
    }

    fn make_skipped_match(start: usize, end: usize) -> MatchInfo {
        MatchInfo {
            skip: true,
            ..make_match(start, end)
        }
    }

    #[test]
    fn apply_single_replacement() {
        let content = "hello world";
        let matches = vec![make_match(6, 11)];
        let result = apply_replacements(content, &matches, "rust", MatchMode::Literal);
        assert_eq!(result, "hello rust");
    }

    #[test]
    fn apply_multiple_replacements() {
        let content = "foo bar foo baz foo";
        let matches = vec![make_match(0, 3), make_match(8, 11), make_match(16, 19)];
        let result = apply_replacements(content, &matches, "qux", MatchMode::Literal);
        assert_eq!(result, "qux bar qux baz qux");
    }

    #[test]
    fn skipped_matches_are_not_replaced() {
        let content = "foo bar foo";
        let matches = vec![make_match(0, 3), make_skipped_match(8, 11)];
        let result = apply_replacements(content, &matches, "baz", MatchMode::Literal);
        assert_eq!(result, "baz bar foo");
    }

    #[test]
    fn empty_replacement_deletes_text() {
        let content = "hello world";
        let matches = vec![make_match(5, 11)];
        let result = apply_replacements(content, &matches, "", MatchMode::Literal);
        assert_eq!(result, "hello");
    }

    #[test]
    fn no_active_matches_returns_original() {
        let content = "hello world";
        let matches = vec![make_skipped_match(0, 5)];
        let result = apply_replacements(content, &matches, "bye", MatchMode::Literal);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn detect_overlapping_matches() {
        let matches = vec![make_match(0, 5), make_match(3, 8)];
        assert!(has_overlapping_matches(&matches));
    }

    #[test]
    fn non_overlapping_matches() {
        let matches = vec![make_match(0, 3), make_match(5, 8)];
        assert!(!has_overlapping_matches(&matches));
    }

    #[test]
    fn adjacent_matches_are_not_overlapping() {
        let matches = vec![make_match(0, 3), make_match(3, 6)];
        assert!(!has_overlapping_matches(&matches));
    }

    #[test]
    fn write_file_atomically_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "original").unwrap();
        let result = write_file(&path, "replaced");
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "replaced");
    }

    #[test]
    fn case_aware_snake_to_upper_snake() {
        assert_eq!(
            case_aware_replacement("FOO_BAR", "baz_qux").as_ref(),
            "BAZ_QUX"
        );
    }

    #[test]
    fn case_aware_snake_to_kebab() {
        assert_eq!(
            case_aware_replacement("foo-bar", "baz_qux").as_ref(),
            "baz-qux"
        );
    }

    #[test]
    fn case_aware_pascal_to_pascal() {
        assert_eq!(
            case_aware_replacement("FooBar", "BazQux").as_ref(),
            "BazQux"
        );
    }

    #[test]
    fn case_aware_pascal_to_camel() {
        assert_eq!(
            case_aware_replacement("fooBar", "BazQux").as_ref(),
            "bazQux"
        );
    }

    #[test]
    fn case_aware_flat_preserves_snake_replacement() {
        assert_eq!(
            case_aware_replacement("foobar", "baz_qux").as_ref(),
            "baz_qux"
        );
        assert!(matches!(
            case_aware_replacement("foobar", "baz_qux"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn case_aware_flat_to_flat() {
        assert_eq!(
            case_aware_replacement("foobar", "bazqux").as_ref(),
            "bazqux"
        );
    }

    #[test]
    fn case_aware_flat_converts_pascal_replacement() {
        assert_eq!(
            case_aware_replacement("foobar", "BazQux").as_ref(),
            "bazqux"
        );
    }

    #[test]
    fn case_aware_same_case_no_change() {
        assert_eq!(
            case_aware_replacement("foo_bar", "baz_qux").as_ref(),
            "baz_qux"
        );
        assert!(matches!(
            case_aware_replacement("foo_bar", "baz_qux"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn case_aware_empty_matched() {
        assert_eq!(case_aware_replacement("", "bar").as_ref(), "bar");
        assert!(matches!(
            case_aware_replacement("", "bar"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn case_aware_empty_replacement() {
        assert_eq!(case_aware_replacement("Foo", "").as_ref(), "");
        assert!(matches!(
            case_aware_replacement("Foo", ""),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn case_aware_single_word_pascal() {
        assert_eq!(case_aware_replacement("Hello", "world").as_ref(), "World");
    }

    #[test]
    fn case_aware_single_word_upper() {
        assert_eq!(case_aware_replacement("HELLO", "world").as_ref(), "WORLD");
    }

    #[test]
    fn expand_to_word_camel_case_middle() {
        assert_eq!(expand_to_word("fooBar", 0..3), 0..6);
    }

    #[test]
    fn expand_to_word_pascal_case_middle() {
        assert_eq!(expand_to_word("FooBar", 3..6), 0..6);
    }

    #[test]
    fn expand_to_word_snake_case_middle() {
        assert_eq!(expand_to_word("foo_bar_baz", 4..7), 0..11);
    }

    #[test]
    fn expand_to_word_kebab_case_middle() {
        assert_eq!(expand_to_word("foo-bar-baz", 4..7), 0..11);
    }

    #[test]
    fn expand_to_word_upper_snake_middle() {
        assert_eq!(expand_to_word("FOO_BAR_BAZ", 4..7), 0..11);
    }

    #[test]
    fn expand_to_word_match_at_left_edge_of_identifier() {
        assert_eq!(expand_to_word("fooBar", 0..3), 0..6);
    }

    #[test]
    fn expand_to_word_match_at_right_edge_of_identifier() {
        assert_eq!(expand_to_word("fooBar", 3..6), 0..6);
    }

    #[test]
    fn expand_to_word_trims_leading_underscore_outside_match() {
        assert_eq!(expand_to_word("_foo", 1..4), 1..4);
    }

    #[test]
    fn expand_to_word_trims_trailing_underscore_outside_match() {
        assert_eq!(expand_to_word("foo_", 0..3), 0..3);
    }

    #[test]
    fn expand_to_word_trims_leading_hyphen_outside_match() {
        assert_eq!(expand_to_word("-foo", 1..4), 1..4);
    }

    #[test]
    fn expand_to_word_keeps_separator_in_match_itself() {
        assert_eq!(expand_to_word("_foo", 0..4), 0..4);
    }

    #[test]
    fn expand_to_word_match_at_start_of_content() {
        assert_eq!(expand_to_word("fooBar", 0..3), 0..6);
    }

    #[test]
    fn expand_to_word_match_at_end_of_content() {
        assert_eq!(expand_to_word("fooBar", 3..6), 0..6);
    }

    #[test]
    fn expand_to_word_match_surrounded_by_whitespace() {
        assert_eq!(expand_to_word("  foo  ", 2..5), 2..5);
    }

    #[test]
    fn expand_to_word_match_surrounded_by_punctuation() {
        assert_eq!(expand_to_word("(foo)", 1..4), 1..4);
    }

    #[test]
    fn expand_to_word_does_not_cross_whitespace() {
        assert_eq!(expand_to_word("foo bar", 0..3), 0..3);
    }

    #[test]
    fn expand_to_word_match_is_separator_only() {
        assert_eq!(expand_to_word("foo_bar", 3..4), 0..7);
    }

    #[test]
    fn expand_to_word_match_contains_path_separator() {
        assert_eq!(expand_to_word("x_foo::bar_y", 2..10), 0..12);
    }

    #[test]
    fn expand_to_word_unicode_alphanumeric_neighbour() {
        assert_eq!(expand_to_word("é_foo", 3..6), 0..6);
    }

    #[test]
    fn expand_to_word_empty_content() {
        assert_eq!(expand_to_word("", 0..0), 0..0);
    }

    #[test]
    fn case_aware_apply_replacements() {
        let content = "Hello hello";
        let matches = vec![make_match(0, 5), make_match(6, 11)];
        let result = apply_replacements(content, &matches, "world", MatchMode::CaseAware);
        assert_eq!(result, "World world");
    }

    #[test]
    fn case_aware_substring_in_camel_case_identifier() {
        let content = "fooBar";
        let matches = vec![make_match(0, 3)];
        let result = apply_replacements(content, &matches, "new_thing", MatchMode::CaseAware);
        assert_eq!(result, "newThingBar");
    }

    #[test]
    fn case_aware_substring_in_upper_snake_identifier() {
        let content = "FOO_BAR_BAZ";
        let matches = vec![make_match(4, 7)];
        let result = apply_replacements(content, &matches, "new_thing", MatchMode::CaseAware);
        assert_eq!(result, "FOO_NEW_THING_BAZ");
    }

    #[test]
    fn case_aware_substring_in_kebab_case_identifier() {
        let content = "foo-bar-baz";
        let matches = vec![make_match(4, 7)];
        let result = apply_replacements(content, &matches, "qux_thing", MatchMode::CaseAware);
        assert_eq!(result, "foo-qux-thing-baz");
    }

    #[test]
    fn case_aware_substring_in_pascal_case_identifier() {
        let content = "FooBar";
        let matches = vec![make_match(0, 3)];
        let result = apply_replacements(content, &matches, "new_thing", MatchMode::CaseAware);
        assert_eq!(result, "NewThingBar");
    }

    #[test]
    fn expand_no_escapes_borrows() {
        let s = "hello world";
        assert!(matches!(expand_escape_sequences(s), Cow::Borrowed(_)));
    }

    #[test]
    fn expand_newline() {
        assert_eq!(
            expand_escape_sequences(r"\n"),
            Cow::Owned::<str>("\n".to_string())
        );
    }

    #[test]
    fn expand_carriage_return() {
        assert_eq!(
            expand_escape_sequences(r"\r"),
            Cow::Owned::<str>("\r".to_string())
        );
    }

    #[test]
    fn expand_tab() {
        assert_eq!(
            expand_escape_sequences(r"\t"),
            Cow::Owned::<str>("\t".to_string())
        );
    }

    #[test]
    fn expand_backslash() {
        assert_eq!(
            expand_escape_sequences(r"\\"),
            Cow::Owned::<str>("\\".to_string())
        );
    }

    #[test]
    fn expand_unknown_escape_preserved() {
        assert_eq!(
            expand_escape_sequences(r"\x"),
            Cow::Owned::<str>(r"\x".to_string())
        );
    }

    #[test]
    fn expand_trailing_backslash_preserved() {
        assert_eq!(
            expand_escape_sequences("foo\\"),
            Cow::Owned::<str>("foo\\".to_string())
        );
    }

    #[test]
    fn expand_combination() {
        assert_eq!(
            expand_escape_sequences(r"line1\nline2\ttabbed"),
            Cow::Owned::<str>("line1\nline2\ttabbed".to_string())
        );
    }

    #[test]
    fn captures_no_dollar_borrows() {
        let caps: Box<[Box<str>]> = vec![Box::from("full")].into();
        assert!(matches!(
            expand_captures("no refs", &caps),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn captures_empty_captures_borrows() {
        assert!(matches!(expand_captures("$1 ref", &[]), Cow::Borrowed(_)));
    }

    #[test]
    fn captures_expand_group() {
        let caps: Box<[Box<str>]> =
            vec![Box::from("foo bar"), Box::from("foo"), Box::from("bar")].into();
        assert_eq!(expand_captures("$2-$1", &caps).as_ref(), "bar-foo");
    }

    #[test]
    fn captures_expand_full_match() {
        let caps: Box<[Box<str>]> = vec![Box::from("hello")].into();
        assert_eq!(expand_captures("[$0]", &caps).as_ref(), "[hello]");
    }

    #[test]
    fn captures_dollar_escape() {
        let caps: Box<[Box<str>]> = vec![Box::from("x")].into();
        assert_eq!(expand_captures("$$0", &caps).as_ref(), "$0");
    }

    #[test]
    fn captures_missing_group() {
        let caps: Box<[Box<str>]> = vec![Box::from("x")].into();
        assert_eq!(expand_captures("$1$9", &caps).as_ref(), "");
    }

    #[test]
    fn captures_trailing_dollar() {
        let caps: Box<[Box<str>]> = vec![Box::from("x")].into();
        assert_eq!(expand_captures("end$", &caps).as_ref(), "end$");
    }

    #[test]
    fn captures_apply_replacements_regex() {
        let content = "foo bar";
        let matches = vec![MatchInfo {
            captures: vec![Box::from("foo bar"), Box::from("foo"), Box::from("bar")].into(),
            ..make_match(0, 7)
        }];
        let result = apply_replacements(content, &matches, "$2 $1", MatchMode::Regex);
        assert_eq!(result, "bar foo");
    }
}
