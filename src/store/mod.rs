use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use rayon::prelude::*;

use charabia::Tokenize;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use roaring::RoaringTreemap;

use crate::error::AppError;
use crate::models::{
    CollectionCreated, CollectionInfo, CollectionSummary, ListCollectionsResponse,
};

pub use config::{CollectionMeta, IdType, StoreConfig};

pub(crate) const META: TableDefinition<&[u8], &[u8]> = TableDefinition::new("meta");
pub(crate) const QUEUE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("queue");
pub(crate) const DOCS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("docs");
pub(crate) const INVERTED: TableDefinition<&[u8], &[u8]> = TableDefinition::new("inverted");

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
    db: Database,
    config: StoreConfig,
    lock: Mutex<()>,
    collections: RwLock<HashMap<String, config::CollectionMeta>>,
    next_seq: AtomicU64,
}

impl Store {
    pub fn new(db: Database) -> Self {
        Self::with_config(db, StoreConfig::default())
    }

    pub fn with_config(db: Database, config: StoreConfig) -> Self {
        let collections = {
            let mut map = HashMap::new();
            if let Ok(txn) = db.begin_read() {
                if let Ok(meta) = txn.open_table(META) {
                    if let Ok(iter) = meta.iter() {
                        for result in iter {
                            if let Ok((key, value)) = result {
                                let name = String::from_utf8_lossy(key.value()).to_string();
                                if let Ok(col_meta) = decode_rkyv!(config::CollectionMeta, value.value()) {
                                    map.insert(name, col_meta);
                                }
                            }
                        }
                    }
                }
            }
            map
        };
        let next_seq = Self::init_next_seq(&db);

        Self {
            db,
            config,
            lock: Mutex::new(()),
            collections: RwLock::new(collections),
            next_seq: AtomicU64::new(next_seq),
        }
    }

    fn init_next_seq(db: &Database) -> u64 {
        let txn = match db.begin_read() {
            Ok(t) => t,
            Err(_) => return 1,
        };
        let table = match txn.open_table(QUEUE) {
            Ok(t) => t,
            Err(_) => return 1,
        };
        let mut max = 0u64;
        if let Ok(iter) = table.iter() {
            for result in iter {
                if let Ok((key, _)) = result {
                    if key.value().len() == 8 {
                        let mut buf = [0u8; 8];
                        buf.copy_from_slice(key.value());
                        let seq = u64::from_be_bytes(buf);
                        if seq > max {
                            max = seq;
                        }
                    }
                }
            }
        }
        max + 1
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

            let txn = self.db.begin_write()?;
            {
                let mut meta = txn.open_table(META)?;
                let value = encode_rkyv!(&col_meta)?;
                meta.insert(name.as_bytes(), value.as_slice())?;
            }
            txn.commit()?;

            map.insert(name.to_string(), col_meta);
        }

        tracing::info!(collection = %name, id_type = %id_type, searchable_fields = ?searchable_fields, "collection created");

