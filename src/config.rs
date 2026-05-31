use std::path::Path;

use std::time::Duration;

use serde::Deserialize;

use crate::store::StoreConfig;

#[derive(Deserialize, Default)]
pub struct AppConfig {
    pub min_token_length: Option<usize>,
    pub max_string_shard_size: Option<usize>,
    pub max_roaring_shard_size: Option<u64>,
    pub block_cache_size: Option<u64>,
    pub inverted_write_buffer_size: Option<u64>,
    pub docs_buffer_size: Option<u64>,
    pub index_queue_buffer_size: Option<u64>,
    pub inverted_roaring_block_size: Option<u32>,
    pub inverted_string_block_size: Option<u32>,
    pub docs_block_size: Option<u32>,
    pub queue_block_size: Option<u32>,
    pub meta_block_size: Option<u32>,
    pub maintenance_threads: Option<usize>,
    pub docs_compression: Option<String>,
    pub inverted_string_compression: Option<String>,
    pub inverted_roaring_compression: Option<String>,
    pub index_queue_compression: Option<String>,
    pub collections_compression: Option<String>,
    pub inverted_hash_ratio: Option<f32>,
    pub docs_hash_ratio: Option<f32>,
    pub log_level: Option<String>,
    pub index_interval_ms: Option<u64>,
    pub max_queue_batch_size: Option<usize>,
    pub main_api_key: Option<String>,
    pub search_api_key: Option<String>,
    pub dumps_folder: Option<String>,
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Self {
        let path = match path {
            Some(p) => p,
            None => return Self::default(),
        };
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!("failed to read config file '{}': {e}", path.display());
        });
        toml::from_str(&content).unwrap_or_else(|e| {
            panic!("failed to parse config file '{}': {e}", path.display());
        })
    }

    pub fn merge_into_store_config(self) -> StoreConfig {
        StoreConfig {
            min_token_length: self.min_token_length.unwrap_or(3),
            max_string_shard_size: self.max_string_shard_size.unwrap_or(1000),
            max_roaring_shard_size: self.max_roaring_shard_size.unwrap_or(100_000),
            inverted_write_buffer_size: self.inverted_write_buffer_size,
            docs_buffer_size: self.docs_buffer_size,
            index_queue_buffer_size: self.index_queue_buffer_size,
            inverted_roaring_block_size: self.inverted_roaring_block_size.unwrap_or(16384),
            inverted_string_block_size: self.inverted_string_block_size.unwrap_or(65536),
            docs_block_size: self.docs_block_size.unwrap_or(8192),
            queue_block_size: self.queue_block_size.unwrap_or(32768),
            meta_block_size: self.meta_block_size.unwrap_or(8192),
            docs_compression: self.docs_compression.and_then(|s| parse_compression(&s)),
            inverted_string_compression: self
                .inverted_string_compression
                .and_then(|s| parse_compression(&s)),
            inverted_roaring_compression: self
                .inverted_roaring_compression
                .and_then(|s| parse_compression(&s)),
            index_queue_compression: self
                .index_queue_compression
                .and_then(|s| parse_compression(&s)),
            collections_compression: self
                .collections_compression
                .and_then(|s| parse_compression(&s)),
            inverted_hash_ratio: self.inverted_hash_ratio.unwrap_or(8.0),
            docs_hash_ratio: self.docs_hash_ratio.unwrap_or(8.0),
            index_interval: Duration::from_millis(self.index_interval_ms.unwrap_or(900)),
            max_queue_batch_size: self.max_queue_batch_size.unwrap_or(5000),
        }
    }
}

