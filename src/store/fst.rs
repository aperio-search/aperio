use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use fst::automaton::{Levenshtein, Str};
use fst::{Automaton, IntoStreamer, Set, SetBuilder, Streamer};

use super::config::FSTConfig;

pub struct CollectionFST {
    set: Option<Set<Vec<u8>>>,
    path: PathBuf,
    pending_push: HashSet<Vec<u8>>,
    pending_pop: HashSet<Vec<u8>>,
    dirty: AtomicBool,
    last_consolidated: Instant,
}

impl CollectionFST {
    fn open_or_create(path: &Path) -> Self {
        let set = path
            .exists()
            .then(|| {
                std::fs::read(path)
                    .ok()
                    .and_then(|data| Set::new(data).ok())
            })
            .flatten();
        Self {
            set,
            path: path.to_path_buf(),
            pending_push: HashSet::new(),
            pending_pop: HashSet::new(),
            dirty: AtomicBool::new(false),
            last_consolidated: Instant::now(),
        }
    }

    fn contains(&self, word: &[u8]) -> bool {
        self.set.as_ref().is_some_and(|s| s.contains(word))
    }

    fn push_word(&mut self, word: &[u8]) {
        if self.contains(word) {
            self.pending_pop.remove(word);
            return;
        }
        if !self.pending_push.contains(word) {
            self.pending_push.insert(word.to_vec());
            self.dirty.store(true, Ordering::Release);
        }
    }

