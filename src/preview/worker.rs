use std::{
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, atomic::AtomicUsize, mpsc},
};

use crate::{
    hash::FileHash,
    prelude::OrPanic as _,
    preview::{cache::PreviewCache, data::PreviewData},
    search::{MAX_MATCHES, Pattern, find_matches_in_content},
    types::{ByteRange, MatchInfo, MatchMode},
};

/// Number of workers is based on the fact that the wanted set has 3 items at most
const NUM_WORKERS: usize = 3;

/// Read in chunks to be able to quickly stop if the file is not needed anymore
const READ_CHUNK_BYTES: u64 = 64 * 1024;

pub type WantedSet = Arc<RwLock<[Option<PathBuf>; NUM_WORKERS]>>;

#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub path: PathBuf,
    pub byte_ranges: Box<[ByteRange]>,
    pub hash: FileHash,
    pub pattern: String,
    pub mode: MatchMode,
    pub generation: u64,
}

#[derive(Debug)]
pub enum PreviewCommand {
    Request(PreviewRequest),
    Invalidate(PathBuf),
    Clear,
}

#[derive(Debug)]
pub enum PreviewResult {
    /// File didn't change, preview is ready
    Ready {
        path: PathBuf,
        generation: u64,
        data: Arc<PreviewData>,
    },

    /// File did change, communicate new matches and preview data
    Updated {
        path: PathBuf,
        generation: u64,
        matches: Vec<MatchInfo>,
        hash: FileHash,
        data: Arc<PreviewData>,
    },

    /// File should be removed (no more matches in the file)
    Removed { path: PathBuf, generation: u64 },

    /// Error when reading file
    Error {
        path: PathBuf,
        generation: u64,
        message: String,
    },
}

pub struct PreviewWorker {
    /// Receive channel shared by the workers
    cmd_rx: Arc<Mutex<mpsc::Receiver<PreviewCommand>>>,
    /// Channel cloned into each worker thread to send back results
    result_tx: mpsc::Sender<PreviewResult>,
    /// The preview LRU cache
    cache: Arc<Mutex<PreviewCache>>,
    /// The set is used to check if we should continue reading from the file or interrupt
    wanted: WantedSet,
}

impl PreviewWorker {
    #[must_use]
    pub fn new(
        cmd_rx: mpsc::Receiver<PreviewCommand>,
        result_tx: mpsc::Sender<PreviewResult>,
        wanted: WantedSet,
    ) -> Self {
        Self {
            cmd_rx: Arc::new(Mutex::new(cmd_rx)),
            result_tx,
            cache: Arc::new(Mutex::new(PreviewCache::new())),
            wanted,
        }
    }

    pub fn run(self) {
        let handles = (0..NUM_WORKERS)
            .map(|_| {
                let cmd_rx = Arc::clone(&self.cmd_rx);
                let result_tx = self.result_tx.clone();
                let cache = Arc::clone(&self.cache);
                let wanted = Arc::clone(&self.wanted);
                std::thread::spawn(move || {
                    worker_loop(&cmd_rx, &result_tx, &cache, &wanted);
                })
            })
            .collect::<Vec<_>>();
        for h in handles {
            let _ = h.join();
        }
    }
}

fn worker_loop(
    cmd_rx: &Mutex<mpsc::Receiver<PreviewCommand>>,
    result_tx: &mpsc::Sender<PreviewResult>,
    cache: &Mutex<PreviewCache>,
    wanted: &WantedSet,
) {
    loop {
        let cmd = {
            let rx = cmd_rx.lock().or_panic("poisoned lock");
            let Ok(c) = rx.recv() else { return };
            c
        };
        match cmd {
            PreviewCommand::Clear => {
                let mut cache = cache.lock().or_panic("poisoned lock");
                cache.clear();
            }
            PreviewCommand::Invalidate(path) => {
                let mut cache = cache.lock().or_panic("poisoned lock");
                cache.invalidate(&path);
            }
            PreviewCommand::Request(req) => {
                handle_request(req, result_tx, cache, wanted);
            }
        }
    }
}

