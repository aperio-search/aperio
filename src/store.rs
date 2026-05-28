use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock};

use roaring::RoaringTreemap;
use serde::{Deserialize, Serialize};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::error::AppError;
use crate::models::{CollectionCreated, CollectionInfo};

const MAX_SHARD_SIZE: usize = 1000;
const MAX_ROARING_SHARD_SIZE: u64 = 100_000;
const SHARD_DELIM: char = '\0';

#[derive(Clone)]
pub struct StoreConfig {
    pub min_token_length: usize,
    pub strip_punctuation: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            min_token_length: 2,
            strip_punctuation: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdType {
    Number,
    String,
}

#[derive(Serialize, Deserialize, Clone)]
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
            if let Ok(meta) = db.keyspace("_collections", fjall::KeyspaceCreateOptions::default) {
                for guard in meta.iter() {
                    if let Ok((key, value)) = guard.into_inner() {
                        let name = String::from_utf8_lossy(&key).to_string();
                        if let Ok((id_type, _)) =
                            bincode::serde::decode_from_slice(&value, bincode::config::standard())
                        {
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
        Ok(self.db.keyspace(&name, fjall::KeyspaceCreateOptions::default)?)
    }

    fn docs_keyspace(&self, collection: &str) -> Result<fjall::Keyspace, AppError> {
        let name = format!("{}.docs", collection);
        Ok(self.db.keyspace(&name, fjall::KeyspaceCreateOptions::default)?)
    }

    fn meta_keyspace(&self) -> Result<fjall::Keyspace, AppError> {
        Ok(self.db.keyspace("_collections", fjall::KeyspaceCreateOptions::default)?)
    }

    fn validate_collection_exists(&self, collection: &str) -> Result<IdType, AppError> {
        self.collections
            .read()
            .unwrap()
            .get(collection)
            .copied()
            .ok_or_else(|| AppError::NotFound(format!("collection '{}' not found", collection)))
    }

    pub fn create_collection(&self, name: &str, id_type: &str) -> Result<CollectionCreated, AppError> {
        let id_type_enum = match id_type {
            "number" => IdType::Number,
            "string" => IdType::String,
            _ => {
                return Err(AppError::BadRequest(format!(
                    "invalid id_type '{}', expected 'number' or 'string'",
                    id_type
                )))
            }
        };

        let _lock = self.lock.lock().unwrap();

        {
            let mut map = self.collections.write().unwrap();
            if map.contains_key(name) {
                return Err(AppError::BadRequest(format!("collection '{}' already exists", name)));
            }

            let meta = self.meta_keyspace()?;
            let value = bincode::serde::encode_to_vec(&id_type_enum, bincode::config::standard())?;
            meta.insert(name.as_bytes(), &value)?;

            map.insert(name.to_string(), id_type_enum);
        }

        self.db.persist(fjall::PersistMode::SyncData)?;

        Ok(CollectionCreated {
            name: name.to_string(),
            id_type: id_type.to_string(),
        })
    }

    fn normalize(word: &str, strip_punctuation: bool) -> String {
        word.nfkd()
            .filter(|c| !is_combining_mark(*c))
            .collect::<String>()
            .to_lowercase()
            .chars()
            .filter(|c| !strip_punctuation || c.is_alphanumeric())
            .collect()
    }

    fn tokenize(content: &str, config: &StoreConfig) -> HashSet<String> {
        content
            .split_whitespace()
            .filter(|w| !w.is_empty())
            .map(|w| Self::normalize(w, config.strip_punctuation))
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
                    bincode::serde::decode_from_slice(&data, bincode::config::standard())
                        .map(|(v, _)| v)
                        .unwrap_or_else(|_| PostingShard {
                            first: String::new(),
                            last: String::new(),
                            ids: Vec::new(),
                        });
                Ok(Some(shard))
            }
            None => Ok(None),
        }
    }

    fn list_shard_indices(
        inverted: &fjall::Keyspace,
        word: &str,
    ) -> Result<Vec<usize>, AppError> {
        let prefix = format!("{}{}", word, SHARD_DELIM).into_bytes();
        let mut indices: Vec<usize> = Vec::new();
        for guard in inverted.prefix(&prefix) {
            let (key, _) = guard.into_inner()?;
            let key_str = String::from_utf8_lossy(&key);
            if let Some(idx_str) = key_str.rsplit(SHARD_DELIM).next() {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    indices.push(idx);
                }
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
        let first_shard = Self::load_posting_shard(inverted, word, indices[0])?
            .unwrap_or_else(|| PostingShard {
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
            let value = bincode::serde::encode_to_vec(&shard, bincode::config::standard())?;
            inverted.insert(Self::shard_key(word, 0), &value)?;
            return Ok(());
        }

        let last_idx = *indices.last().unwrap();
        let last_shard = Self::load_posting_shard(inverted, word, last_idx)?.unwrap_or_else(
            || PostingShard {
                first: String::new(),
                last: String::new(),
                ids: Vec::new(),
            },
        );

        if *id > *last_shard.last {
            if last_shard.ids.len() < MAX_SHARD_SIZE {
                if last_shard.ids.binary_search(&id.to_string()).is_ok() {
                    return Ok(());
                }
                let mut new_shard = last_shard;
                new_shard.ids.push(id.to_string());
                new_shard.last = id.to_string();
                let new_value =
                    bincode::serde::encode_to_vec(&new_shard, bincode::config::standard())?;
                inverted.insert(Self::shard_key(word, last_idx), &new_value)?;
            } else {
                let shard = PostingShard {
                    first: id.to_string(),
                    last: id.to_string(),
                    ids: vec![id.to_string()],
                };
                let value = bincode::serde::encode_to_vec(&shard, bincode::config::standard())?;
                inverted.insert(Self::shard_key(word, last_idx + 1), &value)?;
            }
            return Ok(());
        }

        let target = Self::find_shard_for_id(inverted, word, id, &indices)?;
        let current = Self::load_posting_shard(inverted, word, target)?.unwrap_or_else(
            || PostingShard {
                first: String::new(),
                last: String::new(),
                ids: Vec::new(),
            },
        );

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

        let new_value = bincode::serde::encode_to_vec(&new_shard, bincode::config::standard())?;
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
            let new_value =
                bincode::serde::encode_to_vec(&new_shard, bincode::config::standard())?;
            inverted.insert(&key, &new_value)?;
        }

        Ok(())
    }

    fn add_to_roaring_posting_list(
        inverted: &fjall::Keyspace,
        word: &str,
        id: u64,
    ) -> Result<(), AppError> {
        let marker_key = word.as_bytes();
        if inverted.get(marker_key)?.is_none() {
            inverted.insert(marker_key, &[])?;
        }

        let indices = Self::list_shard_indices(inverted, word)?;

        if indices.is_empty() {
            let mut bitmap = RoaringTreemap::new();
            bitmap.insert(id);
            let value = bincode::serde::encode_to_vec(&bitmap, bincode::config::standard())?;
            inverted.insert(Self::shard_key(word, 0), &value)?;
            return Ok(());
        }

        let last_idx = *indices.last().unwrap();
        let last_key = Self::shard_key(word, last_idx);
        let mut bitmap: RoaringTreemap = match inverted.get(&last_key)? {
            Some(data) => {
                bincode::serde::decode_from_slice(&data, bincode::config::standard())
                    .map(|(v, _)| v)?
            }
            None => RoaringTreemap::new(),
        };

        if bitmap.len() < MAX_ROARING_SHARD_SIZE {
            bitmap.insert(id);
            let value = bincode::serde::encode_to_vec(&bitmap, bincode::config::standard())?;
            inverted.insert(&last_key, &value)?;
        } else {
            let mut new_bitmap = RoaringTreemap::new();
            new_bitmap.insert(id);
            let value = bincode::serde::encode_to_vec(&new_bitmap, bincode::config::standard())?;
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
                Some(data) => {
                    bincode::serde::decode_from_slice(&data, bincode::config::standard())
                        .map(|(v, _)| v)?
                }
                None => continue,
            };
            if !bitmap.contains(id) {
                continue;
            }
            bitmap.remove(id);
            if bitmap.is_empty() {
                inverted.remove(&key)?;
            } else {
                let value = bincode::serde::encode_to_vec(&bitmap, bincode::config::standard())?;
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

        let words: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let any_empty = words.iter().any(|w| {
            Self::list_shard_indices(&inverted, w)
                .map(|idx| idx.is_empty())
                .unwrap_or(true)
        });
        if any_empty {
            return Ok(Vec::new());
        }

        let mut result: Option<RoaringTreemap> = None;
        for word in &words {
            let indices = Self::list_shard_indices(&inverted, word)?;
            let mut word_bitmap = RoaringTreemap::new();
            for &shard_idx in &indices {
                let key = Self::shard_key(word, shard_idx);
                if let Some(data) = inverted.get(&key)? {
                    if let Ok((bitmap, _)) =
                        bincode::serde::decode_from_slice::<RoaringTreemap, _>(
                            &data,
                            bincode::config::standard(),
                        )
                    {
                        word_bitmap |= &bitmap;
                    }
                }
            }
            result = match result {
                None => Some(word_bitmap),
                Some(r) => Some(&r & &word_bitmap),
            };
        }

        let bitmap = match result {
            Some(b) => b,
            None => return Ok(Vec::new()),
        };

        let after_val = after.and_then(|a| a.parse::<u64>().ok());
        let mut ids: Vec<u64> = bitmap.into_iter().collect();
        if sort_desc {
            ids.sort_unstable_by(|a, b| b.cmp(a));
        } else {
            ids.sort_unstable();
        }

        let mut results: Vec<String> = Vec::with_capacity(take);
        for id in ids {
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

        Ok(results)
    }

    pub fn upsert(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        let id_u64 = match id_type {
            IdType::Number => {
                Some(id.parse::<u64>().map_err(|_| {
                    AppError::BadRequest(format!(
                        "invalid id '{}': collection '{}' expects numeric ids",
                        id, collection
                    ))
                })?)
            }
            IdType::String => None,
        };

        let _lock = self.lock.lock().unwrap();

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let new_words = Self::tokenize(content, &self.config);

        if let Some(old_data) = docs.get(id.as_bytes())? {
            let old_tokens: Vec<String> =
                bincode::serde::decode_from_slice(&old_data, bincode::config::standard())
                    .map(|(v, _)| v)?;
            let old_words: HashSet<String> = old_tokens.into_iter().collect();
            for word in old_words.difference(&new_words) {
                match id_type {
                    IdType::Number => Self::remove_from_roaring_posting_list(&inverted, word, id_u64.unwrap())?,
                    IdType::String => Self::remove_from_posting_list(&inverted, word, id)?,
                }
            }
        }

        for word in &new_words {
            match id_type {
                IdType::Number => Self::add_to_roaring_posting_list(&inverted, word, id_u64.unwrap())?,
                IdType::String => Self::add_to_posting_list(&inverted, word, id)?,
            }
        }

        let tokens: Vec<String> = new_words.into_iter().collect();
        docs.insert(
            id.as_bytes(),
            bincode::serde::encode_to_vec(&tokens, bincode::config::standard())?,
        )?;

        self.db.persist(fjall::PersistMode::SyncData)?;
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

        let words: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let any_empty = words.iter().any(|w| {
            Self::list_shard_indices(&inverted, w)
                .map(|idx| idx.is_empty())
                .unwrap_or(true)
        });
        if any_empty {
            return Ok(Vec::new());
        }

        let mut iters: Vec<WordIterState> = Vec::with_capacity(words.len());
        for word in &words {
            let indices = Self::list_shard_indices(&inverted, word)?;
            let mut state = WordIterState::new(indices, sort_desc);
            if !sort_desc {
                Self::load_first_shard(&inverted, word, &mut state)?;
            } else {
                Self::load_last_shard(&inverted, word, &mut state)?;
            }
            iters.push(state);
        }

        if let Some(cursor) = after {
            for (i, word) in words.iter().enumerate() {
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
                loop {
                    let cur = state.current();
                    match cur {
                        None => {
                            all_have = false;
                            break;
                        }
                        Some(id) => {
                            let cmp = id.cmp(&pivot);
                            let should_advance = if sort_desc {
                                cmp == std::cmp::Ordering::Greater
                            } else {
                                cmp == std::cmp::Ordering::Less
                            };
                            if should_advance {
                                Self::advance_iter(&inverted, &words[i], state, sort_desc)?;
                            } else {
                                if id != pivot.as_str() {
                                    all_have = false;
                                }
                                break;
                            }
                        }
                    }
                }
                if !all_have {
                    break;
                }
            }

            if all_have {
                results.push(pivot);
                if results.len() >= take {
                    break;
                }
                for (i, state) in iters.iter_mut().enumerate() {
                    Self::advance_iter(&inverted, &words[i], state, sort_desc)?;
                }
            }
        }

        Ok(results)
    }

    fn load_first_shard(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
    ) -> Result<(), AppError> {
        while (state.shard_pos as usize) < state.indices.len() {
            let idx = state.indices[state.shard_pos as usize];
            match Self::load_posting_shard(inverted, word, idx)? {
                Some(shard) if !shard.ids.is_empty() => {
                    state.cur = shard.ids;
                    state.cur_pos = 0;
                    return Ok(());
                }
                _ => {
                    state.shard_pos += 1;
                }
            }
        }
        state.cur.clear();
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
            match Self::load_posting_shard(inverted, word, idx)? {
                Some(shard) if !shard.ids.is_empty() => {
                    state.cur = shard.ids;
                    state.cur_pos = state.cur.len().saturating_sub(1);
                    return Ok(());
                }
                _ => {
                    state.shard_pos -= 1;
                }
            }
        }
        state.cur.clear();
        state.cur_pos = 0;
        Ok(())
    }

    fn advance_iter(
        inverted: &fjall::Keyspace,
        word: &str,
        state: &mut WordIterState,
        desc: bool,
    ) -> Result<(), AppError> {
        if desc {
            if state.cur_pos == 0 {
                loop {
                    state.shard_pos -= 1;
                    if state.shard_pos < 0 {
                        state.cur.clear();
                        state.cur_pos = 0;
                        return Ok(());
                    }
                    let idx = state.indices[state.shard_pos as usize];
                    match Self::load_posting_shard(inverted, word, idx)? {
                        Some(shard) if !shard.ids.is_empty() => {
                            state.cur = shard.ids;
                            state.cur_pos = state.cur.len().saturating_sub(1);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            } else {
                state.cur_pos -= 1;
            }
        } else {
            state.cur_pos += 1;
            if state.cur_pos >= state.cur.len() {
                loop {
                    state.shard_pos += 1;
                    if (state.shard_pos as usize) >= state.indices.len() {
                        state.cur.clear();
                        state.cur_pos = 0;
                        return Ok(());
                    }
                    let idx = state.indices[state.shard_pos as usize];
                    match Self::load_posting_shard(inverted, word, idx)? {
                        Some(shard) if !shard.ids.is_empty() => {
                            state.cur = shard.ids;
                            state.cur_pos = 0;
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
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

        let shard = match Self::load_posting_shard(inverted, word, shard_idx)? {
            Some(s) => s,
            None => {
                state.cur.clear();
                state.cur_pos = 0;
                return Ok(());
            }
        };
        state.cur = shard.ids;

        let pos = state
            .cur
            .binary_search(&cursor.to_string())
            .unwrap_or_else(|e| e);
        state.cur_pos = pos.min(state.cur.len().saturating_sub(1));

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
        let normalized = Self::normalize(last_word, self.config.strip_punctuation);
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
        Ok(results)
    }

    pub fn delete_item(&self, collection: &str, id: &str) -> Result<(), AppError> {
        let id_type = self.validate_collection_exists(collection)?;
        let id_u64 = match id_type {
            IdType::Number => {
                Some(id.parse::<u64>().map_err(|_| {
                    AppError::BadRequest(format!(
                        "invalid id '{}': collection '{}' expects numeric ids",
                        id, collection
                    ))
                })?)
            }
            IdType::String => None,
        };

        let _lock = self.lock.lock().unwrap();

        let inverted = self.inverted_keyspace(collection)?;
        let docs = self.docs_keyspace(collection)?;

        let tokens: Vec<String> = match docs.get(id.as_bytes())? {
            Some(data) => {
                bincode::serde::decode_from_slice(&data, bincode::config::standard())
                    .map(|(v, _)| v)?
            }
            None => return Err(AppError::NotFound(format!("item '{}' not found", id))),
        };

        for word in &tokens {
            match id_type {
                IdType::Number => Self::remove_from_roaring_posting_list(&inverted, word, id_u64.unwrap())?,
                IdType::String => Self::remove_from_posting_list(&inverted, word, id)?,
            }
        }

        docs.remove(id.as_bytes())?;

        self.db.persist(fjall::PersistMode::SyncData)?;
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

    pub fn delete_collection(&self, collection: &str) -> Result<(), AppError> {
        self.validate_collection_exists(collection)?;

        let _lock = self.lock.lock().unwrap();

        self.collections.write().unwrap().remove(collection);

        let inv_name = format!("{}.inverted", collection);
        if self.db.keyspace_exists(&inv_name) {
            let inv = self.db.keyspace(&inv_name, fjall::KeyspaceCreateOptions::default)?;
            self.db.delete_keyspace(inv)?;
        }
        let docs_name = format!("{}.docs", collection);
        if self.db.keyspace_exists(&docs_name) {
            let docs = self.db.keyspace(&docs_name, fjall::KeyspaceCreateOptions::default)?;
            self.db.delete_keyspace(docs)?;
        }

        let meta = self.meta_keyspace()?;
        meta.remove(collection.as_bytes())?;

        self.db.persist(fjall::PersistMode::SyncData)?;
        Ok(())
    }
}

struct WordIterState {
    indices: Vec<usize>,
    shard_pos: isize,
    cur: Vec<String>,
    cur_pos: usize,
}

impl WordIterState {
    fn new(indices: Vec<usize>, desc: bool) -> Self {
        if desc {
            let last = indices.len().saturating_sub(1);
            Self {
                indices,
                shard_pos: last as isize,
                cur: Vec::new(),
                cur_pos: 0,
            }
        } else {
            Self {
                indices,
                shard_pos: 0,
                cur: Vec::new(),
                cur_pos: 0,
            }
        }
    }

    fn current(&self) -> Option<&str> {
        if self.cur.is_empty() || self.cur_pos >= self.cur.len() {
            None
        } else {
            Some(&self.cur[self.cur_pos])
        }
    }
}
