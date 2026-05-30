use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use charabia::Tokenize;
use roaring::RoaringTreemap;

use crate::error::AppError;
use crate::models::{
    CollectionCreated, CollectionInfo, CollectionSummary, ListCollectionsResponse,
};

pub use config::{CollectionMeta, IdType, StoreConfig};

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

fn extract_searchable_content(doc: &serde_json::Value, fields: &[String]) -> String {
    let mut parts = Vec::new();
    for field in fields {
        if let Some(value) = doc.get(field) {
            match value {
                serde_json::Value::String(s) => parts.push(s.clone()),
                serde_json::Value::Number(n) => parts.push(n.to_string()),
                serde_json::Value::Bool(b) => parts.push(b.to_string()),
                serde_json::Value::Array(arr) => {
                    for v in arr {
                        match v {
                            serde_json::Value::String(s) => parts.push(s.clone()),
                            serde_json::Value::Number(n) => parts.push(n.to_string()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    parts.join(" ")
}

fn extract_id(doc: &serde_json::Value, id_type: IdType) -> Result<String, AppError> {
    let id_val = doc
        .get("id")
        .ok_or_else(|| AppError::BadRequest("missing 'id' field in document".into()))?;
    match id_type {
        IdType::String => id_val.as_str().map(|s| s.to_string()).ok_or_else(|| {
            AppError::BadRequest("'id' must be a string for string collection".into())
        }),
        IdType::Number => match id_val {
            serde_json::Value::Number(n) => Ok(n.to_string()),
            serde_json::Value::String(s) => s.parse::<u64>().map(|n| n.to_string()).map_err(|_| {
                AppError::BadRequest(format!(
                    "'id' must be a valid number for number collection, got '{}'",
                    s
                ))
            }),
            _ => Err(AppError::BadRequest(
                "'id' must be a number for number collection".into(),
            )),
        },
    }
}

mod config;
mod posting_list;
mod search;

pub struct Store {
    db: fjall::Database,
    config: StoreConfig,
    lock: Mutex<()>,
    collections: RwLock<HashMap<String, config::CollectionMeta>>,
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
                        if let Ok(col_meta) = decode_rkyv!(config::CollectionMeta, &value) {
                            map.insert(name, col_meta);
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

    fn validate_collection_exists(
        &self,
        collection: &str,
    ) -> Result<config::CollectionMeta, AppError> {
        self.collections
            .read()
            .unwrap()
            .get(collection)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("collection '{}' not found", collection)))
    }

    pub fn create_collection(
        &self,
        name: &str,
        id_type: &str,
        searchable_fields: &[String],
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

            let col_meta = config::CollectionMeta {
                id_type: id_type_enum,
                searchable_fields: searchable_fields.to_vec(),
            };

            let meta = self.meta_keyspace()?;
            let value = encode_rkyv!(&col_meta)?;
            meta.insert(name.as_bytes(), &value)?;

            map.insert(name.to_string(), col_meta);
        }

        tracing::info!(collection = %name, id_type = %id_type, searchable_fields = ?searchable_fields, "collection created");

        Ok(CollectionCreated {
            name: name.to_string(),
            id_type: id_type.to_string(),
            searchable_fields: searchable_fields.to_vec(),
        })
    }

    fn upsert_apply_batch(
        &self,
        collection: &str,
        id: &str,
        doc: &serde_json::Value,
        batch: &mut fjall::OwnedWriteBatch,
    ) -> Result<(), AppError> {
        let meta = self.validate_collection_exists(collection)?;

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;
        let searchable_content = extract_searchable_content(doc, &meta.searchable_fields);
        let new_words = tokenize(&searchable_content, self.config.min_token_length);

        // If old document exists, compute its tokens and remove diff
        if let Some(old_data) = docs.get(id.as_bytes())?
            && let Ok(old_doc) = serde_json::from_slice::<serde_json::Value>(&old_data)
        {
            let old_content = extract_searchable_content(&old_doc, &meta.searchable_fields);
            let old_words = tokenize(&old_content, self.config.min_token_length);
            for word in old_words.difference(&new_words) {
                match meta.id_type {
                    IdType::Number => {
                        let id_u64 = id.parse::<u64>().map_err(|_| {
                            AppError::Internal(format!("invalid numeric id in storage: {}", id))
                        })?;
                        posting_list::remove_from_roaring_posting_list(
                            batch, &inverted, word, id_u64,
                        )?;
                    }
                    IdType::String => {
                        posting_list::remove_from_posting_list(batch, &inverted, word, id)?;
                    }
                }
            }
        }

        for word in &new_words {
            match meta.id_type {
                IdType::Number => {
                    let id_u64 = id.parse::<u64>().map_err(|_| {
                        AppError::Internal(format!("invalid numeric id in storage: {}", id))
                    })?;
                    posting_list::add_to_roaring_posting_list(
                        batch,
                        &inverted,
                        word,
                        id_u64,
                        self.config.max_roaring_shard_size,
                    )?;
                }
                IdType::String => {
                    posting_list::add_to_posting_list(
                        batch,
                        &inverted,
                        word,
                        id,
                        self.config.max_shard_size,
                    )?;
                }
            }
        }

        let doc_bytes = serde_json::to_vec(doc).map_err(|e| AppError::Internal(e.to_string()))?;
        batch.insert(&docs, id.as_bytes(), &doc_bytes);
        tracing::debug!(collection = %collection, id = %id, tokens = new_words.len(), "item upserted");
        Ok(())
    }

    fn upsert_internal(
        &self,
        collection: &str,
        id: &str,
        doc: &serde_json::Value,
    ) -> Result<(), AppError> {
        let mut batch = fjall::OwnedWriteBatch::with_capacity(self.db.clone(), 64);
        self.upsert_apply_batch(collection, id, doc, &mut batch)?;
        batch.commit()?;
        Ok(())
    }

    pub fn upsert(&self, collection: &str, doc: serde_json::Value) -> Result<(), AppError> {
        let meta = self.validate_collection_exists(collection)?;
        let id = extract_id(&doc, meta.id_type)?;

        if !self.background_active.load(Ordering::Acquire) {
            return self.upsert_internal(collection, &id, &doc);
        }

        let seq = self.allocate_seq();
        let queue = self.queue_keyspace()?;
        let doc_bytes = serde_json::to_vec(&doc).map_err(|e| AppError::Internal(e.to_string()))?;
        let entry = config::QueuedIndex {
            collection: collection.to_string(),
            id: id.to_string(),
            document: doc_bytes,
        };
        queue.insert(seq.to_be_bytes(), &encode_rkyv!(&entry)?)?;
        tracing::debug!(collection = %collection, id = %id, seq = %seq, "item queued for indexing");
        Ok(())
    }

    pub fn process_pending_queue(&self) -> Result<(), AppError> {
        let queue = self.queue_keyspace()?;
        let items: Vec<(Vec<u8>, config::QueuedIndex)> = queue
            .iter()
            .filter_map(|guard| guard.into_inner().ok())
            .filter(|(key, _)| key.len() == 8)
            .filter_map(|(key, value)| {
                decode_rkyv!(config::QueuedIndex, &value)
                    .ok()
                    .map(|entry| (key.to_vec(), entry))
            })
            .collect();

        if items.is_empty() {
            return Ok(());
        }

        let mut batch = fjall::OwnedWriteBatch::with_capacity(self.db.clone(), items.len() * 16);

        for (key, entry) in &items {
            if let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&entry.document)
                && let Err(e) =
                    self.upsert_apply_batch(&entry.collection, &entry.id, &doc, &mut batch)
            {
                tracing::error!(
                    error = ?e,
                    collection = %entry.collection,
                    id = %entry.id,
                    "failed to index queued item"
                );
            }
            batch.remove(&queue, key);
        }

        batch.commit()?;
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
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let meta = self.validate_collection_exists(collection)?;
        let inverted = self.inverted_keyspace(collection)?;
        let ids = match meta.id_type {
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

        let docs = self.docs_keyspace(collection)?;
        let results: Vec<serde_json::Value> = ids
            .iter()
            .filter_map(|id| {
                docs.get(id.as_bytes())
                    .ok()
                    .flatten()
                    .and_then(|data| serde_json::from_slice(&data).ok())
            })
            .collect();

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
        let meta = self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let doc_data = docs
            .get(id.as_bytes())?
            .ok_or_else(|| AppError::NotFound(format!("item '{}' not found", id)))?;
        let doc: serde_json::Value =
            serde_json::from_slice(&doc_data).map_err(|e| AppError::Internal(e.to_string()))?;
        let content = extract_searchable_content(&doc, &meta.searchable_fields);
        let tokens = tokenize(&content, self.config.min_token_length);

        let mut batch = fjall::OwnedWriteBatch::with_capacity(self.db.clone(), tokens.len() + 1);

        for word in &tokens {
            match meta.id_type {
                IdType::Number => {
                    let id_u64 = id.parse::<u64>().map_err(|_| {
                        AppError::BadRequest(format!(
                            "invalid id '{}': collection '{}' expects numeric ids",
                            id, collection
                        ))
                    })?;
                    posting_list::remove_from_roaring_posting_list(
                        &mut batch, &inverted, word, id_u64,
                    )?;
                }
                IdType::String => {
                    posting_list::remove_from_posting_list(&mut batch, &inverted, word, id)?
                }
            }
        }

        batch.remove(&docs, id.as_bytes());
        batch.commit()?;
        tracing::debug!(collection = %collection, id = %id, "item deleted");
        Ok(())
    }

    pub fn collection_info(&self, collection: &str) -> Result<CollectionInfo, AppError> {
        let meta = self.validate_collection_exists(collection)?;
        let docs = self.docs_keyspace(collection)?;

        Ok(CollectionInfo {
            name: collection.to_string(),
            id_type: format!("{:?}", meta.id_type).to_lowercase(),
            document_count: docs.len()?,
            searchable_fields: meta.searchable_fields,
        })
    }

    pub fn list_collections(&self) -> Result<ListCollectionsResponse, AppError> {
        let map = self.collections.read().unwrap();
        let collections: Vec<CollectionSummary> = map
            .iter()
            .map(|(name, meta)| CollectionSummary {
                name: name.clone(),
                id_type: format!("{:?}", meta.id_type).to_lowercase(),
                searchable_fields: meta.searchable_fields.clone(),
            })
            .collect();
        Ok(ListCollectionsResponse { collections })
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, AppError> {
        self.process_pending_queue()?;
        crate::backup::export_snapshot(&self.db)
    }

    pub fn import_snapshot(&self, data: &[u8]) -> Result<(), AppError> {
        crate::backup::import_snapshot(&self.db, data)?;
        self.refresh_collections_cache()?;
        Ok(())
    }

    fn refresh_collections_cache(&self) -> Result<(), AppError> {
        let meta = self.meta_keyspace()?;
        let mut map = std::collections::HashMap::new();
        for guard in meta.iter() {
            if let Ok((key, value)) = guard.into_inner() {
                let name = String::from_utf8_lossy(&key).to_string();
                if let Ok(col_meta) = decode_rkyv!(config::CollectionMeta, &value) {
                    map.insert(name, col_meta);
                }
            }
        }
        *self.collections.write().unwrap() = map;
        Ok(())
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
    use serde_json::json;

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

    fn ids(results: &[serde_json::Value]) -> Vec<String> {
        results
            .iter()
            .filter_map(|v| match &v["id"] {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect()
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
        store.create_collection("mycol", "string", &[]).unwrap();
        let list = store.list_collections().unwrap();
        assert_eq!(list.collections.len(), 1);
        assert_eq!(list.collections[0].name, "mycol");
        assert_eq!(list.collections[0].id_type, "string");
        assert!(list.collections[0].searchable_fields.is_empty());
    }

    #[test]
    fn create_collection_with_fields() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["title".into(), "body".into()])
            .unwrap();
        let list = store.list_collections().unwrap();
        assert_eq!(list.collections[0].searchable_fields, vec!["title", "body"]);
        let info = store.collection_info("docs").unwrap();
        assert_eq!(info.searchable_fields, vec!["title", "body"]);
    }

    #[test]
    fn create_multiple_collections() {
        let (store, _dir) = default_store();
        store.create_collection("a", "string", &[]).unwrap();
        store.create_collection("b", "number", &[]).unwrap();
        let list = store.list_collections().unwrap();
        assert_eq!(list.collections.len(), 2);
    }

    #[test]
    fn create_duplicate_collection_errors() {
        let (store, _dir) = default_store();
        store.create_collection("mycol", "string", &[]).unwrap();
        let err = store.create_collection("mycol", "string", &[]).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn create_invalid_id_type_errors() {
        let (store, _dir) = default_store();
        let err = store
            .create_collection("mycol", "invalid", &[])
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn upsert_and_search_string() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn upsert_and_search_number() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": 42, "content": "hello world"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["42"]);
    }

    #[test]
    fn search_multi_token_intersection() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "apple banana"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "2", "content": "apple cherry"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "3", "content": "banana cherry"}))
            .unwrap();
        let results = store
            .search("docs", "apple banana", false, 10, None)
            .unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn search_no_match() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        let results = store
            .search("docs", "nonexistent", false, 10, None)
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_empty_query() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        let results = store.search("docs", "", false, 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_sort_asc() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "b", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "a", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["a", "b"]);
    }

    #[test]
    fn search_sort_desc_default() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "a", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "b", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", true, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["b", "a"]);
    }

    #[test]
    fn search_pagination_after() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "a", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "b", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "c", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, Some("a")).unwrap();
        assert_eq!(ids(&results), vec!["b", "c"]);
    }

    #[test]
    fn search_pagination_after_desc() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "a", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "b", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "c", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", true, 10, Some("c")).unwrap();
        assert_eq!(ids(&results), vec!["b", "a"]);
    }

    #[test]
    fn search_take_limit() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "a", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "b", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "c", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 2, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn upsert_update_reindex() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "apple banana"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "apple cherry"}))
            .unwrap();
        let r1 = store.search("docs", "banana", false, 10, None).unwrap();
        assert!(r1.is_empty());
        let r2 = store.search("docs", "cherry", false, 10, None).unwrap();
        assert_eq!(ids(&r2), vec!["1"]);
        let r3 = store.search("docs", "apple", false, 10, None).unwrap();
        assert_eq!(ids(&r3), vec!["1"]);
    }

    #[test]
    fn delete_item() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        store.delete_item("docs", "1").unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn delete_nonexistent_item_errors() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        let err = store.delete_item("docs", "1").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn delete_and_reinsert() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
        store.delete_item("docs", "1").unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn suggest_basic() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "2", "content": "helpful tips"}))
            .unwrap();
        let suggestions = store.suggest("docs", "hel").unwrap();
        assert!(suggestions.contains(&"helpful".to_string()));
        assert!(suggestions.contains(&"hello".to_string()));
    }

    #[test]
    fn suggest_no_matches() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
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
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello world"}))
            .unwrap();
        let info = store.collection_info("docs").unwrap();
        assert_eq!(info.name, "docs");
        assert_eq!(info.id_type, "string");
        assert_eq!(info.document_count, 1);
    }

    #[test]
    fn collection_info_empty() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        let info = store.collection_info("docs").unwrap();
        assert_eq!(info.document_count, 0);
    }

    #[test]
    fn delete_collection() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
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
        let err = store
            .upsert("nonexistent", json!({"id": "1", "content": "hello"}))
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn upsert_invalid_numeric_id() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        let err = store
            .upsert("docs", json!({"id": "not-a-number", "content": "hello"}))
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn search_number_sort_asc() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": 3, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 1, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 2, "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1", "2", "3"]);
    }

    #[test]
    fn search_number_sort_desc() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": 1, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 2, "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", true, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["2", "1"]);
    }

    #[test]
    fn search_number_pagination() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": 1, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 2, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 3, "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, Some("1")).unwrap();
        assert_eq!(ids(&results), vec!["2", "3"]);
    }

    #[test]
    fn upsert_string_idempotent() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn upsert_number_idempotent() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        store
            .upsert("docs", json!({"id": 1, "content": "hello"}))
            .unwrap();
        store
            .upsert("docs", json!({"id": 1, "content": "hello"}))
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn shard_splitting_string() {
        let conf = StoreConfig {
            max_shard_size: 3,
            ..Default::default()
        };
        let (store, _dir) = test_store(conf);
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        for i in 0..10u64 {
            store
                .upsert("docs", json!({"id": i.to_string(), "content": "hello"}))
                .unwrap();
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
        store
            .create_collection("docs", "number", &["content".into()])
            .unwrap();
        for i in 0..10u64 {
            store
                .upsert("docs", json!({"id": i, "content": "hello"}))
                .unwrap();
        }
        let results = store.search("docs", "hello", false, 20, None).unwrap();
        assert_eq!(results.len(), 10);
    }

    #[test]
    fn searchable_fields_only_indexed() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["title".into()])
            .unwrap();
        store
            .upsert(
                "docs",
                json!({"id": "1", "title": "hello", "body": "world", "ignored": "yes"}),
            )
            .unwrap();
        // "world" should not be indexed because "body" is not a searchable field
        let r1 = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(r1.len(), 1);
        let r2 = store.search("docs", "world", false, 10, None).unwrap();
        assert!(r2.is_empty());
        // Full document should include all fields
        assert_eq!(r1[0]["id"], "1");
        assert_eq!(r1[0]["title"], "hello");
        assert_eq!(r1[0]["body"], "world");
        assert_eq!(r1[0]["ignored"], "yes");
    }

    #[test]
    fn upsert_missing_id_errors() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["content".into()])
            .unwrap();
        let err = store
            .upsert("docs", json!({"content": "hello"}))
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn search_returns_full_documents() {
        let (store, _dir) = default_store();
        store
            .create_collection("docs", "string", &["title".into(), "body".into()])
            .unwrap();
        store
            .upsert(
                "docs",
                json!({"id": "1", "title": "hello", "body": "world"}),
            )
            .unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["id"], "1");
        assert_eq!(results[0]["title"], "hello");
        assert_eq!(results[0]["body"], "world");
    }
}
