use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use charabia::Tokenize;
use heed::types::Unit;
use heed::{BoxedError, BytesDecode, BytesEncode};
use roaring::RoaringTreemap;

use crate::error::AppError;
use crate::models::{
    CollectionCreated, CollectionInfo, CollectionSummary, ListCollectionsResponse,
};

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

pub use config::{CollectionMeta, FSTConfig, IdType, StoreConfig};

mod config;
mod fst;
mod posting_list;
mod search;

pub use fst::FSTPool;

fn roaring_to_vec(b: &RoaringTreemap) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::with_capacity(b.serialized_size());
    b.serialize_into(&mut buf)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(buf)
}

fn roaring_from_slice(bytes: &[u8]) -> Result<RoaringTreemap, AppError> {
    RoaringTreemap::deserialize_from(bytes).map_err(|e| AppError::Internal(e.to_string()))
}

fn tokenize(content: &str, min_token_length: usize, max_token_length: usize) -> HashSet<String> {
    content
        .tokenize()
        .filter(|t| t.is_word())
        .map(|t| t.lemma().to_string())
        .filter(|w| w.len() >= min_token_length && w.len() <= max_token_length)
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

/// Maximum length (in bytes) of a collection name. Collection names appear
/// inside LMDB keys and FST filenames; capping length keeps both well-behaved
/// (LMDB's default max key size is 511 bytes, and we want plenty of headroom
/// for the word + shard suffix).
const COLLECTION_NAME_MAX_LEN: usize = 64;

/// Validate a collection name against the on-disk safe-character set used by
/// [`fst::FSTPool::collection_path`]. The FST encoding lossy-maps any
/// character outside `[A-Za-z0-9_-]` to `_`, so two collections with names
/// like `"foo bar"` and `"foo/bar"` would map to the same FST file and
/// silently share / clobber each other's vocabulary indices. By restricting
/// names to that exact set at creation time we make the encoding bijective
/// (one collection → one FST file) and remove the collision risk.
pub(crate) fn validate_collection_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "collection name must not be empty".into(),
        ));
    }
    if name.len() > COLLECTION_NAME_MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "collection name must be at most {COLLECTION_NAME_MAX_LEN} bytes long, got {}",
            name.len()
        )));
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
    {
        return Err(AppError::BadRequest(format!(
            "collection name '{name}' contains invalid character {bad:?}; only ASCII letters, digits, '_' and '-' are allowed"
        )));
    }
    Ok(())
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
            // Only accept JSON numbers that round-trip to a u64. This rejects
            // floats (incl. whole-number floats like 1.0, whose to_string()
            // would be "1.0" and break later u64 parsing) and negatives.
            serde_json::Value::Number(n) => n.as_u64().map(|v| v.to_string()).ok_or_else(|| {
                AppError::BadRequest(format!(
                    "'id' must be a non-negative integer that fits in u64 for number collection, got {n}"
                ))
            }),
            // Also accept stringified integers — but only those that parse as
            // u64, not "1.0", "1e3", "+1", etc.
            serde_json::Value::String(s) => {
                s.parse::<u64>().map(|n| n.to_string()).map_err(|parse_err| {
                    AppError::BadRequest(format!(
                        "'id' must be a valid non-negative integer for number collection, got '{s}': {parse_err}"
                    ))
                })
            }
            _ => Err(AppError::BadRequest(
                "'id' must be a number for number collection".into(),
            )),
        },
    }
}

/// A bytes codec that returns `Vec<u8>` (Sized) for decoded data.
pub struct Raw;

impl<'a> BytesEncode<'a> for Raw {
    type EItem = [u8];

    fn bytes_encode(item: &'a [u8]) -> Result<Cow<'a, [u8]>, BoxedError> {
        Ok(Cow::Borrowed(item))
    }
}

impl<'a> BytesDecode<'a> for Raw {
    type DItem = Vec<u8>;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Vec<u8>, BoxedError> {
        Ok(bytes.to_vec())
    }
}

pub type DbBytes = heed::Database<Raw, Raw>;

pub struct Store {
    env: heed::Env,
    db_meta: DbBytes,
    db_queue: DbBytes,
    db_docs: DbBytes,
    db_inverted: DbBytes,
    config: StoreConfig,
    lock: Mutex<()>,
    collections: RwLock<HashMap<String, config::CollectionMeta>>,
    next_seq: AtomicU64,
    pub fst_pool: FSTPool,
}

impl Store {
    pub fn new(env: heed::Env, fst_path: PathBuf) -> Self {
        Self::with_config(env, StoreConfig::default(), fst_path)
    }

