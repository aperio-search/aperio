use std::path::Path;

use serde::Deserialize;

use crate::store::StoreConfig;

#[derive(Deserialize, Default)]
pub struct AppConfig {
    pub min_token_length: Option<usize>,
    pub max_shard_size: Option<usize>,
    pub max_roaring_shard_size: Option<u64>,
    pub block_cache_size: Option<u64>,
    pub write_buffer_size: Option<u64>,
    pub maintenance_threads: Option<usize>,
    pub compression: Option<String>,
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
                eprintln!(
                    "warning: failed to read config file '{}': {}",
                    path.display(),
                    e
                );
                return Self::default();
            }
        };
        match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!(
                    "warning: failed to parse config file '{}': {}",
                    path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    pub fn merge_into_store_config(self) -> StoreConfig {
        StoreConfig {
            min_token_length: self.min_token_length.unwrap_or(2),
            max_shard_size: self.max_shard_size.unwrap_or(1000),
            max_roaring_shard_size: self.max_roaring_shard_size.unwrap_or(100_000),
            write_buffer_size: self.write_buffer_size,
            compression: self.compression.and_then(|s| parse_compression(&s)),
        }
    }
}

fn parse_compression(s: &str) -> Option<fjall::CompressionType> {
    match s.to_lowercase().as_str() {
        "none" => Some(fjall::CompressionType::None),
        "lz4" => Some(fjall::CompressionType::Lz4),
        _ => {
            eprintln!(
                "warning: unknown compression type '{}', expected 'none' or 'lz4'",
                s
            );
            None
        }
    }
}
