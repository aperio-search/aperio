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

#[derive(Clone)]
pub struct StoreConfig {
    pub min_token_length: usize,
    pub max_string_shard_size: usize,
    pub max_roaring_shard_size: u64,
    pub inverted_write_buffer_size: Option<u64>,
    pub docs_buffer_size: Option<u64>,
    pub index_queue_buffer_size: Option<u64>,
    pub inverted_roaring_block_size: u32,
    pub inverted_string_block_size: u32,
    pub docs_block_size: u32,
    pub queue_block_size: u32,
    pub meta_block_size: u32,
    pub docs_compression: Option<fjall::CompressionType>,
    pub inverted_string_compression: Option<fjall::CompressionType>,
    pub inverted_roaring_compression: Option<fjall::CompressionType>,
    pub index_queue_compression: Option<fjall::CompressionType>,
    pub collections_compression: Option<fjall::CompressionType>,
    pub inverted_hash_ratio: f32,
    pub docs_hash_ratio: f32,
    pub index_interval: Duration,
    pub max_queue_batch_size: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            min_token_length: 3,
            max_string_shard_size: 1000,
            max_roaring_shard_size: 100_000,
            inverted_write_buffer_size: None,
            docs_buffer_size: None,
            index_queue_buffer_size: None,
            inverted_roaring_block_size: 16384,
            inverted_string_block_size: 65536,
            docs_block_size: 8192,
            queue_block_size: 32768,
            meta_block_size: 8192,
            docs_compression: None,
            inverted_string_compression: None,
            inverted_roaring_compression: None,
            index_queue_compression: None,
            collections_compression: None,
            inverted_hash_ratio: 8.0,
            docs_hash_ratio: 8.0,
            index_interval: Duration::from_millis(900),
            max_queue_batch_size: 5000,
        }
    }
}

impl StoreConfig {
    pub fn keyspace_opts(
        &self,
        block_size: u32,
        hash_ratio: f32,
        write_buffer_size: Option<u64>,
        compression: Option<fjall::CompressionType>,
    ) -> fjall::KeyspaceCreateOptions {
        let mut opts = fjall::KeyspaceCreateOptions::default();
        if let Some(size) = write_buffer_size {
            opts = opts.max_memtable_size(size);
        }
        opts = opts.data_block_size_policy(fjall::config::BlockSizePolicy::all(block_size));
        if hash_ratio > 0.0 {
            opts =
                opts.data_block_hash_ratio_policy(fjall::config::HashRatioPolicy::all(hash_ratio));
        }
        if let Some(comp) = compression {
            opts = opts.data_block_compression_policy(fjall::config::CompressionPolicy::all(comp));
        }
        opts
    }
}
