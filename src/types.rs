use std::{borrow::Cow, path::PathBuf};

use crate::preview::data::ContextLine;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchMode {
    #[default]
    CaseAware,
    Literal,
    Regex,
    RegexMultiline,
}

impl MatchMode {
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::CaseAware => Self::Literal,
            Self::Literal => Self::Regex,
            Self::Regex => Self::RegexMultiline,
            Self::RegexMultiline => Self::CaseAware,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MatchKind {
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
pub struct MatchInfo {
    pub byte_offset_start: usize,
    pub byte_offset_end: usize,
    pub match_col_start: usize,
    pub match_col_end: usize,
    pub context_before: Box<[ContextLine]>,
    pub context_after: Box<[ContextLine]>,
    pub skip: bool,
    pub kind: MatchKind,
    /// Captured groups from regex matches. Index 0 = full match ($0), 1..=9 = groups.
    /// Empty in non-regex modes.
    pub captures: Box<[Box<str>]>,
}

impl MatchInfo {
    /// Derive the matched text from the kind and context.
    #[must_use]
    pub fn matched_text(&self) -> Cow<'_, str> {
        match &self.kind {
            MatchKind::SingleLine { line_content, .. } => {
                Cow::Borrowed(&line_content[self.match_col_start..self.match_col_end])
            }
            MatchKind::MultiLine { matched_lines, .. } => {
                let last = matched_lines.len() - 1;
                let mut parts = Vec::with_capacity(matched_lines.len());
                for (i, line) in matched_lines.iter().enumerate() {
                    if i == 0 {
                        parts.push(&line[self.match_col_start..]);
                    } else if i == last {
                        parts.push(&line[..self.match_col_end]);
                    } else {
                        parts.push(line);
                    }
                }
                Cow::Owned(parts.join("\n"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileMatches {
    pub path: PathBuf,
    pub matches: Vec<MatchInfo>,
    pub content_hash: [u8; 32],
}

impl FileMatches {
    #[must_use]
    pub fn active_match_count(&self) -> usize {
        self.matches.iter().filter(|m| !m.skip).count()
    }
}

pub struct SearchRequest {
    pub pattern: String,
    pub mode: MatchMode,
    pub generation: u64,
}

pub enum WorkerCommand {
    Search(SearchRequest),
    Rebuild { include_hidden: bool },
}

pub enum SearchResult {
    FileListReady {
        count: usize,
        truncated: bool,
    },
    FileMatches {
        generation: u64,
        file_matches: FileMatches,
    },
    Complete {
        generation: u64,
        truncated: bool,
    },
    Error {
        generation: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pane {
    #[default]
    SearchInput,
    ReplaceInput,
    FileList,
    Preview,
}

impl Pane {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::SearchInput => Self::ReplaceInput,
            Self::ReplaceInput => Self::FileList,
            Self::FileList => Self::Preview,
            Self::Preview => Self::SearchInput,
        }
    }

    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::SearchInput => Self::Preview,
            Self::ReplaceInput => Self::SearchInput,
            Self::FileList => Self::ReplaceInput,
            Self::Preview => Self::FileList,
        }
    }

    #[must_use]
    pub fn is_input(self) -> bool {
        matches!(self, Self::SearchInput | Self::ReplaceInput)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_mode_toggle() {
        let mut mode = MatchMode::CaseAware;
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::Literal);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::Regex);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::RegexMultiline);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::CaseAware);
    }

    #[test]
    fn match_info_default_skip_is_false() {
        let m = MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 5,
            match_col_start: 0,
            match_col_end: 5,
            context_before: Box::new([]),
            context_after: Box::new([]),
            skip: false,
            kind: MatchKind::SingleLine {
                line_number: 1,
                line_content: "hello world".into(),
            },
            captures: Box::new([]),
        };
        assert!(!m.skip);
    }

    #[test]
    fn file_matches_match_count() {
        let fm = FileMatches {
            path: PathBuf::from("test.rs"),
            matches: vec![
                MatchInfo {
                    byte_offset_start: 0,
                    byte_offset_end: 3,
                    match_col_start: 0,
                    match_col_end: 3,
                    context_before: Box::new([]),
                    context_after: Box::new([]),
                    skip: false,
                    kind: MatchKind::SingleLine {
                        line_number: 1,
                        line_content: "foo bar".into(),
                    },
                    captures: Box::new([]),
                },
                MatchInfo {
                    byte_offset_start: 10,
                    byte_offset_end: 13,
                    match_col_start: 4,
                    match_col_end: 7,
                    context_before: Box::new([]),
                    context_after: Box::new([]),
                    skip: true,
                    kind: MatchKind::SingleLine {
                        line_number: 2,
                        line_content: "baz foo qux".into(),
                    },
                    captures: Box::new([]),
                },
            ],
            content_hash: [0; 32],
        };
        assert_eq!(fm.matches.len(), 2);
        assert_eq!(fm.active_match_count(), 1);
    }

    #[test]
    fn pane_cycle_forward() {
        let mut pane = Pane::SearchInput;
        pane = pane.next();
        assert_eq!(pane, Pane::ReplaceInput);
        pane = pane.next();
        assert_eq!(pane, Pane::FileList);
        pane = pane.next();
        assert_eq!(pane, Pane::Preview);
        pane = pane.next();
        assert_eq!(pane, Pane::SearchInput);
    }

    #[test]
    fn pane_cycle_backward() {
        let mut pane = Pane::SearchInput;
        pane = pane.prev();
        assert_eq!(pane, Pane::Preview);
        pane = pane.prev();
        assert_eq!(pane, Pane::FileList);
    }

    #[test]
    fn match_kind_single_line() {
        let m = MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 5,
            match_col_start: 0,
            match_col_end: 5,
            context_before: Box::new([]),
            context_after: Box::new([]),
            skip: false,
            kind: MatchKind::SingleLine {
                line_number: 1,
                line_content: "hello world".into(),
            },
            captures: Box::new([]),
        };
        assert!(!m.skip);
        assert_eq!(&*m.matched_text(), "hello");
        assert!(matches!(m.kind, MatchKind::SingleLine { .. }));
    }

    #[test]
    fn match_kind_multi_line() {
        let m = MatchInfo {
            byte_offset_start: 0,
            byte_offset_end: 20,
            match_col_start: 0,
            match_col_end: 5,
            context_before: Box::new([]),
            context_after: Box::new([]),
            skip: false,
            kind: MatchKind::MultiLine {
                line_number_start: 1,
                line_number_end: 2,
                matched_lines: vec![Box::from("hello"), Box::from("world")].into(),
            },
            captures: Box::new([]),
        };
        assert!(matches!(m.kind, MatchKind::MultiLine { .. }));
        if let MatchKind::MultiLine {
            line_number_start,
            line_number_end,
            matched_lines,
        } = &m.kind
        {
            assert_eq!(*line_number_start, 1);
            assert_eq!(*line_number_end, 2);
            assert_eq!(matched_lines.len(), 2);
        }
    }
}
