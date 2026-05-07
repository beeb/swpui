use std::{
    fs,
    num::NonZero,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
};

pub const MAX_FILES: usize = 250_000;
pub const MAX_MATCHES: usize = 1_000_000;

use ignore::{WalkBuilder, WalkState};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use regex::Regex;
use tracing::debug;

use crate::{
    hash::FileHash,
    path::ResponsivePath,
    types::{MatchInfo, MatchMode, Options},
};

pub struct SearchRequest {
    pub pattern: String,
    pub mode: MatchMode,
    pub generation: u64,
}

pub enum WorkerCommand {
    Search(SearchRequest),
    Rebuild(WalkOptions),
}

#[derive(Debug, Clone, Copy)]
pub struct WalkOptions {
    pub include_hidden: bool,
    pub include_gitignored: bool,
}

impl From<Options> for WalkOptions {
    fn from(options: Options) -> Self {
        Self {
            include_hidden: options.include_hidden,
            include_gitignored: options.include_gitignored,
        }
    }
}

impl From<SearchRequest> for WorkerCommand {
    fn from(value: SearchRequest) -> Self {
        WorkerCommand::Search(value)
    }
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

#[derive(Clone, Debug)]
pub enum Pattern<'a> {
    Empty,
    Literal(&'a [u8]),
    Regex(Regex),
}

impl<'a> Pattern<'a> {
    pub fn new(pattern: &'a str, mode: MatchMode) -> anyhow::Result<Self> {
        if pattern.is_empty() {
            return Ok(Pattern::Empty);
        }
        Ok(match mode {
            MatchMode::Literal => Pattern::Literal(pattern.as_bytes()),
            MatchMode::Regex => {
                Pattern::Regex(regex::RegexBuilder::new(pattern).crlf(true).build()?)
            }
            MatchMode::CaseAware => Pattern::Regex(
                regex::RegexBuilder::new(&regex::escape(pattern))
                    .case_insensitive(true)
                    .build()?,
            ),
            MatchMode::RegexMultiline => Pattern::Regex(
                regex::RegexBuilder::new(pattern)
                    .dot_matches_new_line(true)
                    .multi_line(true)
                    .crlf(true)
                    .build()?,
            ),
        })
    }
}

pub struct SearchWorker {
    root: PathBuf,
    cmd_rx: Receiver<WorkerCommand>,
    result_tx: Sender<SearchResult>,
    cancelled: Arc<AtomicBool>,
    file_list: Vec<PathBuf>,
    pool: rayon::ThreadPool,
    walk_options: WalkOptions,
}

impl SearchWorker {
    pub fn new(
        root: PathBuf,
        cmd_rx: Receiver<WorkerCommand>,
        result_tx: Sender<SearchResult>,
        cancelled: Arc<AtomicBool>,
        walk_options: WalkOptions,
    ) -> anyhow::Result<Self> {
        let threads = std::thread::available_parallelism()
            .map_or(1, NonZero::get)
            .min(12);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        Ok(Self {
            root,
            cmd_rx,
            result_tx,
            cancelled,
            file_list: Vec::new(),
            pool,
            walk_options,
        })
    }

    pub fn run(mut self) {
        self.walk_files();
        while let Ok(mut cmd) = self.cmd_rx.recv() {
            // skip to the latest queued search command (makes cancelling faster)
            // but don't ignore rebuild commands
            while let Ok(newer) = self.cmd_rx.try_recv() {
                if let WorkerCommand::Rebuild(opts) = cmd {
                    self.walk_options = opts;
                    self.walk_files();
                }
                cmd = newer;
            }
            match cmd {
                WorkerCommand::Search(request) => {
                    self.cancelled.store(false, Ordering::Relaxed);
                    self.execute_search(&request);
                }
                WorkerCommand::Rebuild(opts) => {
                    self.walk_options = opts;
                    self.walk_files();
                }
            }
        }
    }

    fn walk_files(&mut self) {
        let (tx, rx) = mpsc::channel();
        let threads = std::thread::available_parallelism()
            .map_or(1, NonZero::get)
            .min(12);
        let WalkOptions {
            include_hidden,
            include_gitignored,
        } = self.walk_options;
        let walker = WalkBuilder::new(&self.root)
            .filter_entry(|entry| {
                !(entry.path().is_dir() && entry.path().file_name().unwrap_or_default() == ".git")
            })
            .hidden(!include_hidden)
            .git_ignore(!include_gitignored)
            .git_global(!include_gitignored)
            .git_exclude(!include_gitignored)
            .ignore(!include_gitignored)
            .parents(!include_gitignored)
            .threads(threads)
            .build_parallel();
        let file_count = &AtomicUsize::new(0);
        walker.run(|| {
            let tx = tx.clone();
            Box::new(move |entry| {
                if file_count.load(Ordering::Relaxed) >= MAX_FILES {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return WalkState::Continue;
                }
                file_count.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(entry.into_path());
                WalkState::Continue
            })
        });
        drop(tx);
        self.file_list = rx.into_iter().collect();
        let count = self.file_list.len();
        let truncated = count >= MAX_FILES;
        debug!(
            len = count,
            capacity = self.file_list.capacity(),
            mem_bytes = self.file_list.capacity() * size_of::<PathBuf>()
                + self.file_list.iter().map(PathBuf::capacity).sum::<usize>(),
            truncated,
            "walk_files complete"
        );
        let _ = self
            .result_tx
            .send(SearchResult::FileListReady { count, truncated });
    }