    fn pop_word(&mut self, word: &[u8]) {
        if self.pending_push.remove(word) {
            self.dirty.store(true, Ordering::Release);
            return;
        }
        if self.contains(word) && !self.pending_pop.contains(word) {
            self.pending_pop.insert(word.to_vec());
            self.dirty.store(true, Ordering::Release);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    fn needs_consolidate(&self, config: &FSTConfig) -> bool {
        if !self.is_dirty() {
            return false;
        }
        self.last_consolidated.elapsed().as_secs() >= config.consolidate_after_secs
    }

    fn consolidate(&mut self, _config: &FSTConfig) -> Result<(), String> {
        let tmp_path = self.path.with_extension("fst.tmp");

        let writer = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("failed to create tmp fst: {e}"))?;
        let mut builder =
            SetBuilder::new(writer).map_err(|e| format!("failed to create FST builder: {e}"))?;

        let mut push_words: Vec<Vec<u8>> = self.pending_push.iter().cloned().collect();
        push_words.sort();

        let mut push_idx = 0;

        if let Some(set) = &self.set {
            let mut stream = set.stream();
            while let Some(word) = stream.next() {
                while push_idx < push_words.len() && push_words[push_idx] < word.to_vec() {
                    builder
                        .insert(push_words[push_idx].as_slice())
                        .map_err(|e| format!("fst insert error: {e}"))?;
                    push_idx += 1;
                }
                if !self.pending_pop.contains(word) {
                    builder
                        .insert(word)
                        .map_err(|e| format!("fst insert error: {e}"))?;
                }
            }
        }

        while push_idx < push_words.len() {
            builder
                .insert(push_words[push_idx].as_slice())
                .map_err(|e| format!("fst insert error: {e}"))?;
            push_idx += 1;
        }

        builder
            .finish()
            .map_err(|e| format!("fst finish error: {e}"))?;

        // Rename first, *then* re-read. If re-read fails we must leave
        // `pending_push`/`pending_pop` intact so the next consolidate retries
        // — clearing them here would silently drop vocabulary updates.
        std::fs::rename(&tmp_path, &self.path)
            .map_err(|e| format!("failed to rename fst file: {e}"))?;

        let new_data = std::fs::read(&self.path)
            .map_err(|e| format!("failed to read consolidated fst file: {e}"))?;
        let new_set =
            Set::new(new_data).map_err(|e| format!("consolidated fst file is invalid: {e}"))?;

        self.set = Some(new_set);
        self.pending_push.clear();
        self.pending_pop.clear();
        self.dirty.store(false, Ordering::Release);
        self.last_consolidated = Instant::now();

        Ok(())
    }

    fn word_count(&self) -> usize {
        self.set.as_ref().map_or(0, |s| s.len())
    }

    fn search_prefix(&self, prefix: &str, limit: usize) -> Vec<String> {
        let set = match &self.set {
            Some(s) => s,
            None => return Vec::new(),
        };

        let aut = Str::new(prefix).starts_with();
        let mut stream = set.search(&aut).into_stream();
        let mut results = Vec::new();
        while let Some(word) = stream.next() {
            if results.len() >= limit {
                break;
            }
            if let Ok(s) = std::str::from_utf8(word) {
                results.push(s.to_string());
            }
        }
        results
    }

    fn search_fuzzy(&self, word: &str, limit: usize, max_distance: Option<u32>) -> Vec<String> {
        let set = match &self.set {
            Some(s) => s,
            None => return Vec::new(),
        };

        let dist = max_distance.unwrap_or(match word.len() {
            1..=3 => 0,
            4..=6 => 1,
            7..=9 => 2,
            _ => 3,
        });

        let lev = match Levenshtein::new(word, dist) {
            Ok(l) => l,
            Err(_) => return Vec::new(),
        };

        let mut stream = set.search(&lev).into_stream();
        let mut results = Vec::new();
        while let Some(w) = stream.next() {
            if results.len() >= limit {
                break;
            }
            if let Ok(s) = std::str::from_utf8(w) {
                results.push(s.to_string());
            }
        }
        results
    }

    fn list_words(&self, limit: usize, offset: usize) -> Vec<String> {
        let set = match &self.set {
            Some(s) => s,
            None => return Vec::new(),
        };

        let mut stream = set.stream();
        let mut results = Vec::new();
        let mut idx = 0usize;
        while let Some(word) = stream.next() {
            if idx < offset {
                idx += 1;
                continue;
            }
            if results.len() >= limit {
                break;
            }
            if let Ok(s) = std::str::from_utf8(word) {
                results.push(s.to_string());
            }
            idx += 1;
        }
        results
    }
}

pub struct FSTPool {
    stores: Mutex<HashMap<String, Arc<Mutex<CollectionFST>>>>,
    base_path: PathBuf,
    config: FSTConfig,
}

impl FSTPool {
    pub fn new(base_path: PathBuf, config: FSTConfig) -> Self {
        if config.enabled {
            std::fs::create_dir_all(&base_path).ok();
        }
        Self {
            stores: Mutex::new(HashMap::new()),
            base_path,
            config,
        }
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn collection_path(&self, collection: &str) -> PathBuf {
        let safe: String = collection
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.base_path.join(format!("{}.fst", safe))
    }

    fn get_or_create(&self, collection: &str) -> Arc<Mutex<CollectionFST>> {
        let mut stores = self.stores.lock().unwrap();
        if let Some(existing) = stores.get(collection) {
            return Arc::clone(existing);
        }
        let path = self.collection_path(collection);
        let fst = CollectionFST::open_or_create(&path);
        let fst = Arc::new(Mutex::new(fst));
        stores.insert(collection.to_string(), Arc::clone(&fst));
        fst
    }

    pub fn push_words(&self, collection: &str, words: &HashSet<String>) {
        if !self.enabled() || words.is_empty() {
            return;
        }
        let fst = self.get_or_create(collection);
        let mut guard = fst.lock().unwrap();
        for word in words {
            guard.push_word(word.as_bytes());
        }
    }

    pub fn pop_words(&self, collection: &str, words: &HashSet<String>) {
        if !self.enabled() || words.is_empty() {
            return;
        }
        let fst = self.get_or_create(collection);
        let mut guard = fst.lock().unwrap();
        for word in words {
            guard.pop_word(word.as_bytes());
        }
    }

    pub fn contains(&self, collection: &str, word: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        let fst = self.get_or_create(collection);
        let guard = fst.lock().unwrap();
        guard.contains(word.as_bytes())
    }

    pub fn suggest_prefix(&self, collection: &str, prefix: &str, limit: usize) -> Vec<String> {
        if !self.enabled() || prefix.is_empty() {
            return Vec::new();
        }
        let fst = self.get_or_create(collection);
        let guard = fst.lock().unwrap();
        guard.search_prefix(prefix, limit)
    }

    pub fn suggest_fuzzy(
        &self,
        collection: &str,
        word: &str,
        limit: usize,
        max_distance: Option<u32>,
    ) -> Vec<String> {
        if !self.enabled() || word.is_empty() {
            return Vec::new();
        }
        let fst = self.get_or_create(collection);
        let guard = fst.lock().unwrap();
        guard.search_fuzzy(word, limit, max_distance)
    }

    pub fn list_words(&self, collection: &str, limit: usize, offset: usize) -> Vec<String> {
        if !self.enabled() {
            return Vec::new();
        }
        let fst = self.get_or_create(collection);
        let guard = fst.lock().unwrap();
        guard.list_words(limit, offset)
    }

    pub fn word_count(&self, collection: &str) -> usize {
        if !self.enabled() {
            return 0;
        }
        let fst = self.get_or_create(collection);
        let guard = fst.lock().unwrap();
        guard.word_count()
    }

    pub fn consolidate(&self, collection: &str) -> Result<(), String> {
        if !self.enabled() {
            return Ok(());
        }
        let fst = self.get_or_create(collection);
        let mut guard = fst.lock().unwrap();
        guard.consolidate(&self.config)
    }

    pub fn consolidate_dirty(&self) {
        if !self.enabled() {
            return;
        }
        let collections: Vec<String> = {
            let stores = self.stores.lock().unwrap();
            stores
                .iter()
                .filter(|(_, fst)| fst.lock().unwrap().needs_consolidate(&self.config))
                .map(|(name, _)| name.clone())
                .collect()
        };

        for collection in &collections {
            if let Err(e) = self.consolidate(collection) {
                tracing::error!(collection = %collection, error = %e, "FST consolidation failed");
            } else {
                tracing::debug!(collection = %collection, "FST consolidation completed");
            }
        }
    }

    pub fn delete_collection(&self, collection: &str) {
        if !self.enabled() {
            return;
        }
        {
            let mut stores = self.stores.lock().unwrap();
            stores.remove(collection);
        }
        let path = self.collection_path(collection);
        std::fs::remove_file(&path).ok();
    }

    pub fn clear_all(&self) {
        if !self.enabled() {
            return;
        }
        {
            let mut stores = self.stores.lock().unwrap();
            stores.clear();
        }
        std::fs::remove_dir_all(&self.base_path).ok();
        std::fs::create_dir_all(&self.base_path).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_pool() -> (FSTPool, TempDir) {
        let dir = TempDir::new().unwrap();
        let pool = FSTPool::new(dir.path().join("fst"), FSTConfig::default());
        (pool, dir)
    }

    #[test]
    fn empty_pool_contains_nothing() {
        let (pool, _dir) = test_pool();
        assert!(!pool.contains("c", "hello"));
    }

    #[test]
    fn push_and_contains_after_consolidate() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        words.insert("world".to_string());
        pool.push_words("c", &words);
        assert!(!pool.contains("c", "hello"));
        pool.consolidate("c").unwrap();
        assert!(pool.contains("c", "hello"));
        assert!(pool.contains("c", "world"));
    }

    #[test]
    fn push_and_pop() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(pool.contains("c", "hello"));

        let mut removed = HashSet::new();
        removed.insert("hello".to_string());
        pool.pop_words("c", &removed);
        pool.consolidate("c").unwrap();
        assert!(!pool.contains("c", "hello"));
    }

