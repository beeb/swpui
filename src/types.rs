use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

use crate::path::ResponsivePath;

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

#[derive(Debug, Clone)]
pub struct FileMatches {
    pub path: PathBuf,
    pub responsive_path: Option<ResponsivePath>,
    pub matches: Vec<MatchInfo>,
    pub hash: FileHash,
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

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct FileHash([u8; 32]);

impl From<[u8; 32]> for FileHash {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl FileHash {
    /// Hash the contents of a file.
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        Ok(Self::digest(&mut reader))
    }

    /// Hash the bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        Self(hash)
    }

    /// Hash the bytes produced by a reader.
    pub fn digest<R: Read>(content: &mut R) -> Self {
        let mut hasher = Sha256::new();
        let mut buf = [0; 1024];
        while let Ok(size) = content.read(&mut buf) {
            if size == 0 {
                break;
            }
            hasher.update(&buf[0..size]);
        }
        Self(hasher.finalize().into())
    }

    /// Check whether the hash matches the current contents of a file.
    pub fn matches(&self, path: impl AsRef<Path>) -> anyhow::Result<bool> {
        Ok(&Self::new(path)? == self)
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
    fn file_matches_match_count() {
        let fm = FileMatches {
            path: PathBuf::from("test.rs"),
            responsive_path: None,
            matches: vec![
                MatchInfo {
                    byte_offset_start: 0,
                    byte_offset_end: 3,
                    skip: false,
                    captures: Box::new([]),
                },
                MatchInfo {
                    byte_offset_start: 10,
                    byte_offset_end: 13,
                    skip: true,
                    captures: Box::new([]),
                },
            ],
            hash: FileHash::default(),
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
}
