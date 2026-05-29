use std::path::Path;

use std::time::Duration;

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
    pub log_level: Option<String>,
    pub index_interval_ms: Option<u64>,
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
                tracing::warn!(path = %path.display(), error = %e, "failed to read config file");
                return Self::default();
            }
        };
        match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to parse config file");
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
            index_interval: Duration::from_millis(self.index_interval_ms.unwrap_or(900)),
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
        assert!(cfg.max_shard_size.is_none());
        assert!(cfg.max_roaring_shard_size.is_none());
    }

    #[test]
    fn load_invalid_path() {
        let cfg = AppConfig::load(Some(std::path::Path::new("/nonexistent/config.toml")));
        assert!(cfg.min_token_length.is_none());
    }

    #[test]
    fn load_valid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
min_token_length = 3
max_shard_size = 500
max_roaring_shard_size = 50000
block_cache_size = 67108864
write_buffer_size = 16777216
maintenance_threads = 2
compression = "lz4"
log_level = "debug"
index_interval_ms = 500
"#,
        )
        .unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert_eq!(cfg.min_token_length, Some(3));
        assert_eq!(cfg.max_shard_size, Some(500));
        assert_eq!(cfg.max_roaring_shard_size, Some(50000));
        assert_eq!(cfg.block_cache_size, Some(67108864));
        assert_eq!(cfg.write_buffer_size, Some(16777216));
        assert_eq!(cfg.maintenance_threads, Some(2));
        assert_eq!(cfg.compression.as_deref(), Some("lz4"));
        assert_eq!(cfg.log_level.as_deref(), Some("debug"));
        assert_eq!(cfg.index_interval_ms, Some(500));
    }

    #[test]
    fn load_partial_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"min_token_length = 5"#).unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert_eq!(cfg.min_token_length, Some(5));
        assert!(cfg.max_shard_size.is_none());
    }

    #[test]
    fn load_invalid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        let cfg = AppConfig::load(Some(&path));
        assert!(cfg.min_token_length.is_none());
    }

    #[test]
    fn merge_defaults() {
        let store_cfg = AppConfig::default().merge_into_store_config();
        assert_eq!(store_cfg.min_token_length, 2);
        assert_eq!(store_cfg.max_shard_size, 1000);
        assert_eq!(store_cfg.max_roaring_shard_size, 100_000);
        assert!(store_cfg.write_buffer_size.is_none());
        assert!(store_cfg.compression.is_none());
        assert_eq!(store_cfg.index_interval, Duration::from_millis(900));
    }

    #[test]
    fn merge_overrides() {
        let app_cfg = AppConfig {
            min_token_length: Some(5),
            max_shard_size: Some(200),
            max_roaring_shard_size: Some(50_000),
            write_buffer_size: Some(8_000_000),
            compression: Some("lz4".into()),
            index_interval_ms: Some(300),
            block_cache_size: None,
            maintenance_threads: None,
            log_level: None,
        };
        let store_cfg = app_cfg.merge_into_store_config();
        assert_eq!(store_cfg.min_token_length, 5);
        assert_eq!(store_cfg.max_shard_size, 200);
        assert_eq!(store_cfg.max_roaring_shard_size, 50_000);
        assert_eq!(store_cfg.write_buffer_size, Some(8_000_000));
        assert_eq!(store_cfg.compression, Some(fjall::CompressionType::Lz4));
        assert_eq!(store_cfg.index_interval, Duration::from_millis(300));
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