fn parse_compression(s: &str) -> Option<fjall::CompressionType> {
    match s.to_lowercase().as_str() {
        "none" => Some(fjall::CompressionType::None),
        "lz4" => Some(fjall::CompressionType::Lz4),
        _ => {
            tracing::warn!(value = %s, "unknown compression type, expected 'none' or 'lz4'");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_none_path() {
        let cfg = AppConfig::load(None);
        assert!(cfg.min_token_length.is_none());
        assert!(cfg.max_string_shard_size.is_none());
        assert!(cfg.max_roaring_shard_size.is_none());
    }

    #[test]
    #[should_panic(expected = "failed to read config file")]
    fn load_invalid_path() {
        AppConfig::load(Some(std::path::Path::new("/nonexistent/config.toml")));
    }

    #[test]
    fn load_valid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
min_token_length = 3
max_string_shard_size = 500
max_roaring_shard_size = 50000
max_queue_batch_size = 2000
block_cache_size = 67108864
inverted_write_buffer_size = 16777216
docs_buffer_size = 8388608
index_queue_buffer_size = 33554432
inverted_roaring_block_size = 16384
inverted_string_block_size = 65536
docs_block_size = 8192
queue_block_size = 32768
meta_block_size = 8192
maintenance_threads = 2
docs_compression = "lz4"
inverted_string_compression = "lz4"
log_level = "debug"
index_interval_ms = 500
dumps_folder = "/data/dumps"
"#,
        )
        .unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert_eq!(cfg.min_token_length, Some(3));
        assert_eq!(cfg.max_string_shard_size, Some(500));
        assert_eq!(cfg.max_roaring_shard_size, Some(50000));
        assert_eq!(cfg.max_queue_batch_size, Some(2000));
        assert_eq!(cfg.block_cache_size, Some(67108864));
        assert_eq!(cfg.inverted_write_buffer_size, Some(16777216));
        assert_eq!(cfg.docs_buffer_size, Some(8388608));
        assert_eq!(cfg.index_queue_buffer_size, Some(33554432));
        assert_eq!(cfg.inverted_roaring_block_size, Some(16384));
        assert_eq!(cfg.inverted_string_block_size, Some(65536));
        assert_eq!(cfg.docs_block_size, Some(8192));
        assert_eq!(cfg.queue_block_size, Some(32768));
        assert_eq!(cfg.meta_block_size, Some(8192));
        assert_eq!(cfg.maintenance_threads, Some(2));
        assert_eq!(cfg.docs_compression.as_deref(), Some("lz4"));
        assert_eq!(cfg.inverted_string_compression.as_deref(), Some("lz4"));
        assert!(cfg.inverted_roaring_compression.is_none());
        assert!(cfg.index_queue_compression.is_none());
        assert!(cfg.collections_compression.is_none());
        assert_eq!(cfg.log_level.as_deref(), Some("debug"));
        assert_eq!(cfg.index_interval_ms, Some(500));
        assert_eq!(cfg.dumps_folder.as_deref(), Some("/data/dumps"));
    }

    #[test]
    fn load_partial_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"min_token_length = 5"#).unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert_eq!(cfg.min_token_length, Some(5));
        assert!(cfg.max_string_shard_size.is_none());
    }

    #[test]
    fn load_with_api_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
