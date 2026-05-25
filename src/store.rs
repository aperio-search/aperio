use std::collections::HashSet;

use sled::Db;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::error::AppError;
use crate::models::CollectionInfo;

const MAX_SHARD_SIZE: usize = 1000;
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

pub struct Store {
    db: Db,
    config: StoreConfig,
}

impl Store {
    pub fn new(db: Db) -> Self {
        Self::with_config(db, StoreConfig::default())
    }

    pub fn with_config(db: Db, config: StoreConfig) -> Self {
        Self { db, config }
    }

    fn inverted_tree(&self, collection: &str) -> Result<sled::Tree, AppError> {
        Ok(self.db.open_tree(format!("{}:inverted", collection))?)
    }

    fn docs_tree(&self, collection: &str) -> Result<sled::Tree, AppError> {
        Ok(self.db.open_tree(format!("{}:docs", collection))?)
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

    fn collect_shards(
        inverted: &sled::Tree,
        word: &str,
    ) -> Result<Vec<(Vec<u8>, Vec<String>)>, AppError> {
        let prefix = format!("{}{}", word, SHARD_DELIM).into_bytes();
        let mut shards = Vec::new();
        for res in inverted.scan_prefix(prefix) {
            let (key, value) = res?;
            let ids: Vec<String> = serde_json::from_slice(&value).unwrap_or_default();
            shards.push((key.to_vec(), ids));
        }
        Ok(shards)
    }

    fn add_to_posting_list(
        inverted: &sled::Tree,
        word: &str,
        id: &str,
    ) -> Result<(), AppError> {
        let marker_key = word.as_bytes();
        let _ = inverted.compare_and_swap(marker_key, None::<&[u8]>, Some(&[]));

        loop {
            let shards = Self::collect_shards(inverted, word)?;

            for (_, ids) in &shards {
                if ids.contains(&id.to_string()) {
                    return Ok(());
                }
            }

            let mut target_idx: Option<usize> = None;
            for (i, (_, ids)) in shards.iter().enumerate() {
                if ids.len() < MAX_SHARD_SIZE {
                    target_idx = Some(i);
                    break;
                }
            }

            if let Some(idx) = target_idx {
                let key = shards[idx].0.clone();
                let old_value = inverted.get(&key)?;
                let mut current_ids: Vec<String> = match old_value.as_deref() {
                    Some(data) => serde_json::from_slice(data).unwrap_or_default(),
                    None => Vec::new(),
                };

                if current_ids.contains(&id.to_string()) {
                    return Ok(());
                }

                if current_ids.len() >= MAX_SHARD_SIZE {
                    continue;
                }

                current_ids.push(id.to_string());
                current_ids.sort();
                let value = serde_json::to_vec(&current_ids)
                    .map_err(|e| AppError::Internal(format!("serialization error: {}", e)))?;
                match inverted.compare_and_swap(&key, old_value.as_deref(), Some(value))? {
                    Ok(()) => return Ok(()),
                    Err(_) => continue,
                }
            } else {
                let new_key = Self::shard_key(word, shards.len());
                let value = serde_json::to_vec(&[id])
                    .map_err(|e| AppError::Internal(format!("serialization error: {}", e)))?;
                match inverted.compare_and_swap(&new_key, None::<&[u8]>, Some(value))? {
                    Ok(()) => return Ok(()),
                    Err(_) => continue,
                }
            }
        }
    }

    fn remove_from_posting_list(
        inverted: &sled::Tree,
        word: &str,
        id: &str,
    ) -> Result<(), AppError> {
        'outer: loop {
            let shards = Self::collect_shards(inverted, word)?;
            if shards.is_empty() {
                return Ok(());
            }

            for (key, ids) in &shards {
                if !ids.contains(&id.to_string()) {
                    continue;
                }

                let old_value = inverted.get(key)?;
                let current_ids: Vec<String> = match old_value.as_deref() {
                    Some(data) => serde_json::from_slice(data).unwrap_or_default(),
                    None => return Ok(()),
                };

                let len_before = current_ids.len();
                let new_ids: Vec<String> = current_ids.into_iter().filter(|i| i != id).collect();
                if new_ids.len() == len_before {
                    return Ok(());
                }

                if new_ids.is_empty() {
                    match inverted.compare_and_swap(key, old_value.as_deref(), None::<Vec<u8>>)? {
                        Ok(()) => return Ok(()),
                        Err(_) => continue 'outer,
                    }
                } else {
                    let value = serde_json::to_vec(&new_ids)
                        .map_err(|e| AppError::Internal(format!("serialization error: {}", e)))?;
                    match inverted.compare_and_swap(key, old_value.as_deref(), Some(value))? {
                        Ok(()) => return Ok(()),
                        Err(_) => continue 'outer,
                    }
                }
            }

            return Ok(());
        }
    }