    pub fn with_config(env: heed::Env, config: StoreConfig, fst_path: PathBuf) -> Self {
        let mut wtxn = env.write_txn().unwrap();
        let db_meta = env.create_database(&mut wtxn, Some("meta")).unwrap();
        let db_queue = env.create_database(&mut wtxn, Some("queue")).unwrap();
        let db_docs = env.create_database(&mut wtxn, Some("docs")).unwrap();
        let db_inverted = env.create_database(&mut wtxn, Some("inverted")).unwrap();
        wtxn.commit().unwrap();

        let collections = HashMap::new();
        let next_seq = Self::init_next_seq(&env, db_queue);
        let fst_pool = FSTPool::new(fst_path.clone(), config.fst_config);
        if !config.fst_config.enabled {
            // Wipe any existing FST files when disabling the feature
            fst_pool.clear_all();
        }

        let store = Self {
            env,
            db_meta,
            db_queue,
            db_docs,
            db_inverted,
            config,
            lock: Mutex::new(()),
            collections: RwLock::new(collections),
            next_seq: AtomicU64::new(next_seq),
            fst_pool,
        };

        // Warm the collections cache from persisted metadata before any
        // request can race in. Without this, the background indexer can run
        // before any user request has called `validate_collection_exists`,
        // see an empty cache, and incorrectly conclude that queued items
        // reference unknown collections — dropping them. Warming is
        // best-effort: any failure leaves the cache empty and the
        // db_meta-fallback path inside `process_pending_queue` will recover
        // each entry lazily on first access.
        if let Err(e) = store.warm_collections_cache() {
            tracing::warn!(
                error = %e,
                "failed to warm collections cache at startup; the db_meta fallback in process_pending_queue will recover lazily"
            );
        }

        store
    }