    fn execute_search(&self, request: &SearchRequest) {
        debug!(
            pattern = %request.pattern,
            generation = request.generation,
            file_count = self.file_list.len(),
            "search started"
        );
        let pattern = match Pattern::new(&request.pattern, request.mode) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                let _ = self.result_tx.send(SearchResult::Error {
                    generation: request.generation,
                    message: e.to_string(),
                });
                return;
            }
        };

        let match_count = AtomicUsize::new(0);
        let cancelled = &self.cancelled;
        let result_tx = &self.result_tx;
        let file_list = &self.file_list;
        self.pool.install(|| {
            let _ = file_list.par_iter().try_for_each(|path| {
                if cancelled.load(Ordering::Relaxed)
                    || match_count.load(Ordering::Relaxed) >= MAX_MATCHES
                {
                    return Err(());
                }

                let Ok(content) = fs::read_to_string(path) else {
                    return Ok(());
                };

                let Ok(matches) =
                    find_matches_in_content(&content, &pattern, &match_count, MAX_MATCHES)
                else {
                    return Ok(());
                };

                if !matches.is_empty() {
                    let content_hash = FileHash::from_bytes(content.as_bytes());
                    let _ = result_tx.send(SearchResult::FileMatches {
                        generation: request.generation,
                        file_matches: FileMatches {
                            path: path.clone(),
                            responsive_path: None,
                            matches,
                            hash: content_hash,
                        },
                    });
                }

                Ok(())
            });
        });

        let total_matches = match_count.load(Ordering::Relaxed);
        let truncated = total_matches >= MAX_MATCHES;
        let was_cancelled = cancelled.load(Ordering::Relaxed);
        if was_cancelled {
            debug!(generation = request.generation, "search cancelled");
        } else if truncated {
            debug!(
                generation = request.generation,
                total_matches, "search hit match limit"
            );
        } else {
            debug!(
                generation = request.generation,
                total_matches, "search complete"
            );
        }
        let _ = self.result_tx.send(SearchResult::Complete {
            generation: request.generation,
            truncated,
        });
    }
}

pub fn find_matches_in_content(
    content: &str,
    pattern: &Pattern,
    match_count: &AtomicUsize,
    max_matches: usize,
) -> anyhow::Result<Vec<MatchInfo>> {
    if matches!(pattern, Pattern::Empty) || match_count.load(Ordering::Relaxed) >= max_matches {
        return Ok(Vec::new());
    }

    let raw_matches = find_byte_ranges(content, pattern);

    let mut matches = Vec::new();
    for raw in raw_matches {
        if match_count.load(Ordering::Relaxed) >= max_matches {
            break;
        }
        matches.push(MatchInfo::new(raw.start, raw.end, raw.captures));
        match_count.fetch_add(1, Ordering::Relaxed);
    }

    Ok(matches)
}

/// A match's byte range plus any captured groups (empty for non-regex patterns).
struct RawMatch {
    start: usize,
    end: usize,
    captures: Box<[Box<str>]>,
}

