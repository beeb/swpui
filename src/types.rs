use ratatui::layout::{Position, Rect};

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
    IncludeInput,
    ExcludeInput,
    FileList,
    Preview,
}

impl Pane {
    #[must_use]
    pub fn next(self, filter_view: bool) -> Self {
        match self {
            Self::SearchInput | Self::IncludeInput => {
                if filter_view {
                    Self::ExcludeInput
                } else {
                    Self::ReplaceInput
                }
            }
            Self::ReplaceInput | Self::ExcludeInput => Self::FileList,
            Self::FileList => Self::Preview,
            Self::Preview => {
                if filter_view {
                    Self::IncludeInput
                } else {
                    Self::SearchInput
                }
            }
        }
    }

    /// Previous pane in the cycle; only the input pair visible in the current view is included.
    #[must_use]
    pub fn prev(self, filter_view: bool) -> Self {
        match self {
            Self::SearchInput | Self::IncludeInput => Self::Preview,
            Self::ReplaceInput => Self::SearchInput,
            Self::ExcludeInput => Self::IncludeInput,
            Self::FileList => {
                if filter_view {
                    Self::ExcludeInput
                } else {
                    Self::ReplaceInput
                }
            }
            Self::Preview => Self::FileList,
        }
    }

    #[must_use]
    pub fn is_input(self) -> bool {
        matches!(
            self,
            Self::SearchInput | Self::ReplaceInput | Self::IncludeInput | Self::ExcludeInput
        )
    }

    #[must_use]
    pub fn digit(self) -> char {
        match self {
            Self::SearchInput => '1',
            Self::ReplaceInput => '2',
            Self::FileList => '3',
            Self::Preview => '4',
            Self::IncludeInput => '5',
            Self::ExcludeInput => '6',
        }
    }

    #[must_use]
    pub fn from_digit(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::SearchInput),
            '2' => Some(Self::ReplaceInput),
            '3' => Some(Self::FileList),
            '4' => Some(Self::Preview),
            '5' => Some(Self::IncludeInput),
            '6' => Some(Self::ExcludeInput),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PaneAreas {
    pub search_input: Rect,
    pub replace_input: Rect,
    pub include_input: Rect,
    pub exclude_input: Rect,
    pub file_list: Rect,
    pub preview: Rect,
}

impl PaneAreas {
    /// Return the pane whose rectangle contains `pos`, if any.
    ///
    /// The hidden input pair's rects are zeroed each frame, and an empty `Rect` contains
    /// no position, so hidden panes can never be hit.
    #[must_use]
    pub fn pane_at(&self, pos: Position) -> Option<Pane> {
        if self.search_input.contains(pos) {
            Some(Pane::SearchInput)
        } else if self.replace_input.contains(pos) {
            Some(Pane::ReplaceInput)
        } else if self.include_input.contains(pos) {
            Some(Pane::IncludeInput)
        } else if self.exclude_input.contains(pos) {
            Some(Pane::ExcludeInput)
        } else if self.file_list.contains(pos) {
            Some(Pane::FileList)
        } else if self.preview.contains(pos) {
            Some(Pane::Preview)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_at_hits_correct_pane() {
        use ratatui::layout::{Position, Rect};
        let areas = PaneAreas {
            search_input: Rect::new(0, 0, 10, 3),
            replace_input: Rect::new(0, 3, 10, 3),
            include_input: Rect::default(),
            exclude_input: Rect::default(),
            file_list: Rect::new(0, 6, 10, 10),
            preview: Rect::new(10, 0, 20, 16),
        };
        assert_eq!(areas.pane_at(Position::new(5, 1)), Some(Pane::SearchInput));
        assert_eq!(areas.pane_at(Position::new(5, 4)), Some(Pane::ReplaceInput));
        assert_eq!(areas.pane_at(Position::new(5, 10)), Some(Pane::FileList));
        assert_eq!(areas.pane_at(Position::new(15, 5)), Some(Pane::Preview));
        assert_eq!(areas.pane_at(Position::new(50, 50)), None);
    }

    #[test]
    fn pane_at_filter_view() {
        use ratatui::layout::{Position, Rect};
        let areas = PaneAreas {
            search_input: Rect::default(),
            replace_input: Rect::default(),
            include_input: Rect::new(0, 0, 10, 3),
            exclude_input: Rect::new(0, 3, 10, 3),
            file_list: Rect::new(0, 6, 10, 10),
            preview: Rect::new(10, 0, 20, 16),
        };
        assert_eq!(areas.pane_at(Position::new(5, 1)), Some(Pane::IncludeInput));
        assert_eq!(areas.pane_at(Position::new(5, 4)), Some(Pane::ExcludeInput));
    }

    #[test]
    fn pane_cycle_forward() {
        let mut pane = Pane::SearchInput;
        pane = pane.next(false);
        assert_eq!(pane, Pane::ReplaceInput);
        pane = pane.next(false);
        assert_eq!(pane, Pane::FileList);
        pane = pane.next(false);
        assert_eq!(pane, Pane::Preview);
        pane = pane.next(false);
        assert_eq!(pane, Pane::SearchInput);
    }

    #[test]
    fn pane_cycle_backward() {
        let mut pane = Pane::SearchInput;
        pane = pane.prev(false);
        assert_eq!(pane, Pane::Preview);
        pane = pane.prev(false);
        assert_eq!(pane, Pane::FileList);
    }

    #[test]
    fn pane_cycle_filter_view() {
        let mut pane = Pane::IncludeInput;
        pane = pane.next(true);
        assert_eq!(pane, Pane::ExcludeInput);
        pane = pane.next(true);
        assert_eq!(pane, Pane::FileList);
        pane = pane.next(true);
        assert_eq!(pane, Pane::Preview);
        pane = pane.next(true);
        assert_eq!(pane, Pane::IncludeInput);
        assert_eq!(Pane::IncludeInput.prev(true), Pane::Preview);
        assert_eq!(Pane::FileList.prev(true), Pane::ExcludeInput);
        assert_eq!(Pane::ExcludeInput.prev(true), Pane::IncludeInput);
    }

    #[test]
    fn filter_pane_digits() {
        assert_eq!(Pane::IncludeInput.digit(), '5');
        assert_eq!(Pane::ExcludeInput.digit(), '6');
        assert_eq!(Pane::from_digit('5'), Some(Pane::IncludeInput));
        assert_eq!(Pane::from_digit('6'), Some(Pane::ExcludeInput));
        assert!(Pane::IncludeInput.is_input());
        assert!(Pane::ExcludeInput.is_input());
    }
}