        Ok(CollectionCreated {
            name: name.to_string(),
            id_type: id_type.to_string(),
            searchable_fields: searchable_fields.to_vec(),
        })
    }

    pub fn upsert(&self, collection: &str, doc: serde_json::Value) -> Result<(), AppError> {
        let meta = self.validate_collection_exists(collection)?;
        let id = extract_id(&doc, meta.id_type)?;

        let seq = self.allocate_seq();
        let doc_bytes = serde_json::to_vec(&doc).map_err(|e| AppError::Internal(e.to_string()))?;
        let entry = config::QueuedIndex {
            collection: collection.to_string(),
            id: id.to_string(),
            document: doc_bytes,
        };
        let txn = self.db.begin_write()?;
        {
            let mut queue = txn.open_table(QUEUE)?;
            let seq_key = seq.to_be_bytes();
            queue.insert(seq_key.as_slice(), encode_rkyv!(&entry)?.as_slice())?;
        }
        txn.commit()?;
        tracing::debug!(collection = %collection, id = %id, seq = %seq, "item queued for indexing");
        Ok(())
    }

    pub fn process_pending_queue(&self) -> Result<(), AppError> {
        let items: Vec<(Vec<u8>, config::QueuedIndex)> = {
            let txn = match self.db.begin_read() {
                Ok(t) => t,
                Err(_) => return Ok(()),
            };
            let queue = match txn.open_table(QUEUE) {
                Ok(q) => q,
                Err(_) => return Ok(()),
            };
            queue.iter()?
                .filter_map(|result| result.ok())
                .filter(|(key, _)| key.value().len() == 8)
                .filter_map(|(key, value)| {
                    decode_rkyv!(config::QueuedIndex, value.value())
                        .ok()
                        .map(|entry| (key.value().to_vec(), entry))
                })
                .take(self.config.max_queue_batch_size)
                .collect()
        };

        if items.is_empty() {
            return Ok(());
        }

        let txn = self.db.begin_write()?;
        {
            let mut queue = txn.open_table(QUEUE)?;
            let mut docs = txn.open_table(DOCS)?;
            let mut inverted = txn.open_table(INVERTED)?;

            for (key, entry) in &items {
                let doc = match serde_json::from_slice::<serde_json::Value>(&entry.document) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to deserialize queued document");
                        queue.remove(key.as_slice())?;
                        continue;
                    }
                };

                let meta = match self
                    .collections
                    .read()
                    .unwrap()
                    .get(&entry.collection)
                    .cloned()
                {
                    Some(m) => m,
                    None => {
                        tracing::error!(
                            collection = %entry.collection,
                            "queued item references unknown collection"
                        );
                        queue.remove(key.as_slice())?;
                        continue;
                    }
                };

                let content = extract_searchable_content(&doc, &meta.searchable_fields);
                let new_words = tokenize(&content, self.config.min_token_length);

                let doc_key = doc_key(&entry.collection, &entry.id);
                let old_words = match docs.get(doc_key.as_slice())? {
                    Some(old_data) => {
                        if let Ok(old_doc) =
                            serde_json::from_slice::<serde_json::Value>(old_data.value())
                        {
                            let old_content =
                                extract_searchable_content(&old_doc, &meta.searchable_fields);
                            tokenize(&old_content, self.config.min_token_length)
                        } else {
                            HashSet::new()
                        }
                    }
                    None => HashSet::new(),
                };

                let is_new = old_words.is_empty();

                if !is_new {
                    for word in old_words.difference(&new_words) {
                        match meta.id_type {
                            IdType::Number => {
                                let id_u64 = entry.id.parse::<u64>().map_err(|_| {
                                    AppError::Internal(format!(
                                        "invalid numeric id in storage: {}",
                                        entry.id
                                    ))
                                })?;
                                posting_list::remove_from_roaring_posting_list(
                                    &mut inverted,
                                    &entry.collection,
                                    word,
                                    id_u64,
                                )?;
                            }
                            IdType::String => {
                                posting_list::remove_from_posting_list(
                                    &mut inverted,
                                    &entry.collection,
                                    word,
                                    &entry.id,
                                )?;
                            }
                        }
                    }
                }

                docs.insert(doc_key.as_slice(), entry.document.as_slice())?;

                for word in &new_words {
                    match meta.id_type {
                        IdType::Number => {
                            let id_u64 = entry.id.parse::<u64>().map_err(|_| {
                                AppError::Internal(format!(
                                    "invalid numeric id in storage: {}",
                                    entry.id
                                ))
                            })?;
                            posting_list::add_to_roaring_posting_list(
                                &mut inverted,
                                &entry.collection,
                                word,
                                id_u64,
                                self.config.max_roaring_shard_size,
                            )?;
                        }
                        IdType::String => {
                            posting_list::add_to_posting_list(
                                &mut inverted,
                                &entry.collection,
                                word,
                                &entry.id,
                                self.config.max_string_shard_size,
                            )?;
                        }
                    }
                }

                queue.remove(key.as_slice())?;
            }
        }
        txn.commit()?;

        Ok(())
    }

    pub fn flush(&self) -> Result<(), AppError> {
        self.process_pending_queue()
    }

    pub fn queue_depth(&self) -> Result<u64, AppError> {
        let txn = self.db.begin_read()?;
        let Ok(queue) = txn.open_table(QUEUE) else {
            return Ok(0);
        };
        let mut count = 0u64;
        if let Ok(iter) = queue.iter() {
            for _ in iter {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn spawn_background(self: &std::sync::Arc<Self>) {
        let store = std::sync::Arc::clone(self);
        let interval = self.config.index_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let store = std::sync::Arc::clone(&store);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = store.process_pending_queue() {
                        tracing::error!(error = ?e, "background indexing cycle failed");
                    }
                })
                .await
                .ok();
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
        let txn = self.db.begin_read()?;
        let inverted = match txn.open_table(INVERTED) {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };

        let ids = match meta.id_type {
            IdType::Number => search::roaring_search(
                &inverted,
                collection,
                self.config.min_token_length,
                query,
                sort_desc,
                take,
                after,
            )?,
            IdType::String => search::string_search(
                &inverted,
                collection,
                self.config.min_token_length,
                query,
                sort_desc,
                take,
                after,
            )?,
        };

        let docs = match txn.open_table(DOCS) {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        let results: Vec<serde_json::Value> = ids
            .par_iter()
            .filter_map(|id| {
                let key = doc_key(collection, id);
                docs.get(key.as_slice())
                    .ok()
                    .flatten()
                    .and_then(|data| serde_json::from_slice(data.value()).ok())
            })
            .collect();

        tracing::debug!(collection = %collection, query = %query, results = results.len(), "search completed");
        Ok(results)
    }

    pub fn delete_item(&self, collection: &str, id: &str) -> Result<(), AppError> {
        self.process_pending_queue()?;
        let meta = self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        let txn = self.db.begin_write()?;
        {
            let mut inverted = txn.open_table(INVERTED)?;
            let mut docs = txn.open_table(DOCS)?;

            let doc_key = doc_key(collection, id);
            let (_doc, tokens) = {
                let doc_data = docs
                    .get(doc_key.as_slice())?
                    .ok_or_else(|| AppError::NotFound(format!("item '{}' not found", id)))?;
                let doc: serde_json::Value = serde_json::from_slice(doc_data.value())
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let content = extract_searchable_content(&doc, &meta.searchable_fields);
                let tokens = tokenize(&content, self.config.min_token_length);
                (doc, tokens)
            };

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
                            &mut inverted,
                            collection,
                            word,
                            id_u64,
                        )?;
                    }
                    IdType::String => {
                        posting_list::remove_from_posting_list(
                            &mut inverted,
                            collection,
                            word,
                            id,
                        )?;
                    }
                }
            }

            docs.remove(doc_key.as_slice())?;
        }
        txn.commit()?;

        tracing::debug!(collection = %collection, id = %id, "item deleted");
        Ok(())
    }

    pub fn collection_info(&self, collection: &str) -> Result<CollectionInfo, AppError> {
        let meta = self.validate_collection_exists(collection)?;

        let txn = self.db.begin_read()?;
        let prefix = doc_prefix(collection);
        let mut count = 0u64;
        if let Ok(docs) = txn.open_table(DOCS) {
            let mut end_vec = prefix.clone();
            end_vec.push(0xFF);
            for result in docs.range::<&[u8]>(prefix.as_slice()..end_vec.as_slice())? {
                if result.is_ok() {
                    count += 1;
                } else {
                    break;
                }
            }
        }

        Ok(CollectionInfo {
            name: collection.to_string(),
            id_type: format!("{:?}", meta.id_type).to_lowercase(),
            document_count: count as usize,
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
        let mut map = std::collections::HashMap::new();
        if let Ok(txn) = self.db.begin_read() {
            if let Ok(meta) = txn.open_table(META) {
                if let Ok(iter) = meta.iter() {
                    for result in iter {
                        if let Ok((key, value)) = result {
                            let name = String::from_utf8_lossy(key.value()).to_string();
                            if let Ok(col_meta) = decode_rkyv!(config::CollectionMeta, value.value()) {
                                map.insert(name, col_meta);
                            }
                        }
                    }
                }
            }
        }
        *self.collections.write().unwrap() = map;
        Ok(())
    }

    pub fn delete_collection(&self, collection: &str) -> Result<(), AppError> {
        let _meta = self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        self.collections.write().unwrap().remove(collection);

        let prefix = doc_prefix(collection);
        let mut end_vec = prefix.clone();
        end_vec.push(0xFF);
        let start = prefix.as_slice();
        let end = end_vec.as_slice();

        let txn = self.db.begin_write()?;
        {
            let mut docs = txn.open_table(DOCS)?;
            let mut inverted = txn.open_table(INVERTED)?;
            let mut meta = txn.open_table(META)?;

            let doc_keys: Vec<Vec<u8>> = docs
                .range::<&[u8]>(start..end)?
                .filter_map(|r| r.ok())
                .map(|(k, _)| k.value().to_vec())
                .collect();
            for key in &doc_keys {
                docs.remove(key.as_slice())?;
            }

            let inv_keys: Vec<Vec<u8>> = inverted
                .range::<&[u8]>(start..end)?
                .filter_map(|r| r.ok())
                .map(|(k, _)| k.value().to_vec())
                .collect();
            for key in &inv_keys {
                inverted.remove(key.as_slice())?;
            }

            meta.remove(collection.as_bytes())?;
        }
        txn.commit()?;

        tracing::info!(collection = %collection, "collection deleted");
        Ok(())
    }
}

fn doc_key(collection: &str, doc_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(collection.len() + 1 + doc_id.len());
    key.extend_from_slice(collection.as_bytes());
    key.push(0);
    key.extend_from_slice(doc_id.as_bytes());
    key
}

fn doc_prefix(collection: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(collection.len() + 1);
    key.extend_from_slice(collection.as_bytes());
    key.push(0);
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store(conf: StoreConfig) -> (Store, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = redb::Database::builder()
            .set_cache_size(1_000_000)
            .create(dir.path().join("db"))
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
        let key = posting_list::shard_key("col", "hello", 42);
        assert_eq!(key, b"col\x00hello\x000042");
    }

    #[test]
    fn shard_key_zero_padded() {
        let key = posting_list::shard_key("col", "test", 0);
        assert_eq!(key, b"col\x00test\x000000");
        let key = posting_list::shard_key("col", "test", 9999);
        assert_eq!(key, b"col\x00test\x009999");
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
        assert_eq!(cfg.min_token_length, 3);
        assert_eq!(cfg.max_string_shard_size, 1000);
        assert_eq!(cfg.max_roaring_shard_size, 100_000);
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
        store.delete_item("docs", "1").unwrap();
        store
            .upsert("docs", json!({"id": "1", "content": "hello"}))
            .unwrap();
        store.flush().unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(ids(&results), vec!["1"]);
    }

    #[test]
    fn shard_splitting_string() {
        let conf = StoreConfig {
            max_string_shard_size: 3,
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
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
        store.flush().unwrap();
        let results = store.search("docs", "hello", false, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["id"], "1");
        assert_eq!(results[0]["title"], "hello");
        assert_eq!(results[0]["body"], "world");
    }
}
