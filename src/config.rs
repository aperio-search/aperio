use std::path::Path;

use std::time::Duration;

use serde::Deserialize;

use crate::store::{FSTConfig, StoreConfig};

#[derive(Deserialize, Default)]
pub struct AppConfig {
    pub min_token_length: Option<usize>,
    pub max_token_length: Option<usize>,
    pub max_string_shard_size: Option<usize>,
    pub max_roaring_shard_size: Option<u64>,
    pub log_level: Option<String>,
    pub index_interval_ms: Option<u64>,
    pub max_queue_batch_size: Option<usize>,
    pub main_api_key: Option<String>,
    pub search_api_key: Option<String>,
    pub dumps_folder: Option<String>,
    pub fst_enabled: Option<bool>,
    pub fst_max_words: Option<usize>,
    pub fst_max_size_kb: Option<usize>,
    pub fst_consolidate_interval_secs: Option<u64>,
    pub fuzzy_max_expansions: Option<usize>,
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
        let mut base = FSTConfig::default();
        if let Some(v) = self.fst_enabled {
            base.enabled = v;
        }
        if let Some(v) = self.fst_max_words {
            base.max_words = v;
        }
        if let Some(v) = self.fst_max_size_kb {
            base.max_size_kb = v;
        }
        if let Some(v) = self.fst_consolidate_interval_secs {
            base.consolidate_after_secs = v;
        }
        StoreConfig {
            min_token_length: self.min_token_length.unwrap_or(3),
            max_token_length: self.max_token_length.unwrap_or(400),
            max_string_shard_size: self.max_string_shard_size.unwrap_or(1000),
            max_roaring_shard_size: self.max_roaring_shard_size.unwrap_or(100_000),
            index_interval: Duration::from_millis(self.index_interval_ms.unwrap_or(900)),
            max_queue_batch_size: self.max_queue_batch_size.unwrap_or(5000),
            fst_config: base,
            fuzzy_max_expansions: self.fuzzy_max_expansions.unwrap_or(3),
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
        assert!(cfg.max_token_length.is_none());
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
        assert_eq!(store_cfg.max_token_length, 400);
        assert_eq!(store_cfg.max_string_shard_size, 1000);
        assert_eq!(store_cfg.max_roaring_shard_size, 100_000);
        assert_eq!(store_cfg.index_interval, Duration::from_millis(900));
        assert_eq!(store_cfg.max_queue_batch_size, 5000);
    }

    #[test]
    fn merge_overrides() {
        let app_cfg = AppConfig {
            min_token_length: Some(5),
            max_token_length: None,
            max_string_shard_size: Some(200),
            max_roaring_shard_size: Some(50_000),
            index_interval_ms: Some(300),
            max_queue_batch_size: Some(500),
            log_level: None,
            main_api_key: None,
            search_api_key: None,
            dumps_folder: None,
            fst_enabled: None,
            fst_max_words: None,
            fst_max_size_kb: None,
            fst_consolidate_interval_secs: None,
            fuzzy_max_expansions: None,
        };
        let store_cfg = app_cfg.merge_into_store_config();
        assert_eq!(store_cfg.min_token_length, 5);
        assert_eq!(store_cfg.max_token_length, 400);
        assert_eq!(store_cfg.max_string_shard_size, 200);
        assert_eq!(store_cfg.max_roaring_shard_size, 50_000);
        assert_eq!(store_cfg.index_interval, Duration::from_millis(300));
        assert_eq!(store_cfg.max_queue_batch_size, 500);
    }

    #[test]
    fn fst_consolidate_interval_secs_is_wired_through() {
        // Regression test: previously the outer StoreConfig had a dead
        // `fst_consolidate_interval: Duration` field that looked like the
        // configurable knob but was never read; the real consumer is
        // FSTConfig.consolidate_after_secs. Make sure setting the config
        // option actually changes the value the FST consults.
        let app_cfg = AppConfig {
            fst_consolidate_interval_secs: Some(123),
            ..AppConfig::default()
        };
        let store_cfg = app_cfg.merge_into_store_config();
        assert_eq!(store_cfg.fst_config.consolidate_after_secs, 123);

        // And the default should match the documented 300s default.
        let default_cfg = AppConfig::default().merge_into_store_config();
        assert_eq!(default_cfg.fst_config.consolidate_after_secs, 300);
    }

    #[test]
    fn merge_max_token_length_override() {
        let app_cfg = AppConfig {
            min_token_length: None,
            max_token_length: Some(10),
            max_string_shard_size: None,
            max_roaring_shard_size: None,
            index_interval_ms: None,
            max_queue_batch_size: None,
            log_level: None,
            main_api_key: None,
            search_api_key: None,
            dumps_folder: None,
            fst_enabled: None,
            fst_max_words: None,
            fst_max_size_kb: None,
            fst_consolidate_interval_secs: None,
            fuzzy_max_expansions: None,
        };
        let store_cfg = app_cfg.merge_into_store_config();
        assert_eq!(store_cfg.max_token_length, 10);
    }
}