    #[test]
    fn prefix_search() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        for w in &["apple", "application", "appetite", "banana", "app"] {
            words.insert(w.to_string());
        }
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();

        let results = pool.suggest_prefix("c", "app", 10);
        assert_eq!(results.len(), 4);
        assert!(results.contains(&"app".to_string()));
        assert!(results.contains(&"apple".to_string()));
        assert!(results.contains(&"application".to_string()));
        assert!(results.contains(&"appetite".to_string()));
    }

    #[test]
    fn prefix_search_limit() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        for w in &["a1", "a2", "a3", "a4", "a5"] {
            words.insert(w.to_string());
        }
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();

        let results = pool.suggest_prefix("c", "a", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn fuzzy_search() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        for w in &["apple", "appetite", "banana", "apply"] {
            words.insert(w.to_string());
        }
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();

        let results = pool.suggest_fuzzy("c", "aple", 10, Some(2));
        assert!(results.contains(&"apple".to_string()));
        assert!(results.contains(&"apply".to_string()));
    }

    #[test]
    fn list_words() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        for w in &["a", "b", "c", "d", "e"] {
            words.insert(w.to_string());
        }
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();

        let all = pool.list_words("c", 10, 0);
        assert_eq!(all.len(), 5);

        let page = pool.list_words("c", 2, 1);
        assert_eq!(page, vec!["b", "c"]);
    }

    #[test]
    fn word_count() {
        let (pool, _dir) = test_pool();
        assert_eq!(pool.word_count("c"), 0);

        let mut words = HashSet::new();
        words.insert("hello".to_string());
        words.insert("world".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert_eq!(pool.word_count("c"), 2);
    }

    #[test]
    fn collection_isolation() {
        let (pool, _dir) = test_pool();
        let mut a = HashSet::new();
        a.insert("hello".to_string());
        pool.push_words("a", &a);
        pool.consolidate("a").unwrap();

        let mut b = HashSet::new();
        b.insert("world".to_string());
        pool.push_words("b", &b);
        pool.consolidate("b").unwrap();

        assert!(pool.contains("a", "hello"));
        assert!(!pool.contains("a", "world"));
        assert!(pool.contains("b", "world"));
        assert!(!pool.contains("b", "hello"));
    }

    #[test]
    fn delete_collection() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(pool.contains("c", "hello"));

        pool.delete_collection("c");
        assert!(!pool.contains("c", "hello"));
    }

    #[test]
    fn persistent_across_pool_reopen() {
        let dir = TempDir::new().unwrap();
        let fst_dir = dir.path().join("fst");
        {
            let pool = FSTPool::new(fst_dir.clone(), FSTConfig::default());
            let mut words = HashSet::new();
            words.insert("hello".to_string());
            pool.push_words("c", &words);
            pool.consolidate("c").unwrap();
            assert!(pool.contains("c", "hello"));
        }
        {
            let pool = FSTPool::new(fst_dir, FSTConfig::default());
            assert!(pool.contains("c", "hello"));
            let results = pool.suggest_prefix("c", "hel", 10);
            assert_eq!(results, vec!["hello"]);
        }
    }

    #[test]
    fn empty_prefix_search_returns_empty() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(pool.suggest_prefix("c", "", 10).is_empty());
    }

    #[test]
    fn push_then_push_again_no_duplicate() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert_eq!(pool.word_count("c"), 1);

        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert_eq!(pool.word_count("c"), 1);
    }

    #[test]
    fn pop_nonexistent_word_is_noop() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.pop_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(!pool.contains("c", "hello"));
    }

    #[test]
    fn push_after_pop_removes_from_pop() {
        let (pool, _dir) = test_pool();
        let mut words = HashSet::new();
        words.insert("hello".to_string());
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(pool.contains("c", "hello"));

        pool.pop_words("c", &words);
        pool.push_words("c", &words);
        pool.consolidate("c").unwrap();
        assert!(pool.contains("c", "hello"));
    }
}
