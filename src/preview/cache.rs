use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::preview::data::PreviewData;

pub const MIN_ENTRIES: usize = 1;
pub const MAX_ENTRIES: usize = 16;
pub const BYTE_CAP: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct PreviewCache {
    pub entries: VecDeque<Entry>,
    pub total_bytes: usize,
}

pub struct Entry {
    pub path: PathBuf,
    pub content_hash: [u8; 32],
    pub data: Arc<PreviewData>,
}

impl PreviewCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(
        &mut self,
        path: impl AsRef<Path>,
        content_hash: &[u8; 32],
    ) -> Option<Arc<PreviewData>> {
        let path = path.as_ref();
        let pos = self
            .entries
            .iter()
            .position(|e| e.path == path && &e.content_hash == content_hash)?;
        let entry = self.entries.remove(pos)?;
        let data = Arc::clone(&entry.data);
        self.entries.push_front(entry);
        Some(data)
    }

    pub fn insert(&mut self, path: PathBuf, content_hash: [u8; 32], data: Arc<PreviewData>) {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.path == path && e.content_hash == content_hash)
        {
            let old = self.entries.remove(pos).unwrap_or_else(|| unreachable!());
            self.total_bytes = self.total_bytes.saturating_sub(old.data.size_bytes);
        }
        self.total_bytes += data.size_bytes;
        self.entries.push_front(Entry {
            path,
            content_hash,
            data,
        });
        self.evict();
    }

    pub fn invalidate(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let mut total_bytes_removed = 0;
        self.entries.retain(|e| {
            if e.path == path {
                total_bytes_removed += e.data.size_bytes;
                false
            } else {
                true
            }
        });
        self.total_bytes = self.total_bytes.saturating_sub(total_bytes_removed);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    fn drop_back(&mut self) {
        if let Some(e) = self.entries.pop_back() {
            self.total_bytes = self.total_bytes.saturating_sub(e.data.size_bytes);
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > MAX_ENTRIES {
            self.drop_back();
        }
        while self.total_bytes > BYTE_CAP && self.entries.len() > MIN_ENTRIES {
            self.drop_back();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(size: usize) -> Arc<PreviewData> {
        Arc::new(PreviewData {
            matches: Box::new([]),
            size_bytes: size,
        })
    }

    #[test]
    fn insert_and_lookup() {
        let mut cache = PreviewCache::new();
        let path = PathBuf::from("a.txt");
        let hash = [0u8; 32];
        cache.insert(path.clone(), hash, make_data(10));
        assert!(cache.get(&path, &hash).is_some());
    }

    #[test]
    fn lookup_miss_on_different_hash() {
        let mut cache = PreviewCache::new();
        let path = PathBuf::from("a.txt");
        cache.insert(path.clone(), [0u8; 32], make_data(10));
        assert!(cache.get(&path, &[1u8; 32]).is_none());
    }

    #[test]
    fn lru_touch_on_hit_moves_to_front() {
        let mut cache = PreviewCache::new();
        cache.insert(PathBuf::from("a"), [0u8; 32], make_data(10));
        cache.insert(PathBuf::from("b"), [0u8; 32], make_data(10));
        cache.get(PathBuf::from("a"), &[0u8; 32]);
        let order: Vec<_> = cache.entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(order, vec![PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn evicts_oldest_when_max_entries_exceeded() {
        let mut cache = PreviewCache::new();
        for i in 0..(MAX_ENTRIES + 3) {
            cache.insert(PathBuf::from(format!("{i}")), [0u8; 32], make_data(10));
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get(PathBuf::from("0"), &[0u8; 32]).is_none());
    }

    #[test]
    fn evicts_when_byte_cap_exceeded() {
        let mut cache = PreviewCache::new();
        cache.insert(PathBuf::from("a"), [0u8; 32], make_data(BYTE_CAP / 2));
        cache.insert(PathBuf::from("b"), [0u8; 32], make_data(BYTE_CAP / 2));
        cache.insert(PathBuf::from("c"), [0u8; 32], make_data(BYTE_CAP / 2));
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert!(cache.total_bytes <= BYTE_CAP);
    }

    #[test]
    fn keeps_single_oversized_entry() {
        let mut cache = PreviewCache::new();
        cache.insert(PathBuf::from("big"), [0u8; 32], make_data(BYTE_CAP * 2));
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.get(PathBuf::from("big"), &[0u8; 32]).is_some());
    }

    #[test]
    fn invalidate_removes_entries_for_path() {
        let mut cache = PreviewCache::new();
        cache.insert(PathBuf::from("a"), [0u8; 32], make_data(10));
        cache.insert(PathBuf::from("a"), [1u8; 32], make_data(10));
        cache.insert(PathBuf::from("b"), [0u8; 32], make_data(10));
        cache.invalidate(PathBuf::from("a"));
        assert!(cache.get(PathBuf::from("a"), &[0u8; 32]).is_none());
        assert!(cache.get(PathBuf::from("a"), &[1u8; 32]).is_none());
        assert!(cache.get(PathBuf::from("b"), &[0u8; 32]).is_some());
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut cache = PreviewCache::new();
        cache.insert(PathBuf::from("a"), [0u8; 32], make_data(10));
        cache.insert(PathBuf::from("b"), [0u8; 32], make_data(10));
        cache.clear();
        assert_eq!(cache.entries.len(), 0);
        assert_eq!(cache.total_bytes, 0);
    }
}
