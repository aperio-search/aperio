use std::time::Duration;

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

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
pub struct PostingShard {
    pub ids: Vec<String>,
}

#[derive(Archive, RkyvSerialize, RkyvDeserialize)]
pub struct QueuedIndex {
    pub collection: String,
    pub id: String,
    pub document: Vec<u8>,
}

#[derive(Debug, Clone, Archive, RkyvSerialize, RkyvDeserialize)]
pub struct CollectionMeta {
    pub id_type: IdType,
    pub searchable_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FSTConfig {
    pub enabled: bool,
    pub max_words: usize,
    pub max_size_kb: usize,
    pub consolidate_after_secs: u64,
}

impl Default for FSTConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_words: 250_000,
            max_size_kb: 2048,
            consolidate_after_secs: 300,
        }
    }
}

#[derive(Clone)]
pub struct StoreConfig {
    pub min_token_length: usize,
    pub max_token_length: usize,
    pub max_string_shard_size: usize,
    pub max_roaring_shard_size: u64,
    pub index_interval: Duration,
    pub max_queue_batch_size: usize,
    pub fst_config: FSTConfig,
    pub fst_consolidate_interval: Duration,
    pub fuzzy_max_expansions: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            min_token_length: 3,
            max_token_length: 400,
            max_string_shard_size: 1000,
            max_roaring_shard_size: 100_000,
            index_interval: Duration::from_millis(900),
            max_queue_batch_size: 5000,
            fst_config: FSTConfig::default(),
            fst_consolidate_interval: Duration::from_secs(60),
            fuzzy_max_expansions: 3,
        }
    }
}
