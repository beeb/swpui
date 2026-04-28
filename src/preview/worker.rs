use std::{
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, mpsc},
};

use sha2::{Digest as _, Sha256};

use crate::{
    prelude::OrPanic as _,
    preview::{
        cache::PreviewCache,
        data::{PreviewData, build_preview_data},
    },
    types::{MatchInfo, MatchMode},
};

const NUM_WORKERS: usize = 3;
const READ_CHUNK_BYTES: usize = 64 * 1024;

pub type WantedSet = Arc<RwLock<[Option<PathBuf>; 3]>>;

#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub path: PathBuf,
    pub byte_ranges: Box<[(usize, usize)]>,
    pub content_hash: [u8; 32],
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
    Ready {
        path: PathBuf,
        generation: u64,
        data: Arc<PreviewData>,
    },
    Updated {
        path: PathBuf,
        generation: u64,
        matches: Vec<MatchInfo>,
        content_hash: [u8; 32],
        data: Arc<PreviewData>,
    },
    Removed {
        path: PathBuf,
        generation: u64,
    },
    Error {
        path: PathBuf,
        generation: u64,
        message: String,
    },
}

pub struct PreviewWorker {
    cmd_rx: Arc<Mutex<mpsc::Receiver<PreviewCommand>>>,
    result_tx: mpsc::Sender<PreviewResult>,
    cache: Arc<Mutex<PreviewCache>>,
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

    let maybe_data = {
        let mut cache = cache.lock().or_panic("poisoned lock");
        cache.get(&req.path, &req.content_hash)
    };
    if let Some(data) = maybe_data {
        let _ = result_tx.send(PreviewResult::Ready {
            path: req.path,
            generation: req.generation,
            data,
        });
        return;
    }

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

    if content_hash == req.content_hash {
        let data = Arc::new(build_preview_data(&content, &req.byte_ranges));
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
        let _ = result_tx.send(PreviewResult::Error {
            path: req.path,
            generation: req.generation,
            message: "file modified externally (re-search not yet implemented)".to_string(),
        });
    }
}

fn read_file_with_cancel(
    path: &Path,
    wanted: &WantedSet,
) -> io::Result<Option<(String, [u8; 32])>> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    let mut bytes: Vec<u8> = Vec::new();
    let mut hasher = Sha256::new();
    loop {
        if !path_is_wanted(wanted, path) {
            return Ok(None);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes.extend_from_slice(&buf[..n]);
    }
    let hash: [u8; 32] = hasher.finalize().into();
    let content =
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some((content, hash)))
}

fn path_is_wanted(wanted: &WantedSet, path: &Path) -> bool {
    let slots = wanted.read().or_panic("poisoned lock");
    slots.iter().any(|p| p.as_deref() == Some(path))
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, sync::mpsc, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::utils::hash_file;

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
        let mut f = fs::File::create(&path).unwrap_or_else(|_| unreachable!());
        f.write_all(content.as_bytes())
            .unwrap_or_else(|_| unreachable!());
        path
    }

    #[test]
    fn ready_when_hash_matches() {
        let dir = TempDir::new().unwrap_or_else(|_| unreachable!());
        let path = write_file(&dir, "a.txt", "hello world\n");
        let hash = hash_file(&path).unwrap_or_else(|_| unreachable!());

        let (cmd_tx, result_rx, wanted, _handle) = setup();
        if let Ok(mut slots) = wanted.write() {
            slots[0] = Some(path.clone());
        }

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![(0, 5)].into(),
                content_hash: hash,
                pattern: "hello".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap_or_else(|_| unreachable!());

        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| unreachable!());
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
        let dir = TempDir::new().unwrap_or_else(|_| unreachable!());
        let path = write_file(&dir, "a.txt", "hello\n");
        let hash = hash_file(&path).unwrap_or_else(|_| unreachable!());

        let (cmd_tx, result_rx, _wanted, _handle) = setup();

        cmd_tx
            .send(PreviewCommand::Request(PreviewRequest {
                path: path.clone(),
                byte_ranges: vec![(0, 5)].into(),
                content_hash: hash,
                pattern: "hello".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap_or_else(|_| unreachable!());

        let result = result_rx.recv_timeout(Duration::from_millis(200));
        assert!(result.is_err(), "expected no result, got {result:?}");
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
                byte_ranges: vec![(0, 5)].into(),
                content_hash: [0u8; 32],
                pattern: "x".to_string(),
                mode: MatchMode::Literal,
                generation: 1,
            }))
            .unwrap_or_else(|_| unreachable!());
        let result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| unreachable!());
        assert!(matches!(result, PreviewResult::Error { .. }));
    }
}