fn find_byte_ranges(content: &str, pattern: &Pattern) -> Vec<RawMatch> {
    match pattern {
        Pattern::Empty => Vec::new(),
        Pattern::Literal(pattern) => memchr::memmem::find_iter(content.as_bytes(), pattern)
            .map(|pos| RawMatch {
                start: pos,
                end: pos + pattern.len(),
                captures: Box::new([]),
            })
            .collect(),
        Pattern::Regex(re) if re.captures_len() > 1 => re
            .captures_iter(content)
            .filter_map(|caps| {
                let full = caps.get(0)?;
                let groups: Box<[Box<str>]> = (0..caps.len())
                    .map(|i| {
                        caps.get(i)
                            .map_or_else(|| Box::from(""), |m| Box::from(m.as_str()))
                    })
                    .collect();
                Some(RawMatch {
                    start: full.start(),
                    end: full.end(),
                    captures: groups,
                })
            })
            .collect(),
        Pattern::Regex(re) => re
            .find_iter(content)
            .map(|m| RawMatch {
                start: m.start(),
                end: m.end(),
                captures: Box::new([]),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, sync::mpsc, thread, time::Duration};
    use tempfile::TempDir;

    fn create_test_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }
        dir
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
    fn literal_search_finds_matches() {
        let content = "line one\nfoo bar\nline three\nfoo again\n";
        let matches = find_matches_in_content(
            content,
            &Pattern::new("foo", MatchMode::Literal).unwrap(),
            &AtomicUsize::new(0),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(
            &content[matches[0].byte_offset_start..matches[0].byte_offset_end],
            "foo"
        );
        assert_eq!(
            &content[matches[1].byte_offset_start..matches[1].byte_offset_end],
            "foo"
        );
    }

    #[test]
    fn regex_search_finds_matches() {
        let content = "hello world\nhello rust\ngoodbye\n";
        let matches = find_matches_in_content(
            content,
            &Pattern::new(r"hello \w+", MatchMode::Regex).unwrap(),
            &AtomicUsize::new(0),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(
            &content[matches[0].byte_offset_start..matches[0].byte_offset_end],
            "hello world"
        );
        assert_eq!(
            &content[matches[1].byte_offset_start..matches[1].byte_offset_end],
            "hello rust"
        );
    }

    #[test]
    fn regex_dot_does_not_match_carriage_return() {
        // on CRLF content, greedy `.` patterns must stop before `\r` so that
        // a subsequent replacement doesn't strip the `\r` and corrupt the line ending
        let content = "foo bar\r\nbaz\r\n";
        let matches = find_matches_in_content(
            content,
            &Pattern::new("foo.*", MatchMode::Regex).unwrap(),
            &AtomicUsize::new(0),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(
            &content[matches[0].byte_offset_start..matches[0].byte_offset_end],
            "foo bar"
        );
    }

    #[test]
    fn empty_pattern_returns_no_matches() {
        let content = "hello world\n";
        let matches = find_matches_in_content(
            content,
            &Pattern::new("", MatchMode::Literal).unwrap(),
            &AtomicUsize::new(0),
            usize::MAX,
        )
        .unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn invalid_regex_returns_error() {
        assert!(Pattern::new("[invalid", MatchMode::Regex).is_err());
    }

    #[test]
    fn byte_offsets_are_correct() {
        let content = "hello foo world\n";
        let matches = find_matches_in_content(
            content,
            &Pattern::new("foo", MatchMode::Literal).unwrap(),
            &AtomicUsize::new(0),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(matches[0].byte_offset_start, 6);
        assert_eq!(matches[0].byte_offset_end, 9);
    }

    #[test]
    fn search_worker_sends_results() {
        let dir = create_test_dir(&[("test.txt", "foo bar foo\n")]);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = SearchWorker::new(
            dir.path().to_path_buf(),
            cmd_rx,
            result_tx,
            cancelled,
            Options::default().into(),
        )
        .unwrap();
        let handle = thread::spawn(move || worker.run());

        cmd_tx
            .send(WorkerCommand::Search(SearchRequest {
                pattern: "foo".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        let mut got_file = false;
        loop {
            match result_rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                SearchResult::FileMatches {
                    generation,
                    file_matches: fm,
                } => {
                    assert_eq!(generation, 1);
                    assert_eq!(fm.matches.len(), 2);
                    got_file = true;
                }
                SearchResult::Complete { generation, .. } => {
                    assert_eq!(generation, 1);
                    break;
                }
                SearchResult::FileListReady { .. } => {}
                SearchResult::Error { .. } => panic!("unexpected error"),
            }
        }
        assert!(got_file);

        drop(cmd_tx);
        handle.join().unwrap();
    }

    #[test]
    fn search_worker_cancellation() {
        let dir = create_test_dir(&[
            ("a.txt", "needle\n"),
            ("b.txt", "needle\n"),
            ("c.txt", "needle\n"),
        ]);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = SearchWorker::new(
            dir.path().to_path_buf(),
            cmd_rx,
            result_tx,
            cancelled.clone(),
            Options::default().into(),
        )
        .unwrap();
        let handle = thread::spawn(move || worker.run());

        // send first request then immediately cancel and send second
        cmd_tx
            .send(WorkerCommand::Search(SearchRequest {
                pattern: "needle".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        cancelled.store(true, Ordering::Relaxed);

        cmd_tx
            .send(WorkerCommand::Search(SearchRequest {
                pattern: "needle".to_string(),
                mode: MatchMode::Literal,
                generation: 2,
            }))
            .unwrap();

        // drain results; we should eventually get Complete(2)
        let mut got_gen2_complete = false;
        loop {
            match result_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(SearchResult::Complete { generation: 2, .. }) => {
                    got_gen2_complete = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(got_gen2_complete);

        drop(cmd_tx);
        handle.join().unwrap();
    }
}
