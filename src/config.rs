use std::path::Path;

use serde::Deserialize;

use crate::store::StoreConfig;

#[derive(Deserialize, Default)]
pub struct AppConfig {
    pub min_token_length: Option<usize>,
    pub max_shard_size: Option<usize>,
    pub max_roaring_shard_size: Option<u64>,
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Self {
        let path = match path {
            Some(p) => p,
            None => return Self::default(),
        };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: failed to read config file '{}': {}", path.display(), e);
                return Self::default();
            }
        };
        match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("warning: failed to parse config file '{}': {}", path.display(), e);
                Self::default()
            }
        }
    }

    pub fn merge_into_store_config(self) -> StoreConfig {
        StoreConfig {
            min_token_length: self.min_token_length.unwrap_or(2),
            max_shard_size: self.max_shard_size.unwrap_or(1000),
            max_roaring_shard_size: self.max_roaring_shard_size.unwrap_or(100_000),
            strip_punctuation: true,
        }
    }
}