fn handle_request(
    req: PreviewRequest,
    result_tx: &mpsc::Sender<PreviewResult>,
    cache: &Mutex<PreviewCache>,
    wanted: &WantedSet,
) {
    if !path_is_wanted(wanted, &req.path) {
        return;
    }

    // read the file first so we can use the actual on-disk hash as the cache key
    // using req.content_hash for the lookup would cause stale hits when the file
    // changes externally after the preview was cached
    let (content, content_hash) = match read_file_with_cancel(&req.path, wanted) {
        Ok(Some(pair)) => pair,
        Ok(None) => return, // this file is not in the wanted set anymore
        Err(e) => {
            let _ = result_tx.send(PreviewResult::Error {
                path: req.path,
                generation: req.generation,
                message: e.to_string(),
            });
            return;
        }
    };

    // return cached data if available
    let maybe_data = {
        let mut cache = cache.lock().or_panic("poisoned lock");
        cache.get(&req.path, &content_hash)
    };
    if let Some(data) = maybe_data {
        let _ = result_tx.send(PreviewResult::Ready {
            path: req.path,
            generation: req.generation,
            data,
        });
        return;
    }

    if content_hash == req.hash {
        // file hasn't changed since the search ran, construct preview data
        let data = Arc::new(PreviewData::new(&content, &req.byte_ranges));
        {
            let mut cache = cache.lock().or_panic("poisoned lock");
            cache.insert(req.path.clone(), content_hash, Arc::clone(&data));
        }
        let _ = result_tx.send(PreviewResult::Ready {
            path: req.path,
            generation: req.generation,
            data,
        });
    } else {
        // file has changed, let's refresh the search results for it before building the preview data
        let pattern = match Pattern::new(&req.pattern, req.mode) {
            Ok(p) => p,
            Err(e) => {
                let _ = result_tx.send(PreviewResult::Error {
                    path: req.path,
                    generation: req.generation,
                    message: e.to_string(),
                });
                return;
            }
        };
        let counter = AtomicUsize::new(0);
        let new_matches = match find_matches_in_content(&content, &pattern, &counter, MAX_MATCHES) {
            Ok(m) => m,
            Err(e) => {
                let _ = result_tx.send(PreviewResult::Error {
                    path: req.path,
                    generation: req.generation,
                    message: e.to_string(),
                });
                return;
            }
        };
        if new_matches.is_empty() {
            // file is not interesting anymore, no matches
            let _ = result_tx.send(PreviewResult::Removed {
                path: req.path,
                generation: req.generation,
            });
            return;
        }
        let byte_ranges: Vec<ByteRange> = new_matches.iter().map(|m| m.byte_range).collect();
        let data = Arc::new(PreviewData::new(&content, &byte_ranges));
        {
            let mut cache = cache.lock().or_panic("poisoned lock");
            cache.insert(req.path.clone(), content_hash, Arc::clone(&data));
        }
        let _ = result_tx.send(PreviewResult::Updated {
            path: req.path,
            generation: req.generation,
            matches: new_matches,
            hash: content_hash,
            data,
        });
    }
}

/// Read a file in chunks but stop if its path is not in the wanted set anymore.
fn read_file_with_cancel(
    path: &Path,
    wanted: &WantedSet,
) -> io::Result<Option<(String, FileHash)>> {
    let mut file = fs::File::open(path)?;
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        if !path_is_wanted(wanted, path) {
            return Ok(None);
        }
        let n = (&mut file).take(READ_CHUNK_BYTES).read_to_end(&mut bytes)?;
        if n == 0 {
            break;
        }
    }
    let hash = FileHash::from_bytes(&bytes);
    let content =
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some((content, hash)))
}

