use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, mpsc},
};

use crate::{
    preview::{cache::PreviewCache, data::PreviewData},
    types::{MatchInfo, MatchMode},
};

const NUM_WORKERS: usize = 3;

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
            let Ok(rx) = cmd_rx.lock() else { return };
            let Ok(c) = rx.recv() else { return };
            c
        };
        match cmd {
            PreviewCommand::Clear => {
                let Ok(mut cache) = cache.lock() else {
                    continue;
                };
                cache.clear();
            }
            PreviewCommand::Invalidate(path) => {
                let Ok(mut cache) = cache.lock() else {
                    continue;
                };
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
}

fn path_is_wanted(wanted: &WantedSet, path: &Path) -> bool {
    let Ok(slots) = wanted.read() else {
        return false;
    };
    slots.iter().any(|p| p.as_deref() == Some(path))
}