main_api_key = "custom-main-key"
search_api_key = "custom-search-key"
"#,
        )
        .unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert_eq!(cfg.main_api_key.as_deref(), Some("custom-main-key"));
        assert_eq!(cfg.search_api_key.as_deref(), Some("custom-search-key"));
    }

    #[test]
    fn load_without_api_keys() {
        let cfg = AppConfig::load(None);
        assert!(cfg.main_api_key.is_none());
        assert!(cfg.search_api_key.is_none());
    }

    #[test]
    #[should_panic(expected = "failed to parse config file")]
    fn load_invalid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        AppConfig::load(Some(&path));
    }

    #[test]
    fn merge_defaults() {
        let store_cfg = AppConfig::default().merge_into_store_config();
        assert_eq!(store_cfg.min_token_length, 3);
        assert_eq!(store_cfg.max_string_shard_size, 1000);
        assert_eq!(store_cfg.max_roaring_shard_size, 100_000);
        assert!(store_cfg.inverted_write_buffer_size.is_none());
        assert!(store_cfg.docs_buffer_size.is_none());
        assert!(store_cfg.index_queue_buffer_size.is_none());
        assert_eq!(store_cfg.inverted_roaring_block_size, 16384);
        assert_eq!(store_cfg.inverted_string_block_size, 65536);
        assert_eq!(store_cfg.docs_block_size, 8192);
        assert_eq!(store_cfg.queue_block_size, 32768);
        assert_eq!(store_cfg.meta_block_size, 8192);
        assert!(store_cfg.docs_compression.is_none());
        assert!(store_cfg.inverted_string_compression.is_none());
        assert!(store_cfg.inverted_roaring_compression.is_none());
        assert!(store_cfg.index_queue_compression.is_none());
        assert!(store_cfg.collections_compression.is_none());
        assert_eq!(store_cfg.inverted_hash_ratio, 8.0);
        assert_eq!(store_cfg.docs_hash_ratio, 8.0);
        assert_eq!(store_cfg.index_interval, Duration::from_millis(900));
        assert_eq!(store_cfg.max_queue_batch_size, 5000);
    }

    #[test]
    fn merge_overrides() {
        let app_cfg = AppConfig {
            min_token_length: Some(5),
            max_string_shard_size: Some(200),
            max_roaring_shard_size: Some(50_000),
            inverted_write_buffer_size: Some(8_000_000),
            docs_buffer_size: Some(4_000_000),
            index_queue_buffer_size: Some(16_000_000),
            inverted_roaring_block_size: Some(32768),
            inverted_string_block_size: Some(131072),
            docs_block_size: Some(16384),
            queue_block_size: Some(65536),
            meta_block_size: Some(4096),
            docs_compression: Some("lz4".into()),
            inverted_string_compression: None,
            inverted_roaring_compression: Some("lz4".into()),
            index_queue_compression: Some("none".into()),
            collections_compression: None,
            inverted_hash_ratio: Some(4.0),
            docs_hash_ratio: None,
            index_interval_ms: Some(300),
            max_queue_batch_size: Some(500),
            block_cache_size: None,
            maintenance_threads: None,
            log_level: None,
            main_api_key: None,
            search_api_key: None,
            dumps_folder: None,
        };
        let store_cfg = app_cfg.merge_into_store_config();
        assert_eq!(store_cfg.min_token_length, 5);
        assert_eq!(store_cfg.max_string_shard_size, 200);
        assert_eq!(store_cfg.max_roaring_shard_size, 50_000);
        assert_eq!(store_cfg.inverted_write_buffer_size, Some(8_000_000));
        assert_eq!(store_cfg.docs_buffer_size, Some(4_000_000));
        assert_eq!(store_cfg.index_queue_buffer_size, Some(16_000_000));
        assert_eq!(store_cfg.inverted_roaring_block_size, 32768);
        assert_eq!(store_cfg.inverted_string_block_size, 131072);
        assert_eq!(store_cfg.docs_block_size, 16384);
        assert_eq!(store_cfg.queue_block_size, 65536);
        assert_eq!(store_cfg.meta_block_size, 4096);
        assert_eq!(
            store_cfg.docs_compression,
            Some(fjall::CompressionType::Lz4)
        );
        assert_eq!(
            store_cfg.inverted_roaring_compression,
            Some(fjall::CompressionType::Lz4)
        );
        assert_eq!(
            store_cfg.index_queue_compression,
            Some(fjall::CompressionType::None)
        );
        assert!(store_cfg.inverted_string_compression.is_none());
        assert!(store_cfg.collections_compression.is_none());
        assert_eq!(store_cfg.inverted_hash_ratio, 4.0);
        assert_eq!(store_cfg.docs_hash_ratio, 8.0);
        assert_eq!(store_cfg.index_interval, Duration::from_millis(300));
        assert_eq!(store_cfg.max_queue_batch_size, 500);
    }

    #[test]
    fn parse_compression_none() {
        let result = parse_compression("none");
        assert_eq!(result, Some(fjall::CompressionType::None));
    }

    #[test]
    fn parse_compression_lz4() {
        let result = parse_compression("lz4");
        assert_eq!(result, Some(fjall::CompressionType::Lz4));
    }

    #[test]
    fn parse_compression_unknown() {
        let result = parse_compression("zstd");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_compression_case_insensitive() {
        let result = parse_compression("LZ4");
        assert_eq!(result, Some(fjall::CompressionType::Lz4));
    }
}
