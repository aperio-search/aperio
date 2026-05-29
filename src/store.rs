use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock};

use charabia::Tokenize;
use fjall::Slice;
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use roaring::{MultiOps, RoaringTreemap};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::{
    CollectionCreated, CollectionInfo, CollectionSummary, ListCollectionsResponse,
};

const SHARD_DELIM: char = '\0';

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

fn roaring_to_vec(b: &RoaringTreemap) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::with_capacity(b.serialized_size());
    b.serialize_into(&mut buf)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(buf)
}

fn roaring_from_slice(bytes: &[u8]) -> Result<RoaringTreemap, AppError> {
    RoaringTreemap::deserialize_from(bytes)
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Clone)]
pub struct StoreConfig {
    pub min_token_length: usize,
    pub max_shard_size: usize,
    pub max_roaring_shard_size: u64,
    pub write_buffer_size: Option<u64>,
    pub compression: Option<fjall::CompressionType>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            min_token_length: 2,
            max_shard_size: 1000,
            max_roaring_shard_size: 100_000,
            write_buffer_size: None,
            compression: None,
        }
    }
}

impl StoreConfig {
    fn keyspace_opts(&self) -> fjall::KeyspaceCreateOptions {
        let mut opts = fjall::KeyspaceCreateOptions::default();
        if let Some(size) = self.write_buffer_size {
            opts = opts.max_memtable_size(size);
        }
        if let Some(comp) = self.compression {
            opts = opts.data_block_compression_policy(fjall::config::CompressionPolicy::all(comp));
        }
        opts
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum IdType {
    Number,
    String,
}

#[derive(Serialize, Deserialize, Clone, Archive, RkyvSerialize, RkyvDeserialize)]
struct PostingShard {
    first: String,
    last: String,
    ids: Vec<String>,
}

pub struct Store {
    db: fjall::Database,
    config: StoreConfig,
    lock: Mutex<()>,
    collections: RwLock<HashMap<String, IdType>>,
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
                        if let Ok(id_type) = decode_rkyv!(IdType, &value) {
                            map.insert(name, id_type);
                        }
                    }
                }
            }
            map
        };
        Self {
            db,
            config,
            lock: Mutex::new(()),
            collections: RwLock::new(collections),
        }
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

    fn tokenize(content: &str, config: &StoreConfig) -> HashSet<String> {
        content
            .tokenize()
            .filter(|t| t.is_word())
            .map(|t| t.lemma().to_string())
            .filter(|w| w.len() >= config.min_token_length)
            .collect()
    }

    fn shard_key(word: &str, shard: usize) -> Vec<u8> {
        format!("{}{}{:04}", word, SHARD_DELIM, shard).into_bytes()
    }

    fn load_posting_shard(
        inverted: &fjall::Keyspace,
        word: &str,
        shard: usize,
    ) -> Result<Option<PostingShard>, AppError> {
        let key = Self::shard_key(word, shard);
        match inverted.get(&key)? {
            Some(data) => {
                let shard: PostingShard =
                    decode_rkyv!(PostingShard, &data).unwrap_or_else(|_| PostingShard {
                        first: String::new(),
                        last: String::new(),
                        ids: Vec::new(),
                    });
                Ok(Some(shard))
            }
            None => Ok(None),
        }
    }

    fn list_shard_indices(inverted: &fjall::Keyspace, word: &str) -> Result<Vec<usize>, AppError> {
        let prefix = format!("{}{}", word, SHARD_DELIM).into_bytes();
        let mut indices: Vec<usize> = Vec::new();
        for guard in inverted.prefix(&prefix) {
            let (key, _) = guard.into_inner()?;
            if let Some(null_pos) = key.iter().rposition(|&b| b == SHARD_DELIM as u8) {
                let digit_bytes = &key[null_pos + 1..];
                let mut idx = 0;
                for &b in digit_bytes {
                    idx = idx * 10 + (b - b'0') as usize;
                }
                indices.push(idx);
            }
        }
        indices.sort_unstable();
        Ok(indices)
    }

    fn find_shard_for_id(
        inverted: &fjall::Keyspace,
        word: &str,
        id: &str,
        indices: &[usize],
    ) -> Result<usize, AppError> {
        let first_shard =
            Self::load_posting_shard(inverted, word, indices[0])?.unwrap_or_else(|| PostingShard {
                first: String::new(),
                last: String::new(),
                ids: Vec::new(),
            });

        if *id < *first_shard.first {
            return Ok(indices[0]);
        }

        let mut lo = 0usize;
        let mut hi = indices.len().saturating_sub(1);
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let shard = match Self::load_posting_shard(inverted, word, indices[mid])? {
                Some(s) => s,
                None => {
                    lo = mid + 1;
                    continue;
                }
            };
            if *id < *shard.first {
                if mid == 0 {
                    return Ok(indices[0]);
                }
                hi = mid - 1;
            } else if *id > *shard.last {
                lo = mid + 1;
            } else {
                return Ok(indices[mid]);
            }
        }
        Ok(indices[lo.min(indices.len().saturating_sub(1))])
    }

    fn add_to_posting_list(
        inverted: &fjall::Keyspace,
        word: &str,
        id: &str,
        max_shard_size: usize,
    ) -> Result<(), AppError> {
        let marker_key = word.as_bytes();
        if inverted.get(marker_key)?.is_none() {
            inverted.insert(marker_key, &[])?;
        }

        let indices = Self::list_shard_indices(inverted, word)?;

        if indices.is_empty() {
            let shard = PostingShard {
                first: id.to_string(),
                last: id.to_string(),
                ids: vec![id.to_string()],
            };
            let value = encode_rkyv!(&shard)?;
            inverted.insert(Self::shard_key(word, 0), &value)?;
            return Ok(());
        }

        let last_idx = *indices.last().unwrap();
        let last_shard =
            Self::load_posting_shard(inverted, word, last_idx)?.unwrap_or_else(|| PostingShard {
                first: String::new(),
                last: String::new(),
                ids: Vec::new(),
            });

        if *id > *last_shard.last {
            if last_shard.ids.len() < max_shard_size {
                if last_shard.ids.binary_search(&id.to_string()).is_ok() {
                    return Ok(());
                }
                let mut new_shard = last_shard;
                new_shard.ids.push(id.to_string());
                new_shard.last = id.to_string();
                let new_value = encode_rkyv!(&new_shard)?;
                inverted.insert(Self::shard_key(word, last_idx), &new_value)?;
            } else {
                let shard = PostingShard {
                    first: id.to_string(),
                    last: id.to_string(),
                    ids: vec![id.to_string()],
                };
                let value = encode_rkyv!(&shard)?;
                inverted.insert(Self::shard_key(word, last_idx + 1), &value)?;
            }
            return Ok(());
        }

        let target = Self::find_shard_for_id(inverted, word, id, &indices)?;
        let current =
            Self::load_posting_shard(inverted, word, target)?.unwrap_or_else(|| PostingShard {
                first: String::new(),
                last: String::new(),
                ids: Vec::new(),
            });

        if current.ids.binary_search(&id.to_string()).is_ok() {
            return Ok(());
        }

        let pos = current.ids.binary_search(&id.to_string()).unwrap_err();
        let mut new_shard = current;
        new_shard.ids.insert(pos, id.to_string());
        if pos == 0 {
            new_shard.first = id.to_string();
        }
        if pos == new_shard.ids.len() - 1 {
            new_shard.last = id.to_string();
        }

        let new_value = encode_rkyv!(&new_shard)?;
        inverted.insert(Self::shard_key(word, target), &new_value)?;

        Ok(())
    }

    fn remove_from_posting_list(
        inverted: &fjall::Keyspace,
        word: &str,
        id: &str,
    ) -> Result<(), AppError> {
        let indices = Self::list_shard_indices(inverted, word)?;
        if indices.is_empty() {
            return Ok(());
        }

        let target = Self::find_shard_for_id(inverted, word, id, &indices)?;
        let current = match Self::load_posting_shard(inverted, word, target)? {
            Some(s) => s,
            None => return Ok(()),
        };

        let pos = match current.ids.binary_search(&id.to_string()) {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };

        let ran_first = pos == 0;
        let ran_last = pos == current.ids.len() - 1;
        let mut new_shard = current;
        new_shard.ids.remove(pos);

        let key = Self::shard_key(word, target);
        if new_shard.ids.is_empty() {
            inverted.remove(&key)?;
        } else {
            if ran_first {
                new_shard.first = new_shard.ids[0].clone();
            }
            if ran_last {
                new_shard.last = new_shard.ids.last().unwrap().clone();
            }
            let new_value = encode_rkyv!(&new_shard)?;
            inverted.insert(&key, &new_value)?;
        }

        Ok(())
    }

    fn add_to_roaring_posting_list(
        inverted: &fjall::Keyspace,
        word: &str,
        id: u64,
        max_roaring_shard_size: u64,
    ) -> Result<(), AppError> {
        let marker_key = word.as_bytes();
        if inverted.get(marker_key)?.is_none() {
            inverted.insert(marker_key, &[])?;
        }

        let indices = Self::list_shard_indices(inverted, word)?;

        if indices.is_empty() {
            let mut bitmap = RoaringTreemap::new();
            bitmap.insert(id);
            let value = roaring_to_vec(&bitmap)?;
            inverted.insert(Self::shard_key(word, 0), &value)?;
            return Ok(());
        }

        let last_idx = *indices.last().unwrap();
        let last_key = Self::shard_key(word, last_idx);
        let mut bitmap: RoaringTreemap = match inverted.get(&last_key)? {
            Some(data) => roaring_from_slice(&data)?,
            None => RoaringTreemap::new(),
        };

        if bitmap.len() < max_roaring_shard_size {
            bitmap.insert(id);
            let value = roaring_to_vec(&bitmap)?;
            inverted.insert(&last_key, &value)?;
        } else {
            let mut new_bitmap = RoaringTreemap::new();
            new_bitmap.insert(id);
            let value = roaring_to_vec(&new_bitmap)?;
            inverted.insert(Self::shard_key(word, last_idx + 1), &value)?;
        }

        Ok(())
    }

    fn remove_from_roaring_posting_list(
        inverted: &fjall::Keyspace,
        word: &str,
        id: u64,
    ) -> Result<(), AppError> {
        let indices = Self::list_shard_indices(inverted, word)?;
        if indices.is_empty() {
            return Ok(());
        }

        for &shard_idx in &indices {
            let key = Self::shard_key(word, shard_idx);
            let mut bitmap: RoaringTreemap = match inverted.get(&key)? {
                Some(data) => roaring_from_slice(&data)?,
                None => continue,
            };
            if !bitmap.contains(id) {
                continue;
            }
            bitmap.remove(id);
            if bitmap.is_empty() {
                inverted.remove(&key)?;
            } else {
                let value = roaring_to_vec(&bitmap)?;
                inverted.insert(&key, &value)?;
            }
            return Ok(());
        }

        Ok(())
    }

    fn roaring_search(
        &self,
        collection: &str,
        query: &str,
        sort_desc: bool,
        take: usize,
        after: Option<&str>,
    ) -> Result<Vec<String>, AppError> {
        let inverted = self.inverted_keyspace(collection)?;

        let tokens: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let mut word_shards: Vec<(String, Vec<usize>)> = std::thread::scope(|s| {
            let handles: Vec<_> = tokens.iter().map(|w| {
                let inv = inverted.clone();
                let w = w.clone();
                s.spawn(move || {
                    let indices = Self::list_shard_indices(&inv, &w).unwrap_or_default();
                    (w, indices)
                })
            }).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        word_shards.sort_by(|a, b| a.1.len().cmp(&b.1.len()));
        if word_shards.first().map_or(true, |(_, idx)| idx.is_empty()) {
            return Ok(Vec::new());
        }

        let word_bitmaps: Vec<RoaringTreemap> = std::thread::scope(|s| {
            let handles: Vec<_> = word_shards.iter().map(|(word, indices)| {
                let inv = inverted.clone();
                let word = word.clone();
                let indices = indices.clone();
                s.spawn(move || -> Result<RoaringTreemap, AppError> {
                    let mut word_bitmap = RoaringTreemap::new();
                    for &shard_idx in &indices {
                        let key = Self::shard_key(&word, shard_idx);
                        if let Some(data) = inv.get(&key)? {
                            if let Ok(bitmap) = roaring_from_slice(&data) {
                                word_bitmap |= &bitmap;
                            }
                        }
                    }
                    Ok(word_bitmap)
                })
            }).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect::<Result<Vec<_>, _>>()
        })?;

        let bitmap = word_bitmaps.iter().intersection();

        let after_val = after.and_then(|a| a.parse::<u64>().ok());
        let iter: Box<dyn Iterator<Item = u64>> = if sort_desc {
            Box::new(bitmap.into_iter().rev())
        } else {
            Box::new(bitmap.into_iter())
        };

        let mut results: Vec<String> = Vec::with_capacity(take);
        for id in iter {
            if let Some(cursor) = after_val {
                if sort_desc && id >= cursor {
                    continue;
                }
                if !sort_desc && id <= cursor {
                    continue;
                }
            }
            results.push(id.to_string());
            if results.len() >= take {
                break;
            }
        }

        tracing::debug!(collection = %collection, query = %query, results = results.len(), "roaring search completed");
        Ok(results)
    }

    pub fn upsert(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
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

        let new_words = Self::tokenize(content, &self.config);

        if let Some(old_data) = docs.get(id.as_bytes())? {
            let old_tokens: Vec<String> = decode_rkyv!(Vec<String>, &old_data)?;
            let old_words: HashSet<String> = old_tokens.into_iter().collect();
            for word in old_words.difference(&new_words) {
                match id_type {
                    IdType::Number => {
                        Self::remove_from_roaring_posting_list(&inverted, word, id_u64.unwrap())?
                    }
                    IdType::String => Self::remove_from_posting_list(&inverted, word, id)?,
                }
            }
        }

        for word in &new_words {
            match id_type {
                IdType::Number => Self::add_to_roaring_posting_list(
                    &inverted,
                    word,
                    id_u64.unwrap(),
                    self.config.max_roaring_shard_size,
                )?,
                IdType::String => {
                    Self::add_to_posting_list(&inverted, word, id, self.config.max_shard_size)?
                }
            }
        }

        let tokens: Vec<String> = new_words.into_iter().collect();
        docs.insert(id.as_bytes(), encode_rkyv!(&tokens)?)?;
        tracing::debug!(collection = %collection, id = %id, tokens = tokens.len(), "item upserted");
        Ok(())
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
        match id_type {
            IdType::Number => {
                return self.roaring_search(collection, query, sort_desc, take, after);
            }
            IdType::String => {}
        }
        let inverted = self.inverted_keyspace(collection)?;

        let tokens: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let mut word_shards: Vec<(String, Vec<usize>)> = std::thread::scope(|s| {
            let handles: Vec<_> = tokens.iter().map(|w| {
                let inv = inverted.clone();
                let w = w.clone();
                s.spawn(move || {
                    let indices = Self::list_shard_indices(&inv, &w).unwrap_or_default();
                    (w, indices)
                })
            }).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        word_shards.sort_by(|a, b| a.1.len().cmp(&b.1.len()));
        if word_shards.first().map_or(true, |(_, idx)| idx.is_empty()) {
            return Ok(Vec::new());
        }

        let mut iters: Vec<WordIterState> = std::thread::scope(|s| {
            let handles: Vec<_> = word_shards.iter().map(|(word, indices)| {
                let inv = inverted.clone();
                let word = word.clone();
                let indices = indices.clone();
                s.spawn(move || -> Result<WordIterState, AppError> {
                    let mut state = WordIterState::new(indices, sort_desc);
                    if !sort_desc {
                        Self::load_first_shard(&inv, &word, &mut state)?;
                    } else {
                        Self::load_last_shard(&inv, &word, &mut state)?;
                    }
                    Ok(state)
                })
            }).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect::<Result<Vec<_>, _>>()
        })?;

        if let Some(cursor) = after {
            for (i, (word, _)) in word_shards.iter().enumerate() {
                Self::skip_past_cursor(&inverted, word, cursor, &mut iters[i], sort_desc)?;
            }
        }

        let mut results: Vec<String> = Vec::with_capacity(take);

        loop {
            let mut pivot: Option<String> = None;
            let mut any_exhausted = false;

            for state in &iters {
                match state.current() {
                    None => {
                        any_exhausted = true;
                    }
                    Some(id) => match &pivot {
                        None => {
                            pivot = Some(id.to_string());
                        }
                        Some(p) => {
                            let take_this = if sort_desc {
                                id < p.as_str()
                            } else {
                                id > p.as_str()
                            };
                            if take_this {
                                pivot = Some(id.to_string());
                            }
                        }
                    },
                }
            }

            if pivot.is_none() || any_exhausted {
                break;
            }

            let pivot = pivot.unwrap();
            let mut all_have = true;

            for (i, state) in iters.iter_mut().enumerate() {
                Self::seek_to(&inverted, &word_shards[i].0, state, &pivot, sort_desc)?;
                match state.current() {
                    None => {
                        all_have = false;
                        break;
                    }
                    Some(id) => {
                        if id != pivot.as_str() {
                            all_have = false;
                            break;
                        }
                    }
                }
            }

            if all_have {
                results.push(pivot);
                if results.len() >= take {
                    break;
                }
                for (i, state) in iters.iter_mut().enumerate() {
                    Self::advance_iter(&inverted, &word_shards[i].0, state, sort_desc)?;
                }
            }
        }

        tracing::debug!(collection = %collection, query = %query, results = results.len(), "string search completed");
        Ok(results)
    }

    fn load_first_shard(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
    ) -> Result<(), AppError> {
        while (state.shard_pos as usize) < state.indices.len() {
            let idx = state.indices[state.shard_pos as usize];
            let key = Self::shard_key(word, idx);
            match inverted.get(&key)? {
                Some(data) => {
                    let non_empty = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                        .map(|a| !a.ids.is_empty())
                        .unwrap_or(false);
                    if non_empty {
                        state.cur_slice = Some(data);
                        state.cur_pos = 0;
                        return Ok(());
                    }
                }
                None => {}
            }
            state.shard_pos += 1;
        }
        state.cur_slice = None;
        state.cur_pos = 0;
        Ok(())
    }

    fn load_last_shard(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
    ) -> Result<(), AppError> {
        while state.shard_pos >= 0 {
            let idx = state.indices[state.shard_pos as usize];
            let key = Self::shard_key(word, idx);
            match inverted.get(&key)? {
                Some(data) => {
                    let maybe_len = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                        .map(|a| a.ids.len())
                        .unwrap_or(0);
                    if maybe_len > 0 {
                        state.cur_slice = Some(data);
                        state.cur_pos = maybe_len.saturating_sub(1);
                        return Ok(());
                    }
                }
                None => {}
            }
            state.shard_pos -= 1;
        }
        state.cur_slice = None;
        state.cur_pos = 0;
        Ok(())
    }

    fn advance_shard(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
        desc: bool,
    ) -> Result<(), AppError> {
        loop {
            if desc {
                state.shard_pos -= 1;
                if state.shard_pos < 0 {
                    state.cur_slice = None;
                    state.cur_pos = 0;
                    return Ok(());
                }
            } else {
                state.shard_pos += 1;
                if (state.shard_pos as usize) >= state.indices.len() {
                    state.cur_slice = None;
                    state.cur_pos = 0;
                    return Ok(());
                }
            }
            let idx = state.indices[state.shard_pos as usize];
            let key = Self::shard_key(word, idx);
            match inverted.get(&key)? {
                Some(data) => {
                    let maybe_len = rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(&data)
                        .map(|a| a.ids.len())
                        .unwrap_or(0);
                    if maybe_len > 0 {
                        state.cur_slice = Some(data);
                        state.cur_pos = if desc {
                            maybe_len.saturating_sub(1)
                        } else {
                            0
                        };
                        return Ok(());
                    }
                }
                None => {}
            }
        }
    }

    fn advance_iter(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
        desc: bool,
    ) -> Result<(), AppError> {
        if desc {
            if state.cur_pos == 0 {
                return Self::advance_shard(inverted, word, state, desc);
            }
            state.cur_pos -= 1;
        } else {
            state.cur_pos += 1;
            if state.cur_pos >= state.ids_len() {
                return Self::advance_shard(inverted, word, state, desc);
            }
        }
        Ok(())
    }

    fn seek_to(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
        target: &str,
        desc: bool,
    ) -> Result<(), AppError> {
        loop {
            match state.current() {
                None => return Ok(()),
                Some(id) => {
                    let at_or_past = if desc { id <= target } else { id >= target };
                    if at_or_past {
                        return Ok(());
                    }
                }
            }

            if let Some(archived) = state.current_archived() {
                let ids = &archived.ids;
                let len = ids.len();
                if desc {
                    let mut lo = 0usize;
                    let mut hi = len;
                    while lo < hi {
                        let mid = (lo + hi) / 2;
                        match ids.get(mid) {
                            Some(s) if s.as_str() <= target => lo = mid + 1,
                            _ => hi = mid,
                        }
                    }
                    if lo > 0 {
                        state.cur_pos = lo - 1;
                        return Ok(());
                    }
                } else {
                    let mut lo = 0usize;
                    let mut hi = len;
                    while lo < hi {
                        let mid = (lo + hi) / 2;
                        match ids.get(mid) {
                            Some(s) if s.as_str() < target => lo = mid + 1,
                            _ => hi = mid,
                        }
                    }
                    if lo < len {
                        state.cur_pos = lo;
                        return Ok(());
                    }
                }
            }

            Self::advance_shard(inverted, word, state, desc)?;
        }
    }

    fn skip_past_cursor(
        inverted: &fjall::Keyspace,
        word: &str,
        cursor: &str,
        state: &mut WordIterState,
        desc: bool,
    ) -> Result<(), AppError> {
        if state.indices.is_empty() {
            return Ok(());
        }

        let shard_idx = Self::find_shard_for_id(inverted, word, cursor, &state.indices)?;
        let pos_in_indices = state
            .indices
            .iter()
            .position(|&i| i == shard_idx)
            .unwrap_or(state.indices.len().saturating_sub(1));
        state.shard_pos = pos_in_indices as isize;

        let key = Self::shard_key(word, shard_idx);
        match inverted.get(&key)? {
            Some(data) => {
                let (pos, ids_len) = match rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(
                    &data,
                ) {
                    Ok(archived) => {
                        let ids = &archived.ids;
                        let mut lo = 0;
                        let mut hi = ids.len();
                        while lo < hi {
                            let mid = (lo + hi) / 2;
                            match ids.get(mid) {
                                Some(s) if s.as_str() < cursor => {
                                    lo = mid + 1;
                                }
                                _ => {
                                    hi = mid;
                                }
                            }
                        }
                        (lo, ids.len())
                    }
                    Err(_) => {
                        state.cur_slice = None;
                        state.cur_pos = 0;
                        return Ok(());
                    }
                };
                state.cur_slice = Some(data);
                state.cur_pos = pos.min(ids_len.saturating_sub(1));
            }
            None => {
                state.cur_slice = None;
                state.cur_pos = 0;
                return Ok(());
            }
        }

        loop {
            let cur = state.current();
            match cur {
                None => break,
                Some(id) => {
                    let should_skip = if desc { id >= cursor } else { id <= cursor };
                    if should_skip {
                        Self::advance_iter(inverted, word, state, desc)?;
                    } else {
                        break;
                    }
                }
            }
        }

        Ok(())
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
                IdType::Number => {
                    Self::remove_from_roaring_posting_list(&inverted, word, id_u64.unwrap())?
                }
                IdType::String => Self::remove_from_posting_list(&inverted, word, id)?,
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

struct WordIterState {
    indices: Vec<usize>,
    shard_pos: isize,
    cur_slice: Option<Slice>,
    cur_pos: usize,
}

impl WordIterState {
    fn new(indices: Vec<usize>, desc: bool) -> Self {
        let shard_pos = if desc {
            indices.len().saturating_sub(1) as isize
        } else {
            0
        };
        Self {
            indices,
            shard_pos,
            cur_slice: None,
            cur_pos: 0,
        }
    }

    fn current_archived(&self) -> Option<&ArchivedPostingShard> {
        rkyv::access::<ArchivedPostingShard, rkyv::rancor::Error>(self.cur_slice.as_deref()?).ok()
    }

    fn current(&self) -> Option<&str> {
        let archived = self.current_archived()?;
        if self.cur_pos >= archived.ids.len() {
            return None;
        }
        archived.ids.get(self.cur_pos).map(|s| s.as_str())
    }

    fn ids_len(&self) -> usize {
        self.current_archived()
            .map(|a| a.ids.len())
            .unwrap_or(0)
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
        let tokens = Store::tokenize("hello world", &config);
        let mut sorted: Vec<_> = tokens.into_iter().collect();
        sorted.sort();
        assert_eq!(sorted, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_deduplicates() {
        let config = StoreConfig::default();
        let tokens = Store::tokenize("foo foo foo", &config);
        assert_eq!(tokens.len(), 1);
        assert!(tokens.contains("foo"));
    }

    #[test]
    fn tokenize_short_words_filtered() {
        let config = StoreConfig { min_token_length: 3, ..Default::default() };
        let tokens = Store::tokenize("a an the fox", &config);
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains("the"));
        assert!(tokens.contains("fox"));
    }

    #[test]
    fn tokenize_empty() {
        let config = StoreConfig::default();
        let tokens = Store::tokenize("", &config);
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_only_short() {
        let config = StoreConfig { min_token_length: 10, ..Default::default() };
        let tokens = Store::tokenize("hello world", &config);
        assert!(tokens.is_empty());
    }

    #[test]
    fn shard_key_format() {
        let key = Store::shard_key("hello", 42);
        assert_eq!(key, b"hello\x000042");
    }

    #[test]
    fn shard_key_zero_padded() {
        let key = Store::shard_key("test", 0);
        assert_eq!(key, b"test\x000000");
        let key = Store::shard_key("test", 9999);
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
        // both "apple" and "banana" -> only doc 1
        let results = store.search("docs", "apple banana", false, 10, None).unwrap();
        assert_eq!(results, vec!["1"]);
    }

    #[test]
    fn search_no_match() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "1", "hello world").unwrap();
        let results = store.search("docs", "nonexistent", false, 10, None).unwrap();
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
        // asc: a then b
        assert_eq!(results, vec!["a", "b"]);
    }

    #[test]
    fn search_sort_desc_default() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        let results = store.search("docs", "hello", true, 10, None).unwrap();
        // desc: b then a
        assert_eq!(results, vec!["b", "a"]);
    }

    #[test]
    fn search_pagination_after() {
        let (store, _dir) = default_store();
        store.create_collection("docs", "string").unwrap();
        store.upsert("docs", "a", "hello").unwrap();
        store.upsert("docs", "b", "hello").unwrap();
        store.upsert("docs", "c", "hello").unwrap();
        // After "a" -> should skip "a", get "b", "c"
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
        // "banana" removed from index
        let r1 = store.search("docs", "banana", false, 10, None).unwrap();
        assert!(r1.is_empty());
        // "cherry" now indexed
        let r2 = store.search("docs", "cherry", false, 10, None).unwrap();
        assert_eq!(r2, vec!["1"]);
        // "apple" still present
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
        let err = store.search("nonexistent", "hello", false, 10, None).unwrap_err();
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
        store.upsert("docs", "1", "hello").unwrap(); // same content, no-op
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
        let conf = StoreConfig { max_shard_size: 3, ..Default::default() };
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
        let conf = StoreConfig { max_roaring_shard_size: 3, ..Default::default() };
        let (store, _dir) = test_store(conf);
        store.create_collection("docs", "number").unwrap();
        for i in 0..10u64 {
            store.upsert("docs", &i.to_string(), "hello").unwrap();
        }
        let results = store.search("docs", "hello", false, 20, None).unwrap();
        assert_eq!(results.len(), 10);
    }
}
