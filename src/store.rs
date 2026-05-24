use std::collections::HashSet;

use sled::Db;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::error::AppError;
use crate::models::SearchResult;

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

    pub fn upsert(&self, collection: &str, id: &str, content: &str) -> Result<(), AppError> {
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let words = Self::tokenize(content, &self.config);

        for word in &words {
            let key = word.as_bytes();
            let mut ids: Vec<String> = match inverted.get(key)? {
                Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
                None => Vec::new(),
            };

            if !ids.contains(&id.to_string()) {
                ids.push(id.to_string());
                let value = serde_json::to_vec(&ids).map_err(|e| {
                    AppError::Internal(format!("serialization error: {}", e))
                })?;
                inverted.insert(key, value)?;
            }
        }

        docs.insert(id.as_bytes(), content.as_bytes())?;

        Ok(())
    }

    pub fn search(
        &self,
        collection: &str,
        query: &str,
        sort_desc: bool,
        take: usize,
        after: Option<&str>,
    ) -> Result<(Vec<SearchResult>, usize), AppError> {
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let words: Vec<String> = Self::tokenize(query, &self.config).into_iter().collect();
        if words.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let mut ids: Option<Vec<String>> = None;

        for word in &words {
            let key = word.as_bytes();
            let mut word_ids: Vec<String> = match inverted.get(key)? {
                Some(data) => serde_json::from_slice(&data).unwrap_or_default(),
                None => return Ok((Vec::new(), 0)),
            };
            word_ids.sort();

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

        let results = ids
            .iter()
            .map(|id| {
                let content = docs
                    .get(id.as_bytes())
                    .ok()
                    .flatten()
                    .map(|v| String::from_utf8(v.to_vec()).unwrap_or_default())
                    .unwrap_or_default();
                SearchResult {
                    id: id.clone(),
                    content,
                }
            })
            .collect();

        Ok((results, total))
    }

    pub fn suggest(&self, collection: &str, prefix: &str) -> Result<Vec<String>, AppError> {
        let inverted = self.inverted_tree(collection)?;
        let last_word = prefix.split_whitespace().last().unwrap_or(prefix);
        let normalized = Self::normalize(last_word, self.config.strip_punctuation);
        let results: Vec<String> = inverted
            .scan_prefix(normalized.as_bytes())
            .take(10)
            .filter_map(|res| res.ok())
            .map(|(key, _)| String::from_utf8(key.to_vec()).unwrap_or_default())
            .collect();
        Ok(results)
    }

    pub fn delete_item(&self, collection: &str, id: &str) -> Result<(), AppError> {
        let inverted = self.inverted_tree(collection)?;
        let docs = self.docs_tree(collection)?;

        let content = match docs.get(id.as_bytes())? {
            Some(data) => String::from_utf8(data.to_vec()).map_err(|_| {
                AppError::Internal("invalid utf-8 in document store".to_string())
            })?,
            None => return Err(AppError::NotFound(format!("item '{}' not found", id))),
        };

        let words = Self::tokenize(&content, &self.config);

        for word in &words {
            let key = word.as_bytes();
            if let Some(data) = inverted.get(key)? {
                let mut ids: Vec<String> = serde_json::from_slice(&data).unwrap_or_default();
                ids.retain(|i| i != id);
                if ids.is_empty() {
                    inverted.remove(key)?;
                } else {
                    let value = serde_json::to_vec(&ids).map_err(|e| {
                        AppError::Internal(format!("serialization error: {}", e))
                    })?;
                    inverted.insert(key, value)?;
                }
            }
        }

        docs.remove(id.as_bytes())?;

        Ok(())
    }

    pub fn delete_collection(&self, collection: &str) -> Result<(), AppError> {
        self.db.drop_tree(format!("{}:inverted", collection))?;
        self.db.drop_tree(format!("{}:docs", collection))?;
        Ok(())
    }
}
