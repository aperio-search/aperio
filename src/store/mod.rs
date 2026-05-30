use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use charabia::Tokenize;
use roaring::RoaringTreemap;

use crate::error::AppError;
use crate::models::{
    CollectionCreated, CollectionInfo, CollectionSummary, ListCollectionsResponse,
};

pub use config::{IdType, StoreConfig};

macro_rules! encode_rkyv {
    ($value:expr) => {{
        let result: Result<Vec<u8>, AppError> = rkyv::to_bytes::<rkyv::rancor::Error>($value)
            .map(|av| av.to_vec())
            .map_err(|e| AppError::Internal(e.to_string()));
        result
    }};
}

macro_rules! decode_rkyv {
    ($ty:ty, $bytes:expr) => {{
        let result: Result<$ty, AppError> = rkyv::from_bytes::<$ty, rkyv::rancor::Error>($bytes)
            .map_err(|e| AppError::Internal(e.to_string()));
        result
    }};
}

pub(crate) const SHARD_DELIM: char = '\0';

fn roaring_to_vec(b: &RoaringTreemap) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::with_capacity(b.serialized_size());
    b.serialize_into(&mut buf)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(buf)
}

fn roaring_from_slice(bytes: &[u8]) -> Result<RoaringTreemap, AppError> {
    RoaringTreemap::deserialize_from(bytes).map_err(|e| AppError::Internal(e.to_string()))
}

fn tokenize(content: &str, min_token_length: usize) -> HashSet<String> {
    content
        .tokenize()
        .filter(|t| t.is_word())
        .map(|t| t.lemma().to_string())
        .filter(|w| w.len() >= min_token_length)
        .collect()
}

mod config;
mod posting_list;
mod search;

pub struct Store {
    db: fjall::Database,
    config: StoreConfig,
    lock: Mutex<()>,
    collections: RwLock<HashMap<String, IdType>>,
    next_seq: AtomicU64,
    background_active: AtomicBool,
}

impl Store {
    pub fn new(db: fjall::Database) -> Self {
        Self::with_config(db, StoreConfig::default())
    }

    pub fn with_config(db: fjall::Database, config: StoreConfig) -> Self {
        let collections = {
            let mut map = HashMap::new();
            if let Ok(meta) = db.keyspace("_collections", || config.keyspace_opts()) {
                for guard in meta.iter() {
                    if let Ok((key, value)) = guard.into_inner() {
                        let name = String::from_utf8_lossy(&key).to_string();
                        if let Ok(id_type) = decode_rkyv!(config::IdType, &value) {
                            map.insert(name, id_type);
                        }
                    }
                }
            }
            map
        };
        let next_seq = Self::init_next_seq(&db, &config);

        Self {
            db,
            config,
            lock: Mutex::new(()),
            collections: RwLock::new(collections),
            next_seq: AtomicU64::new(next_seq),
            background_active: AtomicBool::new(false),
        }
    }