/// Check whether `path` is part of the wanted set.
fn path_is_wanted(wanted: &WantedSet, path: &Path) -> bool {
    let slots = wanted.read().or_panic("poisoned lock");
    slots.iter().any(|p| p.as_deref() == Some(path))
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, sync::mpsc, time::Duration};

    use tempfile::TempDir;

    use super::*;

    fn setup() -> (
        mpsc::Sender<PreviewCommand>,
        mpsc::Receiver<PreviewResult>,
        WantedSet,
        std::thread::JoinHandle<()>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let wanted: WantedSet = Arc::new(RwLock::new([None, None, None]));
        let worker = PreviewWorker::new(cmd_rx, result_tx, Arc::clone(&wanted));
        let handle = std::thread::spawn(move || worker.run());
        (cmd_tx, result_rx, wanted, handle)
    }

    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn ready_when_hash_matches() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "a.txt", "hello world\n");
        let hash = FileHash::new(&path).unwrap();

        let (cmd_tx, result_rx, wanted, _handle) = setup();
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 5)].into(),
                hash,
                pattern: "hello".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        let result = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let PreviewResult::Ready {
            path: p,
            generation,
            data,
        } = result
        else {
            panic!("expected Ready, got {result:?}");
        };
        assert_eq!(p, path);
        assert_eq!(generation, 1);
        assert_eq!(data.matches.len(), 1);
    }

    #[test]
    fn drops_request_when_path_not_wanted() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "a.txt", "hello\n");
        let hash = FileHash::new(&path).unwrap();

        let (cmd_tx, result_rx, _wanted, _handle) = setup();

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 5)].into(),
                hash,
                pattern: "hello".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        let result = result_rx.recv_timeout(Duration::from_millis(200));
        assert!(result.is_err(), "expected no result, got {result:?}");
    }

    #[test]
    fn updated_when_hash_mismatches_and_matches_found() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "a.txt", "foo bar foo\n");
        let stale_hash = [0xABu8; 32].into();

        let (cmd_tx, result_rx, wanted, _handle) = setup();
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 3)].into(),
                hash: stale_hash,
                pattern: "foo".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        let result = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let PreviewResult::Updated {
            matches,
            hash: content_hash,
            data,
            ..
        } = result
        else {
            panic!("expected Updated, got {result:?}");
        };
        assert_eq!(matches.len(), 2);
        assert_ne!(content_hash, stale_hash);
        assert_eq!(data.matches.len(), 2);
    }

    #[test]
    fn removed_when_research_yields_zero_matches() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "a.txt", "no matches here\n");
        let stale_hash = [0xABu8; 32].into();

        let (cmd_tx, result_rx, wanted, _handle) = setup();
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![].into(),
                hash: stale_hash,
                pattern: "needle".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();

        let result = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(result, PreviewResult::Removed { .. }));
    }

    #[test]
    fn error_when_file_missing() {
        let (cmd_tx, result_rx, wanted, _handle) = setup();
        let path = PathBuf::from("/nonexistent/path/zzz.txt");
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }
        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 5)].into(),
                hash: FileHash::default(),
                pattern: "x".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();
        let result = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(result, PreviewResult::Error { .. }));
    }

    #[test]
    fn updated_when_file_changed_after_cache_populated() {
        // reproduce the bug: file previewed (cache populated with H1), then modified
        // externally (now H2), then previewed again with req.content_hash=H1
        // must return Updated, not the stale cached Ready
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "a.txt", "foo\n");
        let original_hash = FileHash::new(&path).unwrap();

        let (cmd_tx, result_rx, wanted, _handle) = setup();
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }

        // first request: populates cache under original_hash
        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 3)].into(),
                hash: original_hash,
                pattern: "foo".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap();
        let r1 = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            matches!(r1, PreviewResult::Ready { .. }),
            "expected Ready, got {r1:?}"
        );

        // modify the file externally
        write_file(&dir, "a.txt", "foo bar\n");

        // second request: still uses original_hash (FileMatches not updated yet)
        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![ByteRange::new(0, 3)].into(),
                hash: original_hash,
                pattern: "foo".to_string(),
                mode: MatchMode::Literal,
                generation: 2,
            }))
            .unwrap();
        let r2 = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            matches!(r2, PreviewResult::Updated { .. }),
            "expected Updated (file changed externally), got {r2:?}"
        );
    }
}