    /// Populate the in-memory collections cache from `db_meta`. Idempotent —
    /// callable at startup or after an external state change. Iteration
    /// errors and individual decode errors are logged but do not abort the
    /// warming pass, so one malformed row cannot break the whole startup
    /// path.
    fn warm_collections_cache(&self) -> Result<(), AppError> {
        let rtxn = self.env.read_txn()?;
        let iter = self.db_meta.iter(&rtxn)?;
        let mut loaded = 0usize;
        for entry in iter {
            let (name_bytes, value) = match entry {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!(error = %e, "iter error walking db_meta during warm");
                    continue;
                }
            };
            let name = match std::str::from_utf8(&name_bytes) {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "skipping non-utf8 collection name in db_meta during warm"
                    );
                    continue;
                }
            };
            let meta = match decode_rkyv!(config::CollectionMeta, &value) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        collection = %name,
                        error = %e,
                        "skipping undecodable collection meta during warm"
                    );
                    continue;
                }
            };
            match self.collections.write() {
                Ok(mut map) => {
                    map.insert(name, meta);
                    loaded += 1;
                }
                Err(e) => {
                    // Poisoned lock — log and abort the warm; subsequent
                    // callers will hit the same poison and recover via the
                    // lazy fallback.
                    return Err(AppError::Internal(format!(
                        "collections cache lock poisoned during warm: {e}"
                    )));
                }
            }
        }
        tracing::debug!(loaded, "warmed collections cache from db_meta");
        Ok(())
    }

    fn init_next_seq(env: &heed::Env, db_queue: DbBytes) -> u64 {
        let rtxn = match env.read_txn() {
            Ok(t) => t,
            Err(_) => return 1,
        };
        let mut real_max = 0u64;
        let iter = db_queue.remap_data_type::<Unit>().iter(&rtxn);
        let all_keys: Vec<Vec<u8>> = match iter {
            Ok(it) => it.filter_map(|r| r.ok()).map(|(k, _)| k).collect(),
            Err(_) => Vec::new(),
        };
        for key in all_keys {
            if key.len() == 8 {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&key);
                let seq = u64::from_be_bytes(buf);
                if seq > real_max {
                    real_max = seq;
                }
            }
        }
        real_max + 1
    }

    fn allocate_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    fn validate_collection_exists(
        &self,
        collection: &str,
    ) -> Result<config::CollectionMeta, AppError> {
        // Check cache first
        if let Some(meta) = self.collections.read().unwrap().get(collection).cloned() {
            return Ok(meta);
        }
        // Fall back to database
        if let Ok(rtxn) = self.env.read_txn()
            && let Ok(Some(data)) = self.db_meta.get(&rtxn, collection.as_bytes())
            && let Ok(meta) = decode_rkyv!(config::CollectionMeta, &data)
        {
            self.collections
                .write()
                .unwrap()
                .insert(collection.to_string(), meta.clone());
            return Ok(meta);
        }
        Err(AppError::NotFound(format!(
            "collection '{}' not found",
            collection
        )))
    }

    pub fn create_collection(
        &self,
        name: &str,
        id_type: &str,
        searchable_fields: &[String],
    ) -> Result<CollectionCreated, AppError> {
        validate_collection_name(name)?;
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

            let mut wtxn = self.env.write_txn()?;
            let value = encode_rkyv!(&col_meta)?;
            self.db_meta
                .put(&mut wtxn, name.as_bytes(), value.as_slice())?;
            wtxn.commit()?;

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
        let mut wtxn = self.env.write_txn()?;
        let seq_key = seq.to_be_bytes();
        self.db_queue.put(
            &mut wtxn,
            seq_key.as_slice(),
            encode_rkyv!(&entry)?.as_slice(),
        )?;
        wtxn.commit()?;
        tracing::debug!(collection = %collection, id = %id, seq = %seq, "item queued for indexing");
        Ok(())
    }

    pub fn bulk_ingest(
        &self,
        collection: &str,
        docs: Vec<serde_json::Value>,
    ) -> Result<usize, AppError> {
        let meta = self.validate_collection_exists(collection)?;
        let mut wtxn = self.env.write_txn()?;
        let n = docs.len();
        for doc in docs {
            let id = extract_id(&doc, meta.id_type)?;
            let seq = self.allocate_seq();
            let doc_bytes =
                serde_json::to_vec(&doc).map_err(|e| AppError::Internal(e.to_string()))?;
            let entry = config::QueuedIndex {
                collection: collection.to_string(),
                id: id.to_string(),
                document: doc_bytes,
            };
            let seq_key = seq.to_be_bytes();
            self.db_queue.put(
                &mut wtxn,
                seq_key.as_slice(),
                encode_rkyv!(&entry)?.as_slice(),
            )?;
        }
        wtxn.commit()?;
        tracing::info!(collection = %collection, count = n, "bulk ingested items");
        Ok(n)
    }

    pub fn process_pending_queue(&self) -> Result<(), AppError> {
        let items: Vec<(Vec<u8>, config::QueuedIndex)> = {
            let rtxn = match self.env.read_txn() {
                Ok(t) => t,
                Err(_) => return Ok(()),
            };
            let mut raw_items = Vec::new();
            if let Ok(iter) = self.db_queue.iter(&rtxn) {
                for result in iter {
                    let (key, value) = match result {
                        Ok(pair) => pair,
                        _ => continue,
                    };
                    if key.len() != 8 {
                        continue;
                    }
                    match decode_rkyv!(config::QueuedIndex, &value) {
                        Ok(entry) => raw_items.push((key, entry)),
                        Err(_) => continue,
                    }
                    if raw_items.len() >= self.config.max_queue_batch_size {
                        break;
                    }
                }
            }
            raw_items
        };

        if items.is_empty() {
            return Ok(());
        }

        let mut wtxn = self.env.write_txn()?;

        // Track per-collection word changes for FST
        let mut new_words_per_collection: HashMap<String, HashSet<String>> = HashMap::new();
        let mut removed_words_per_collection: HashMap<String, HashSet<String>> = HashMap::new();

        for (key, entry) in &items {
            let doc = match serde_json::from_slice::<serde_json::Value>(&entry.document) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(error = %e, "failed to deserialize queued document");
                    self.db_queue.delete(&mut wtxn, key.as_slice())?;
                    continue;
                }
            };

            // Look up the collection meta, falling back to db_meta on cache
            // miss (the cache is empty until something populates it; without
            // this fallback the background indexer could drop every queued
            // item on process restart before any user request had warmed
            // the cache). A poisoned cache lock is an internal-error
            // condition; propagate it rather than mask it.
            let cache_hit = self
                .collections
                .read()
                .map_err(|e| {
                    AppError::Internal(format!("collections cache read lock poisoned: {e}"))
                })?
                .get(&entry.collection)
                .cloned();
            let meta = match cache_hit {
                Some(m) => m,
                None => match self.db_meta.get(&wtxn, entry.collection.as_bytes())? {
                    Some(data) => match decode_rkyv!(config::CollectionMeta, &data) {
                        Ok(m) => {
                            self.collections
                                .write()
                                .map_err(|e| {
                                    AppError::Internal(format!(
                                        "collections cache write lock poisoned: {e}"
                                    ))
                                })?
                                .insert(entry.collection.clone(), m.clone());
                            m
                        }
                        Err(decode_err) => {
                            tracing::error!(
                                collection = %entry.collection,
                                error = %decode_err,
                                "dropping queue entry: collection meta in db_meta is undecodable"
                            );
                            self.db_queue.delete(&mut wtxn, key.as_slice())?;
                            continue;
                        }
                    },
                    None => {
                        tracing::error!(
                            collection = %entry.collection,
                            "dropping queue entry: collection does not exist in db_meta"
                        );
                        self.db_queue.delete(&mut wtxn, key.as_slice())?;
                        continue;
                    }
                },
            };

            // Resolve the id once, against the collection's id_type. For
            // number-id collections, parse to u64; if a stale queue entry
            // has a malformed id (e.g. one written by an older binary that
            // accepted floats), drop it now — propagating the parse error
            // would abort the whole batch txn and leave the bad entry in
            // the queue, stalling indexing permanently. The enum below
            // carries the parsed value alongside the variant so the call
            // sites don't need to unwrap an Option.
            enum ResolvedId<'a> {
                Number(u64),
                String(&'a str),
            }
            let resolved_id = match meta.id_type {
                IdType::Number => match entry.id.parse::<u64>() {
                    Ok(v) => ResolvedId::Number(v),
                    Err(parse_err) => {
                        tracing::error!(
                            collection = %entry.collection,
                            id = %entry.id,
                            error = %parse_err,
                            "dropping queue entry: non-numeric id in number-id collection"
                        );
                        self.db_queue.delete(&mut wtxn, key.as_slice())?;
                        continue;
                    }
                },
                IdType::String => ResolvedId::String(entry.id.as_str()),
            };

            let content = extract_searchable_content(&doc, &meta.searchable_fields);
            let new_words = tokenize(
                &content,
                self.config.min_token_length,
                self.config.max_token_length,
            );

            let doc_key = doc_key(&entry.collection, &entry.id);
            if doc_key.len() > 500 {
                tracing::warn!(
                    collection = %entry.collection,
                    doc_id = %entry.id,
                    doc_key_len = doc_key.len(),
                    "skipping document with oversized doc_key"
                );
                self.db_queue.delete(&mut wtxn, key.as_slice())?;
                continue;
            }
            let old_words = match self.db_docs.get(&wtxn, doc_key.as_slice())? {
                Some(old_data) => {
                    if let Ok(old_doc) = serde_json::from_slice::<serde_json::Value>(&old_data) {
                        let old_content =
                            extract_searchable_content(&old_doc, &meta.searchable_fields);
                        tokenize(
                            &old_content,
                            self.config.min_token_length,
                            self.config.max_token_length,
                        )
                    } else {
                        HashSet::new()
                    }
                }
                None => HashSet::new(),
            };

            let is_new = old_words.is_empty();

            if !is_new {
                for word in old_words.difference(&new_words) {
                    match resolved_id {
                        ResolvedId::Number(id_u64) => {
                            posting_list::remove_from_roaring_posting_list(
                                self.db_inverted,
                                &mut wtxn,
                                &entry.collection,
                                word,
                                id_u64,
                            )?;
                        }
                        ResolvedId::String(id_str) => {
                            posting_list::remove_from_posting_list(
                                self.db_inverted,
                                &mut wtxn,
                                &entry.collection,
                                word,
                                id_str,
                            )?;
                        }
                    }
                    removed_words_per_collection
                        .entry(entry.collection.clone())
                        .or_default()
                        .insert(word.clone());
                }
            }

            self.db_docs
                .put(&mut wtxn, doc_key.as_slice(), entry.document.as_slice())?;

            for word in &new_words {
                match resolved_id {
                    ResolvedId::Number(id_u64) => {
                        posting_list::add_to_roaring_posting_list(
                            self.db_inverted,
                            &mut wtxn,
                            &entry.collection,
                            word,
                            id_u64,
                            self.config.max_roaring_shard_size,
                        )?;
                    }
                    ResolvedId::String(id_str) => {
                        posting_list::add_to_posting_list(
                            self.db_inverted,
                            &mut wtxn,
                            &entry.collection,
                            word,
                            id_str,
                            self.config.max_string_shard_size,
                        )?;
                    }
                }
                new_words_per_collection
                    .entry(entry.collection.clone())
                    .or_default()
                    .insert(word.clone());
            }

            self.db_queue.delete(&mut wtxn, key.as_slice())?;
        }

        wtxn.commit().map_err(|e| {
            tracing::error!(error = %e, "failed to commit indexing transaction");
            AppError::Internal(format!("indexing commit failed: {e}"))
        })?;

        // Update FST after successful LMDB commit
        for (collection, words) in &new_words_per_collection {
            self.fst_pool.push_words(collection, words);
        }
        for (collection, words) in &removed_words_per_collection {
            self.fst_pool.pop_words(collection, words);
        }

        Ok(())
    }
    pub fn flush(&self) -> Result<(), AppError> {
        self.process_pending_queue()
    }

    pub fn queue_depth(&self) -> Result<u64, AppError> {
        let rtxn = match self.env.read_txn() {
            Ok(t) => t,
            Err(_) => return Ok(0),
        };
        match self.db_queue.len(&rtxn) {
            Ok(n) => Ok(n),
            Err(_) => Ok(0),
        }
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
                    store.fst_pool.consolidate_dirty();
                })
                .await
                .ok();
            }
        });
    }

    pub fn suggest(
        &self,
        collection: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        let _meta = self.validate_collection_exists(collection)?;
        if !self.config.fst_config.enabled {
            return Ok(Vec::new());
        }
        // Use the last whitespace-delimited token as the prefix.
        // This allows multi-word inputs like "hello wo" to suggest
        // terms matching "wo" (e.g. "world", "wonder").
        let prefix = query
            .split_whitespace()
            .last()
            .filter(|s| !s.is_empty())
            .unwrap_or(query);
        Ok(self.fst_pool.suggest_prefix(collection, prefix, limit))
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
        let rtxn = self.env.read_txn()?;

        let fst_pool = if self.config.fst_config.enabled {
            Some(&self.fst_pool)
        } else {
            None
        };
        let fuzzy_max = self.config.fuzzy_max_expansions;

        let ids = match meta.id_type {
            IdType::Number => search::roaring_search(search::SearchParams {
                inverted: self.db_inverted,
                txn: &rtxn,
                collection,
                config_min_token_length: self.config.min_token_length,
                config_max_token_length: self.config.max_token_length,
                query,
                sort_desc,
                take,
                after,
                fuzzy_max_expansions: fuzzy_max,
                fst_pool,
            })?,
            IdType::String => search::string_search(search::SearchParams {
                inverted: self.db_inverted,
                txn: &rtxn,
                collection,
                config_min_token_length: self.config.min_token_length,
                config_max_token_length: self.config.max_token_length,
                query,
                sort_desc,
                take,
                after,
                fuzzy_max_expansions: fuzzy_max,
                fst_pool,
            })?,
        };

        let results: Vec<serde_json::Value> = ids
            .iter()
            .filter_map(|id| {
                let key = doc_key(collection, id);
                let doc_data: Vec<u8> = match self.db_docs.get(&rtxn, key.as_slice()) {
                    Ok(Some(v)) => v,
                    _ => return None,
                };
                serde_json::from_slice(&doc_data).ok()
            })
            .collect();

        tracing::debug!(collection = %collection, query = %query, results = results.len(), "search completed");
        Ok(results)
    }

    pub fn delete_item(&self, collection: &str, id: &str) -> Result<(), AppError> {
        self.process_pending_queue()?;
        let meta = self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        let mut wtxn = self.env.write_txn()?;

        let doc_key = doc_key(collection, id);
        let (_doc, tokens) = {
            let doc_data: Vec<u8> = self
                .db_docs
                .get(&wtxn, doc_key.as_slice())?
                .ok_or_else(|| AppError::NotFound(format!("item '{}' not found", id)))?;
            let doc: serde_json::Value =
                serde_json::from_slice(&doc_data).map_err(|e| AppError::Internal(e.to_string()))?;
            let content = extract_searchable_content(&doc, &meta.searchable_fields);
            let tokens = tokenize(
                &content,
                self.config.min_token_length,
                self.config.max_token_length,
            );
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
                        self.db_inverted,
                        &mut wtxn,
                        collection,
                        word,
                        id_u64,
                    )?;
                }
                IdType::String => {
                    posting_list::remove_from_posting_list(
                        self.db_inverted,
                        &mut wtxn,
                        collection,
                        word,
                        id,
                    )?;
                }
            }
        }

        self.db_docs.delete(&mut wtxn, doc_key.as_slice())?;
        wtxn.commit()?;

        tracing::debug!(collection = %collection, id = %id, "item deleted");
        Ok(())
    }

    pub fn collection_info(&self, collection: &str) -> Result<CollectionInfo, AppError> {
        let meta = self.validate_collection_exists(collection)?;

        let rtxn = self.env.read_txn()?;
        let prefix = doc_prefix(collection);
        let mut count = 0u64;
        if let Ok(iter) = self.db_docs.prefix_iter(&rtxn, prefix.as_slice()) {
            for _ in iter {
                count += 1;
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
        if self.collections.read().unwrap().is_empty() {
            self.refresh_collections_cache()?;
        }
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
        crate::backup::export_snapshot(&self.env)
    }

    pub fn reset_fst(&self) {
        self.fst_pool.clear_all();
    }

    pub fn import_snapshot(&self, data: &[u8]) -> Result<(), AppError> {
        crate::backup::import_snapshot(&self.env, data)?;
        self.refresh_collections_cache()?;
        self.reset_fst();
        Ok(())
    }

    fn refresh_collections_cache(&self) -> Result<(), AppError> {
        let mut map = std::collections::HashMap::new();
        if let Ok(rtxn) = self.env.read_txn()
            && let Ok(cursor_iter) = self.db_meta.iter(&rtxn)
        {
            for result in cursor_iter.flatten() {
                let (k, v) = result;
                let name = String::from_utf8_lossy(&k).to_string();
                if let Ok(col_meta) = decode_rkyv!(config::CollectionMeta, &v) {
                    map.insert(name, col_meta);
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

        let mut wtxn = self.env.write_txn()?;

        let doc_keys: Vec<Vec<u8>> = self
            .db_docs
            .prefix_iter(&wtxn, prefix.as_slice())
            .unwrap_or_else(|_| panic!("prefix_iter on docs for deletion"))
            .filter_map(|r| r.ok())
            .map(|(k, _)| k.to_vec())
            .collect();
        for key in &doc_keys {
            self.db_docs.delete(&mut wtxn, key.as_slice())?;
        }

        let inv_keys: Vec<Vec<u8>> = self
            .db_inverted
            .prefix_iter(&wtxn, prefix.as_slice())
            .unwrap_or_else(|_| panic!("prefix_iter on inverted for deletion"))
            .filter_map(|r| r.ok())
            .map(|(k, _)| k.to_vec())
            .collect();
        for key in &inv_keys {
            self.db_inverted.delete(&mut wtxn, key.as_slice())?;
        }

        self.db_meta.delete(&mut wtxn, collection.as_bytes())?;
        wtxn.commit()?;

        self.fst_pool.delete_collection(collection);

        tracing::info!(collection = %collection, "collection deleted");
        Ok(())
    }
}

fn doc_key(collection: &str, doc_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(collection.len() + 1 + doc_id.len());
    key.extend_from_slice(collection.as_bytes());
    key.push(0);
    key.extend_from_slice(doc_id.as_bytes());
    if key.len() > 450 {
        tracing::warn!(
            key_len = key.len(),
            collection_len = collection.len(),
            doc_id_len = doc_id.len(),
            doc_id = %doc_id.chars().take(200).collect::<String>(),
            "doc_key exceeds safe LMDB key size"
        );
    }
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
        let env = unsafe {
            heed::EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024)
                .max_dbs(4)
                .open(dir.path())
                .unwrap()
        };
        let store = Store::with_config(env, conf, dir.path().join("fst"));
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
        let tokens = tokenize(
            "hello world",
            config.min_token_length,
            config.max_token_length,
        );
        let mut sorted: Vec<_> = tokens.into_iter().collect();
        sorted.sort();
        assert_eq!(sorted, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_deduplicates() {
        let config = StoreConfig::default();
        let tokens = tokenize(
            "foo foo foo",
            config.min_token_length,
            config.max_token_length,
        );
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("foo"));
    }

    #[test]
    fn tokenize_short_words_filtered() {
        let config = StoreConfig {
            min_token_length: 3,
            ..Default::default()
        };
        let tokens = tokenize(
            "a an the fox",
            config.min_token_length,
            config.max_token_length,
        );
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains("the"));
        assert!(tokens.contains("fox"));
    }

    #[test]
    fn tokenize_empty() {
        let config = StoreConfig::default();
        let tokens = tokenize("", config.min_token_length, config.max_token_length);
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_only_short() {
        let config = StoreConfig {
            min_token_length: 10,
            ..Default::default()
        };
        let tokens = tokenize(
            "hello world",
            config.min_token_length,
            config.max_token_length,
        );
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
        assert_eq!(cfg.max_token_length, 400);
        assert_eq!(cfg.max_string_shard_size, 1000);
        assert_eq!(cfg.max_roaring_shard_size, 100_000);
    }

    #[test]
    fn tokenize_long_words_filtered() {
        let config = StoreConfig {
            max_token_length: 4,
            ..Default::default()
        };
        let tokens = tokenize(
            "hello world",
            config.min_token_length,
            config.max_token_length,
        );
        assert!(!tokens.contains("hello")); // 5 chars, excluded by max_token_length
        assert!(tokens.is_empty());
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
    fn upsert_update_reindex() -> Result<(), AppError> {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string", &["content".into()])?;
        store.upsert("docs", json!({"id": "1", "content": "apple banana"}))?;
        store.upsert("docs", json!({"id": "1", "content": "apple cherry"}))?;
        store.flush()?;
        // banana was in the original content but removed in the second upsert
        // — must no longer be searchable.
        let r1 = store.search("docs", "banana", false, 10, None)?;
        assert!(
            r1.is_empty(),
            "expected banana to be reindexed away: {r1:?}"
        );
        // cherry was added in the second upsert.
        let r2 = store.search("docs", "cherry", false, 10, None)?;
        assert_eq!(ids(&r2), vec!["1"]);
        // apple was present in both versions.
        let r3 = store.search("docs", "apple", false, 10, None)?;
        assert_eq!(ids(&r3), vec!["1"]);
        Ok(())
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

    /// Inject a queue entry directly into LMDB. Used to simulate a stale
    /// entry left by an older binary that didn't reject malformed ids.
    /// Returns AppError so call sites must handle failures explicitly
    /// rather than panicking.
    fn inject_queue_entry(
        store: &Store,
        collection: &str,
        id: &str,
        doc: &serde_json::Value,
    ) -> Result<(), AppError> {
        let document = serde_json::to_vec(doc).map_err(|e| AppError::Internal(e.to_string()))?;
        let entry = config::QueuedIndex {
            collection: collection.to_string(),
            id: id.to_string(),
            document,
        };
        let seq = store.allocate_seq();
        let mut wtxn = store.env.write_txn()?;
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&entry)
            .map(|av| av.to_vec())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        store
            .db_queue
            .put(&mut wtxn, seq.to_be_bytes().as_slice(), bytes.as_slice())?;
        wtxn.commit()?;
        Ok(())
    }

    #[test]
    fn collection_name_validator_accepts_safe_names() -> Result<(), AppError> {
        for name in [
            "a",
            "Z",
            "0",
            "_",
            "-",
            "foo",
            "Foo123",
            "my_collection",
            "my-collection",
            "ABC_123-xyz",
        ] {
            validate_collection_name(name)?;
        }
        Ok(())
    }

    #[test]
    fn collection_name_validator_rejects_unsafe_names() {
        // Each name maps to the same FST file as another via the lossy
        // [^A-Za-z0-9_-] → '_' encoding, OR contains characters that would
        // not be filesystem-safe across platforms.
        let bad_names = [
            "", "foo bar", "foo/bar", "foo\\bar", "foo.bar", "foo:bar", "foo\tbar", "foo\0bar",
            "../foo", ".", "..", "café", "中文",
        ];
        for name in bad_names {
            let err = validate_collection_name(name).expect_err(name);
            assert!(matches!(err, AppError::BadRequest(_)), "name={name}");
        }
    }

    #[test]
    fn collection_name_validator_rejects_too_long() {
        let long: String = "a".repeat(COLLECTION_NAME_MAX_LEN + 1);
        let err = validate_collection_name(&long).expect_err("too long");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn create_collection_rejects_colliding_names() -> Result<(), AppError> {
        // Regression: previously `FSTPool::collection_path` lossy-mapped
        // any non-[A-Za-z0-9_-] char to '_', so 'foo bar' and 'foo/bar'
        // both produced 'foo_bar.fst' and overwrote each other.
        let (store, _dir) = default_store();
        let res = store.create_collection("foo bar", "string", &[]);
        assert!(matches!(res, Err(AppError::BadRequest(_))));
        let res = store.create_collection("foo/bar", "string", &[]);
        assert!(matches!(res, Err(AppError::BadRequest(_))));
        // Safe name still works.
        store.create_collection("foo_bar", "string", &[])?;
        Ok(())
    }

    #[test]
    fn process_pending_queue_survives_restart_with_unwarmed_cache() -> Result<(), AppError> {
        // Regression: previously `process_pending_queue` only consulted the
        // in-memory collections cache. After a process restart (or any code
        // path where the background indexer fires before any user request),
        // the cache was empty and every queued item was logged as "unknown
        // collection" and DELETED from the queue — silent data loss.
        let dir =
            tempfile::TempDir::new().map_err(|e| AppError::Internal(format!("tempdir: {e}")))?;

        // First "process": create collection, queue an item, but never flush.
        {
            let env = unsafe {
                heed::EnvOpenOptions::new()
                    .map_size(10 * 1024 * 1024)
                    .max_dbs(4)
                    .open(dir.path())?
            };
            let store = Store::new(env, dir.path().join("fst"));
            store.create_collection("docs", "string", &["content".into()])?;
            store.upsert("docs", json!({"id": "1", "content": "important data"}))?;
            // No flush — the entry must persist in db_queue across restart.
            assert!(store.queue_depth()? > 0);
        }

        // Second "process": reopen the same env. The collections cache
        // starts empty until warming runs; the background indexer (here,
        // our flush call) must still find the collection and index the
        // queued item.
        {
            let env = unsafe {
                heed::EnvOpenOptions::new()
                    .map_size(10 * 1024 * 1024)
                    .max_dbs(4)
                    .open(dir.path())?
            };
            let store = Store::new(env, dir.path().join("fst"));
            // Drive the indexer without calling validate_collection_exists
            // (which would otherwise warm the cache as a side effect).
            store.flush()?;
            // The queue must be drained AND the document must be searchable.
            assert_eq!(store.queue_depth()?, 0, "queue not drained");
            let results = store.search("docs", "important", false, 10, None)?;
            assert_eq!(ids(&results), vec!["1"]);
        }
        Ok(())
    }

    #[test]
    fn process_pending_queue_drops_bad_numeric_id_instead_of_stalling() -> Result<(), AppError> {
        // Regression test: previously a single queue entry with a non-numeric
        // id in a number-id collection (e.g., one written by an older binary
        // before float-id rejection) caused process_pending_queue to abort
        // its write txn with `?` propagation; the bad entry stayed in the
        // queue and every subsequent indexing cycle hit it again — permanent
        // ingest stall.
        let (store, _dir) = default_store();
        store.create_collection("docs", "number", &["content".into()])?;

        // Inject the bad entry directly into LMDB, then a good one via the
        // public API queued behind it.
        inject_queue_entry(
            &store,
            "docs",
            "not-a-number",
            &json!({"id": "not-a-number", "content": "should be dropped"}),
        )?;
        store.upsert("docs", json!({"id": 42, "content": "should survive"}))?;

        // One process call must clear both: bad one dropped, good one indexed.
        store.flush()?;

        // Queue must be drained.
        assert_eq!(store.queue_depth()?, 0, "queue still has entries");

        // The good document is searchable.
        let results = store.search("docs", "survive", false, 10, None)?;
        assert_eq!(ids(&results), vec!["42"]);
        Ok(())
    }
}
