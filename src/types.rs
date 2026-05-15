use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub match_mode: MatchMode,
    pub include_hidden: bool,
    pub include_gitignored: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            match_mode: MatchMode::default(),
            include_hidden: true,
            include_gitignored: false,
        }
    }
}

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

    #[must_use]
    pub fn is_regex(&self) -> bool {
        match self {
            MatchMode::CaseAware | MatchMode::Literal => false,
            MatchMode::Regex | MatchMode::RegexMultiline => true,
        }
    }
}

impl fmt::Display for MatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let disp = match self {
            MatchMode::CaseAware => "case-aware",
            MatchMode::Literal => "literal",
            MatchMode::Regex => "regex",
            MatchMode::RegexMultiline => "regex multiline",
        };
        f.write_str(disp)
    }
}

/// A half-open `[start, end)` byte range within a file's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    #[must_use]
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    #[must_use]
    pub fn as_range(self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub byte_range: ByteRange,
    pub skip: bool,
    /// Captured groups from regex matches. Index 0 = full match ($0), 1..=9 = groups.
    /// Empty in non-regex modes.
    pub captures: Box<[Box<str>]>,
}

impl MatchInfo {
    #[must_use]
    pub fn new(byte_range: ByteRange, captures: Box<[Box<str>]>) -> Self {
        Self {
            byte_range,
            skip: false,
            captures,
        }
    }
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

    #[must_use]
    pub fn digit(self) -> char {
        match self {
            Self::SearchInput => '1',
            Self::ReplaceInput => '2',
            Self::FileList => '3',
            Self::Preview => '4',
        }
    }

    #[must_use]
    pub fn from_digit(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::SearchInput),
            '2' => Some(Self::ReplaceInput),
            '3' => Some(Self::FileList),
            '4' => Some(Self::Preview),
            _ => None,
        }
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
}