    pub fn upsert(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let new_words = Self::tokenize(content, &self.config);

        if let Some(old_data) = docs.get(id.as_bytes())? {
            let old_tokens: Vec<String> = serde_json::from_slice(&old_data)
                .map_err(|e| AppError::Internal(format!("deserialize error: {}", e)))?;
            let old_words: HashSet<String> = old_tokens.into_iter().collect();
            for word in old_words.difference(&new_words) {
                Self::remove_from_posting_list(&inverted, word, id)?;
            }
        }

        for word in &new_words {
            Self::add_to_posting_list(&inverted, word, id)?;
        }

        let tokens: Vec<String> = new_words.into_iter().collect();
        docs.insert(
            id.as_bytes(),
            serde_json::to_vec(&tokens)
                .map_err(|e| AppError::Internal(format!("serialization error: {}", e)))?,
        )?;

        Ok(())
    }

    pub fn search(
        &self,
        collection: &str,
        query: &str,
        sort_desc: bool,
        take: usize,
        after: Option<&str>,
    ) -> Result<(Vec<String>, usize), AppError> {
        let inverted = self.inverted_tree(collection)?;

        let words: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if words.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let mut ids: Option<Vec<String>> = None;

        for word in &words {
            let shards = Self::collect_shards(&inverted, word)?;
            if shards.is_empty() {
                return Ok((Vec::new(), 0));
            }

            let mut word_ids: Vec<String> = Vec::new();
            for (_, shard_ids) in &shards {
                word_ids.extend(shard_ids.iter().cloned());
            }
            word_ids.sort();
            word_ids.dedup();

            match ids {
                Some(ref mut acc) => acc.retain(|id| word_ids.binary_search(id).is_ok()),
                None => ids = Some(word_ids),
            }
        }

        let mut ids = ids.unwrap_or_default();
        let total = ids.len();

        if sort_desc {
            ids.reverse();
        }

        if let Some(cursor) = after {
            if sort_desc {
                ids.retain(|id| id.as_str() < cursor);
            } else {
                ids.retain(|id| id.as_str() > cursor);
            }
        }

        ids.truncate(take);

        Ok((ids, total))
    }

    pub fn suggest(&self, collection: &str, prefix: &str) -> Result<Vec<String>, AppError> {
        let inverted = self.inverted_tree(collection)?;
        let last_word = prefix.split_whitespace().last().unwrap_or(prefix);
        let normalized = Self::normalize(last_word, self.config.strip_punctuation);
        let mut seen = HashSet::new();
        let results: Vec<String> = inverted
            .scan_prefix(normalized.as_bytes())
            .take(50)
            .filter_map(|res| res.ok())
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
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let tokens: Vec<String> = match docs.get(id.as_bytes())? {
            Some(data) => serde_json::from_slice(&data)
                .map_err(|e| AppError::Internal(format!("deserialize error: {}", e)))?,
            None => return Err(AppError::NotFound(format!("item '{}' not found", id))),
        };

        for word in &tokens {
            Self::remove_from_posting_list(&inverted, word, id)?;
        }

        docs.remove(id.as_bytes())?;

        Ok(())
    }

    pub fn collection_info(&self, collection: &str) -> Result<CollectionInfo, AppError> {
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let mut unique_terms = 0usize;
        for res in inverted.iter() {
            let (key, value) = res?;
            if !key.contains(&(SHARD_DELIM as u8)) && value.is_empty() {
                unique_terms += 1;
            }
        }

        Ok(CollectionInfo {
            name: collection.to_string(),
            document_count: docs.len(),
            unique_terms,
        })
    }

    pub fn delete_collection(&self, collection: &str) -> Result<(), AppError> {
        self.db.drop_tree(format!("{}:inverted", collection))?;
        self.db.drop_tree(format!("{}:docs", collection))?;
        Ok(())
    }
}