    fn init_next_seq(db: &fjall::Database, config: &StoreConfig) -> u64 {
        let queue = match db.keyspace("_index_queue", || config.keyspace_opts()) {
            Ok(q) => q,
            Err(_) => return 1,
        };
        let mut max = 0u64;
        for guard in queue.iter() {
            if let Ok((key, _)) = guard.into_inner()
                && key.len() == 8
            {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&key);
                let seq = u64::from_be_bytes(buf);
                if seq > max {
                    max = seq;
                }
            }
        }
        max + 1
    }

    fn inverted_keyspace(&self, collection: &str) -> Result<fjall::Keyspace, AppError> {
        let name = format!("{}.inverted", collection);
        Ok(self.db.keyspace(&name, || self.config.keyspace_opts())?)
    }

    fn docs_keyspace(&self, collection: &str) -> Result<fjall::Keyspace, AppError> {
        let name = format!("{}.docs", collection);
        Ok(self.db.keyspace(&name, || self.config.keyspace_opts())?)
    }

    fn meta_keyspace(&self) -> Result<fjall::Keyspace, AppError> {
        Ok(self
            .db
            .keyspace("_collections", || self.config.keyspace_opts())?)
    }

    fn queue_keyspace(&self) -> Result<fjall::Keyspace, AppError> {
        Ok(self
            .db
            .keyspace("_index_queue", || self.config.keyspace_opts())?)
    }

    fn allocate_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    fn validate_collection_exists(&self, collection: &str) -> Result<IdType, AppError> {
        self.collections
            .read()
            .unwrap()
            .get(collection)
            .copied()
            .ok_or_else(|| AppError::NotFound(format!("collection '{}' not found", collection)))
    }

    pub fn create_collection(
        &self,
        name: &str,
        id_type: &str,
    ) -> Result<CollectionCreated, AppError> {
        let id_type_enum = match id_type {
            "number" => IdType::Number,
            "string" => IdType::String,
            _ => {
                return Err(AppError::BadRequest(format!(
                    "invalid id_type '{}', expected 'number' or 'string'",
                    id_type
                )));
            }
        };

        let _lock = self.lock.lock().unwrap();

        {
            let mut map = self.collections.write().unwrap();
            if map.contains_key(name) {
                return Err(AppError::BadRequest(format!(
                    "collection '{}' already exists",
                    name
                )));
            }

            let meta = self.meta_keyspace()?;
            let value = encode_rkyv!(&id_type_enum)?;
            meta.insert(name.as_bytes(), &value)?;

            map.insert(name.to_string(), id_type_enum);
        }

        tracing::info!(collection = %name, id_type = %id_type, "collection created");

        Ok(CollectionCreated {
            name: name.to_string(),
            id_type: id_type.to_string(),
        })
    }

    fn upsert_internal(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        let id_u64 = match id_type {
            IdType::Number => Some(id.parse::<u64>().map_err(|_| {
                AppError::BadRequest(format!(
                    "invalid id '{}': collection '{}' expects numeric ids",
                    id, collection
                ))
            })?),
            IdType::String => None,
        };

        let _lock = self.lock.lock().unwrap();

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let new_words = tokenize(content, self.config.min_token_length);

        if let Some(old_data) = docs.get(id.as_bytes())? {
            let old_tokens: Vec<String> = decode_rkyv!(Vec<String>, &old_data)?;
            let old_words: HashSet<String> = old_tokens.into_iter().collect();
            for word in old_words.difference(&new_words) {
                match id_type {
                    IdType::Number => posting_list::remove_from_roaring_posting_list(
                        &inverted,
                        word,
                        id_u64.unwrap(),
                    )?,
                    IdType::String => posting_list::remove_from_posting_list(&inverted, word, id)?,
                }
            }
        }

        for word in &new_words {
            match id_type {
                IdType::Number => posting_list::add_to_roaring_posting_list(
                    &inverted,
                    word,
                    id_u64.unwrap(),
                    self.config.max_roaring_shard_size,
                )?,
                IdType::String => posting_list::add_to_posting_list(
                    &inverted,
                    word,
                    id,
                    self.config.max_shard_size,
                )?,
            }
        }

        let tokens: Vec<String> = new_words.into_iter().collect();
        docs.insert(id.as_bytes(), encode_rkyv!(&tokens)?)?;
        tracing::debug!(collection = %collection, id = %id, tokens = tokens.len(), "item upserted");
        Ok(())
    }

    pub fn upsert(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        match id_type {
            IdType::Number => {
                id.parse::<u64>().map_err(|_| {
                    AppError::BadRequest(format!(
                        "invalid id '{}': collection '{}' expects numeric ids",
                        id, collection
                    ))
                })?;
            }
            IdType::String => {}
        }

        if !self.background_active.load(Ordering::Acquire) {
            return self.upsert_internal(collection, id, content);
        }

        let seq = self.allocate_seq();
        let queue = self.queue_keyspace()?;
        let entry = config::QueuedIndex {
            collection: collection.to_string(),
            id: id.to_string(),
            content: content.to_string(),
        };
        queue.insert(seq.to_be_bytes(), &encode_rkyv!(&entry)?)?;
        tracing::debug!(collection = %collection, id = %id, seq = %seq, "item queued for indexing");
        Ok(())
    }

    pub fn process_pending_queue(&self) -> Result<(), AppError> {
        let queue = self.queue_keyspace()?;
        let mut batch: Vec<(Vec<u8>, config::QueuedIndex)> = Vec::new();

        for guard in queue.iter() {
            let (key, value) = guard.into_inner()?;
            if key.len() == 8
                && let Ok(entry) = decode_rkyv!(config::QueuedIndex, &value)
            {
                batch.push((key.to_vec(), entry));
            }
        }

        for (key, entry) in &batch {
            if let Err(e) = self.upsert_internal(&entry.collection, &entry.id, &entry.content) {
                tracing::error!(
                    error = ?e,
                    collection = %entry.collection,
                    id = %entry.id,
                    "failed to index queued item"
                );
            }
            queue.remove(key)?;
        }

        Ok(())
    }

    pub fn flush(&self) -> Result<(), AppError> {
        self.process_pending_queue()
    }

    pub fn spawn_background(self: &std::sync::Arc<Self>) {
        self.background_active.store(true, Ordering::Release);
        let store = std::sync::Arc::clone(self);
        let interval = self.config.index_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if let Err(e) = store.process_pending_queue() {
                    tracing::error!(error = ?e, "background indexing cycle failed");
                }
            }
        });
    }

    pub fn search(
        &self,
        collection: &str,
        query: &str,
        sort_desc: bool,
        take: usize,
        after: Option<&str>,
    ) -> Result<Vec<String>, AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        let inverted = self.inverted_keyspace(collection)?;
        let results = match id_type {
            IdType::Number => search::roaring_search(
                &inverted,
                self.config.min_token_length,
                query,
                sort_desc,
                take,
                after,
            )?,
            IdType::String => search::string_search(
                &inverted,
                self.config.min_token_length,
                query,
                sort_desc,
                take,
                after,
            )?,
        };
        tracing::debug!(collection = %collection, query = %query, results = results.len(), "search completed");
        Ok(results)
    }

    pub fn suggest(&self, collection: &str, prefix: &str) -> Result<Vec<String>, AppError> {
        self.validate_collection_exists(collection)?;
        let inverted = self.inverted_keyspace(collection)?;
        let last_word = prefix.split_whitespace().last().unwrap_or(prefix);
        let normalized = last_word
            .tokenize()
            .find(|t| t.is_word())
            .map(|t| t.lemma().to_string())
            .unwrap_or_else(|| last_word.to_lowercase());
        let mut seen = HashSet::new();
        let results: Vec<String> = inverted
            .prefix(normalized.as_bytes())
            .take(50)
            .filter_map(|guard| guard.into_inner().ok())
            .map(|(key, _)| {
                let s = String::from_utf8(key.to_vec()).unwrap_or_default();
                s.split(SHARD_DELIM).next().unwrap_or(&s).to_string()
            })
            .filter(|w| seen.insert(w.clone()))
            .take(10)
            .collect();
        tracing::debug!(collection = %collection, normalized = %normalized, results = results.len(), "suggest completed");
        Ok(results)
    }

    pub fn delete_item(&self, collection: &str, id: &str) -> Result<(), AppError> {
        if self.background_active.load(Ordering::Acquire) {
            self.process_pending_queue()?;
        }
        let id_type = self.validate_collection_exists(collection)?;
        let id_u64 = match id_type {
            IdType::Number => Some(id.parse::<u64>().map_err(|_| {
                AppError::BadRequest(format!(
                    "invalid id '{}': collection '{}' expects numeric ids",
                    id, collection
                ))
            })?),
            IdType::String => None,
        };

        let _lock = self.lock.lock().unwrap();

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let tokens: Vec<String> = match docs.get(id.as_bytes())? {
            Some(data) => decode_rkyv!(Vec<String>, &data)?,
            None => return Err(AppError::NotFound(format!("item '{}' not found", id))),
        };

        for word in &tokens {
            match id_type {
                IdType::Number => posting_list::remove_from_roaring_posting_list(
                    &inverted,
                    word,
                    id_u64.unwrap(),
                )?,
                IdType::String => posting_list::remove_from_posting_list(&inverted, word, id)?,
            }
        }

        docs.remove(id.as_bytes())?;
        tracing::debug!(collection = %collection, id = %id, "item deleted");
        Ok(())
    }

    pub fn collection_info(&self, collection: &str) -> Result<CollectionInfo, AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let mut unique_terms = 0usize;
        for guard in inverted.iter() {
            let (key, value) = guard.into_inner()?;
            if !key.contains(&(SHARD_DELIM as u8)) && value.is_empty() {
                unique_terms += 1;
            }
        }

        Ok(CollectionInfo {
            name: collection.to_string(),
            id_type: format!("{:?}", id_type).to_lowercase(),
            document_count: docs.len()?,
            unique_terms,
        })
    }

    pub fn list_collections(&self) -> Result<ListCollectionsResponse, AppError> {
        let map = self.collections.read().unwrap();
        let collections: Vec<CollectionSummary> = map
            .iter()
            .map(|(name, id_type)| CollectionSummary {
                name: name.clone(),
                id_type: format!("{:?}", id_type).to_lowercase(),
            })
            .collect();
        Ok(ListCollectionsResponse { collections })
    }

    pub fn delete_collection(&self, collection: &str) -> Result<(), AppError> {
        self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        self.collections.write().unwrap().remove(collection);

        let inv_name = format!("{}.inverted", collection);
        if self.db.keyspace_exists(&inv_name) {
            let inv = self
                .db
                .keyspace(&inv_name, || self.config.keyspace_opts())?;
            self.db.delete_keyspace(inv)?;
        }
        let docs_name = format!("{}.docs", collection);
        if self.db.keyspace_exists(&docs_name) {
            let docs = self
                .db
                .keyspace(&docs_name, || self.config.keyspace_opts())?;
            self.db.delete_keyspace(docs)?;
        }

        let meta = self.meta_keyspace()?;
        meta.remove(collection.as_bytes())?;
        tracing::info!(collection = %collection, "collection deleted");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(conf: StoreConfig) -> (Store, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = fjall::Database::builder(dir.path())
            .cache_size(1_000_000)
            .open()
            .unwrap();
        let store = Store::with_config(db, conf);
        (store, dir)
    }

    fn default_store() -> (Store, tempfile::TempDir) {
        test_store(StoreConfig::default())
    }

    #[test]
    fn tokenize_basic() {
        let config = StoreConfig::default();
        let tokens = tokenize("hello world", config.min_token_length);
        let mut sorted: Vec<_> = tokens.into_iter().collect();
        sorted.sort();
        assert_eq!(sorted, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_deduplicates() {
        let config = StoreConfig::default();
        let tokens = tokenize("foo foo foo", config.min_token_length);
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("foo"));
    }

    #[test]
    fn tokenize_short_words_filtered() {
        let config = StoreConfig {
            min_token_length: 3,
            ..Default::default()
        };
        let tokens = tokenize("a an the fox", config.min_token_length);
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains("the"));
        assert!(tokens.contains("fox"));
    }

    #[test]
    fn tokenize_empty() {
        let config = StoreConfig::default();
        let tokens = tokenize("", config.min_token_length);
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_only_short() {
        let config = StoreConfig {
            min_token_length: 10,
            ..Default::default()
        };
        let tokens = tokenize("hello world", config.min_token_length);
        assert!(tokens.is_empty());
    }

    #[test]
    fn shard_key_format() {
        let key = posting_list::shard_key("hello", 42);
        assert_eq!(key, b"hello\x000042");
    }

    #[test]
    fn shard_key_zero_padded() {
        let key = posting_list::shard_key("test", 0);
        assert_eq!(key, b"test\x000000");
        let key = posting_list::shard_key("test", 9999);
        assert_eq!(key, b"test\x009999");
    }

    #[test]
    fn id_type_serde_roundtrip() {
        for id_type in &[IdType::String, IdType::Number] {
            let json = serde_json::to_string(id_type).unwrap();
            let back: IdType = serde_json::from_str(&json).unwrap();
            assert_eq!(*id_type, back);
        }
    }

    #[test]
    fn id_type_string_serde_name() {
        let json = serde_json::to_string(&IdType::String).unwrap();
        assert_eq!(json, "\"string\"");
        let json = serde_json::to_string(&IdType::Number).unwrap();
        assert_eq!(json, "\"number\"");
    }

    #[test]
    fn store_config_defaults() {
        let cfg = StoreConfig::default();
        assert_eq!(cfg.min_token_length, 2);
        assert_eq!(cfg.max_shard_size, 1000);
        assert_eq!(cfg.max_roaring_shard_size, 100_000);
        assert!(cfg.write_buffer_size.is_none());
        assert!(cfg.compression.is_none());
    }

    #[test]
    fn create_and_list_collections() {
        let (store, _dir) = default_store();
        store.create_collection("mycol", "string").unwrap();
        let list = store.list_collections().unwrap();
        assert_eq!(list.collections.len(), 1);
        assert_eq!(list.collections[0].name, "mycol");
        assert_eq!(list.collections[0].id_type, "string");
    }

    #[test]
    fn create_multiple_collections() {
        let (store, _dir) = default_store();
        store.create_collection("a", "string").unwrap();
        store.create_collection("b", "number").unwrap();
        let list = store.list_collections().unwrap();
        assert_eq!(list.collections.len(), 2);
    }

    #[test]
    fn create_duplicate_collection_errors() {
        let (store, _dir) = default_store();
        store.create_collection("mycol", "string").unwrap();
        let err = store.create_collection("mycol", "string").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn create_invalid_id_type_errors() {
        let (store, _dir) = default_store();
        let err = store.create_collection("mycol", "invalid").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn upsert_and_search_string() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn upsert_and_search_number() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        store.upsert("docs", "42", "hello world").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["42"]);
    }

    #[test]
    fn search_multi_token_intersection() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "apple banana").unwrap();
        store.upsert("docs", "2", "apple cherry").unwrap();
        store.upsert("docs", "3", "banana cherry").unwrap();
        let results = store
            .search("docs", "apple banana", false, 10, None)
            .unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn search_no_match() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let results = store
            .search("docs", "nonexistent", false, 10, None)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_empty_query() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let results = store.search("docs", "", false, 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_sort_asc() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["a", "b"]);
    }

    #[test]
    fn search_sort_desc_default() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        let results = store.search("docs", "hello", true, 10, None).unwrap();
        assert_eq!(results, vec!["b", "a"]);
    }

    #[test]
    fn search_pagination_after() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        store.upsert("docs", "c", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, Some("a")).unwrap();
        assert_eq!(results, vec!["b", "c"]);
    }

    #[test]
    fn search_pagination_after_desc() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        store.upsert("docs", "c", "hello").unwrap();
        let results = store.search("docs", "hello", true, 10, Some("c")).unwrap();
        assert_eq!(results, vec!["b", "a"]);
    }

    #[test]
    fn search_take_limit() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        store.upsert("docs", "c", "hello").unwrap();
        let results = store.search("docs", "hello", false, 2, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn upsert_update_reindex() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "apple banana").unwrap();
        store.upsert("docs", "1", "apple cherry").unwrap();
        let r1 = store.search("docs", "banana", false, 10, None).unwrap();
        assert!(r1.is_empty());
        let r2 = store.search("docs", "cherry", false, 10, None).unwrap();
        assert_eq!(r2, vec!["1"]);
        let r3 = store.search("docs", "apple", false, 10, None).unwrap();
        assert_eq!(r3, vec!["1"]);
    }

    #[test]
    fn delete_item() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        store.delete_item("docs", "1").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn delete_nonexistent_item_errors() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        let err = store.delete_item("docs", "1").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn delete_and_reinsert() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.delete_item("docs", "1").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn suggest_basic() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        store.upsert("docs", "2", "helpful tips").unwrap();
        let suggestions = store.suggest("docs", "hel").unwrap();
        assert!(suggestions.contains(&"helpful".to_string()));
        assert!(suggestions.contains(&"hello".to_string()));
    }

    #[test]
    fn suggest_no_matches() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let suggestions = store.suggest("docs", "xyz").unwrap();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn suggest_on_nonexistent_collection() {
        let (store, _dir) = default_store();
        let err = store.suggest("nonexistent", "hel").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn collection_info() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let info = store.collection_info("docs").unwrap();
        assert_eq!(info.name, "docs");
        assert_eq!(info.id_type, "string");
        assert_eq!(info.document_count, 1);
        assert_eq!(info.unique_terms, 2);
    }

    #[test]
    fn collection_info_empty() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        let info = store.collection_info("docs").unwrap();
        assert_eq!(info.document_count, 0);
        assert_eq!(info.unique_terms, 0);
    }

    #[test]
    fn delete_collection() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.delete_collection("docs").unwrap();
        let list = store.list_collections().unwrap();
        assert!(list.collections.is_empty());
    }

    #[test]
    fn search_on_nonexistent_collection() {
        let (store, _dir) = default_store();
        let err = store
            .search("nonexistent", "hello", false, 10, None)
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn upsert_on_nonexistent_collection() {
        let (store, _dir) = default_store();
        let err = store.upsert("nonexistent", "1", "hello").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn upsert_invalid_numeric_id() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        let err = store.upsert("docs", "not-a-number", "hello").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn search_number_sort_asc() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        store.upsert("docs", "3", "hello").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.upsert("docs", "2", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["1", "2", "3"]);
    }

    #[test]
    fn search_number_sort_desc() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.upsert("docs", "2", "hello").unwrap();
        let results = store.search("docs", "hello", true, 10, None).unwrap();
        assert_eq!(results, vec!["2", "1"]);
    }

    #[test]
    fn search_number_pagination() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.upsert("docs", "2", "hello").unwrap();
        store.upsert("docs", "3", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, Some("1")).unwrap();
        assert_eq!(results, vec!["2", "3"]);
    }

    #[test]
    fn upsert_string_idempotent() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn upsert_number_idempotent() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "number").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        store.upsert("docs", "1", "hello").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn shard_splitting_string() {
        let conf = StoreConfig {
            max_shard_size: 3,
            ..Default::default()
        };
        let (store, _dir) = test_store(conf);
        store.create_collection("docs", "string").unwrap();
        for i in 0..10u64 {
            store.upsert("docs", &i.to_string(), "hello").unwrap();
        }
        let results = store.search("docs", "hello", false, 20, None).unwrap();
        assert_eq!(results.len(), 10);
    }

    #[test]
    fn shard_splitting_roaring() {
        let conf = StoreConfig {
            max_roaring_shard_size: 3,
            ..Default::default()
        };
        let (store, _dir) = test_store(conf);
        store.create_collection("docs", "number").unwrap();
        for i in 0..10u64 {
            store.upsert("docs", &i.to_string(), "hello").unwrap();
        }
        let results = store.search("docs", "hello", false, 20, None).unwrap();
        assert_eq!(results.len(), 10);
    }
}
