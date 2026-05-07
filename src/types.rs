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

#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub byte_offset_start: usize,
    pub byte_offset_end: usize,
    pub skip: bool,

    /// Captured groups from regex matches. Index 0 = full match ($0), 1..=9 = groups.
    /// Empty in non-regex modes.
    pub captures: Box<[Box<str>]>,
}

impl MatchInfo {
    #[must_use]
    pub fn new(byte_start: usize, byte_end: usize, captures: Box<[Box<str>]>) -> Self {
        Self {
            byte_offset_start: byte_start,
            byte_offset_end: byte_end,
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
