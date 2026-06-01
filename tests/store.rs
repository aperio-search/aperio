use std::collections::HashSet;

use aperio::store::{Store, StoreConfig};
use serde_json::json;
use tempfile::TempDir;

fn create_store() -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    (store, dir)
}

fn create_store_with_config(config: StoreConfig) -> (Store, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::with_config(db, config);
    (store, dir)
}

fn ids(results: &[serde_json::Value]) -> Vec<String> {
    results
        .iter()
        .filter_map(|v| match &v["id"] {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn full_string_workflow() {
    let (store, _dir) = create_store();

    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();
    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 0);

    store
        .upsert(
            "docs",
            json!({"id": "id1", "content": "the quick brown fox"}),
        )
        .unwrap();
    store
        .upsert(
            "docs",
            json!({"id": "id2", "content": "jumps over the lazy dog"}),
        )
        .unwrap();
    store
        .upsert("docs", json!({"id": "id3", "content": "brown fox quick"}))
        .unwrap();
    store.flush().unwrap();

    let all = store.search("docs", "fox", false, 10, None).unwrap();
    assert_eq!(all.len(), 2);

    let intersection = store
        .search("docs", "quick brown", false, 10, None)
        .unwrap();
    assert_eq!(ids(&intersection), vec!["id1", "id3"]);

    store.delete_item("docs", "id1").unwrap();
    let after_delete = store.search("docs", "fox", false, 10, None).unwrap();
    assert_eq!(ids(&after_delete), vec!["id3"]);

    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 2);

    store.delete_collection("docs").unwrap();
    let list = store.list_collections().unwrap();
    assert!(list.collections.is_empty());
}

#[test]
fn full_number_workflow() {
    let (store, _dir) = create_store();

    store
        .create_collection("docs", "number", &["content".into()])
        .unwrap();
    store
        .upsert("docs", json!({"id": 10, "content": "hello world"}))
        .unwrap();
    store
        .upsert("docs", json!({"id": 20, "content": "hello there"}))
        .unwrap();
    store
        .upsert("docs", json!({"id": 30, "content": "world there"}))
        .unwrap();
    store.flush().unwrap();

    let r1 = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(ids(&r1), vec!["10", "20"]);

    let r2 = store
        .search("docs", "hello world", false, 10, None)
        .unwrap();
    assert_eq!(ids(&r2), vec!["10"]);

    let r3 = store.search("docs", "hello", true, 10, None).unwrap();
    assert_eq!(ids(&r3), vec!["20", "10"]);

    store.delete_item("docs", "10").unwrap();
    let r4 = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(ids(&r4), vec!["20"]);
}

#[test]
fn string_shard_splitting() {
    let config = StoreConfig {
        max_string_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config);

    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();
    for i in 0..12u64 {
        store
            .upsert("docs", json!({"id": i.to_string(), "content": "hello"}))
            .unwrap();
    }
    store.flush().unwrap();

    let results = store.search("docs", "hello", false, 20, None).unwrap();
    assert_eq!(results.len(), 12);

    let result_ids: HashSet<u64> = ids(&results).iter().map(|s| s.parse().unwrap()).collect();
    for i in 0..12 {
        assert!(result_ids.contains(&i), "missing id {}", i);
    }
}

#[test]
fn roaring_shard_splitting() {
    let config = StoreConfig {
        max_roaring_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config);

    store
        .create_collection("docs", "number", &["content".into()])
        .unwrap();
    for i in 1..=12u64 {
        store
            .upsert("docs", json!({"id": i, "content": "hello"}))
            .unwrap();
    }
    store.flush().unwrap();

    let results = store.search("docs", "hello", false, 20, None).unwrap();
    assert_eq!(results.len(), 12);

    let result_ids: HashSet<u64> = ids(&results).iter().map(|s| s.parse().unwrap()).collect();
    for i in 1..=12 {
        assert!(result_ids.contains(&i), "missing id {}", i);
    }
}

#[test]
fn update_document_removes_old_tokens() {
    let (store, _dir) = create_store();

    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();
    store
        .upsert("docs", json!({"id": "1", "content": "apple banana"}))
        .unwrap();
    store
        .upsert("docs", json!({"id": "1", "content": "apple cherry"}))
        .unwrap();
    store.flush().unwrap();

    let banana = store.search("docs", "banana", false, 10, None).unwrap();
    assert!(banana.is_empty(), "banana should have been removed");

    let cherry = store.search("docs", "cherry", false, 10, None).unwrap();
    assert_eq!(ids(&cherry), vec!["1"]);

    let apple = store.search("docs", "apple", false, 10, None).unwrap();
    assert_eq!(ids(&apple), vec!["1"]);
}

#[test]
fn search_pagination_edge_cases() {
    let (store, _dir) = create_store();
    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();

    for c in ["a", "b", "c", "d", "e"] {
        store
            .upsert("docs", json!({"id": c, "content": "hello"}))
            .unwrap();
    }
    store.flush().unwrap();

    let page1 = store.search("docs", "hello", false, 2, None).unwrap();
    assert_eq!(ids(&page1), vec!["a", "b"]);

    let page2 = store
        .search(
            "docs",
            "hello",
            false,
            2,
            page1.last().and_then(|v| v["id"].as_str()),
        )
        .unwrap();
    assert_eq!(ids(&page2), vec!["c", "d"]);

    let page3 = store
        .search(
            "docs",
            "hello",
            false,
            2,
            page2.last().and_then(|v| v["id"].as_str()),
        )
        .unwrap();
    assert_eq!(ids(&page3), vec!["e"]);

    let page4 = store
        .search(
            "docs",
            "hello",
            false,
            2,
            page3.last().and_then(|v| v["id"].as_str()),
        )
        .unwrap();
    assert!(page4.is_empty());
}

#[test]
fn multiple_collections_isolated() {
    let (store, _dir) = create_store();

    store
        .create_collection("a", "string", &["content".into()])
        .unwrap();
    store
        .create_collection("b", "string", &["content".into()])
        .unwrap();

    store
        .upsert("a", json!({"id": "1", "content": "hello world"}))
        .unwrap();
    store
        .upsert("b", json!({"id": "1", "content": "foo bar"}))
        .unwrap();
    store.flush().unwrap();

    let r1 = store.search("a", "hello", false, 10, None).unwrap();
    assert_eq!(ids(&r1), vec!["1"]);

    let r2 = store.search("b", "hello", false, 10, None).unwrap();
    assert!(r2.is_empty());

    let r3 = store.search("b", "foo", false, 10, None).unwrap();
    assert_eq!(ids(&r3), vec!["1"]);
}

#[test]
fn delete_all_items_in_collection() {
    let (store, _dir) = create_store();
    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();

    store
        .upsert("docs", json!({"id": "1", "content": "hello"}))
        .unwrap();
    store
        .upsert("docs", json!({"id": "2", "content": "hello"}))
        .unwrap();
    store.flush().unwrap();

    store.delete_item("docs", "1").unwrap();
    store.delete_item("docs", "2").unwrap();

    let results = store.search("docs", "hello", false, 10, None).unwrap();
    assert!(results.is_empty());

    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 0);
}

#[test]
fn persist_and_reopen() {
    let dir = TempDir::new().unwrap();

    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();
    store
        .upsert("docs", json!({"id": "persist", "content": "hello world"}))
        .unwrap();
    store.flush().unwrap();
    drop(store);

    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    let results = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(ids(&results), vec!["persist"]);

    let list = store.list_collections().unwrap();
    assert_eq!(list.collections.len(), 1);
}

#[test]
fn export_import_roundtrip() {
    let dir = TempDir::new().unwrap();

    // Seed source
    let src_path = dir.path().join("src");
    let db = fjall::Database::builder(&src_path)
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store
        .create_collection("docs", "string", &["content".into()])
        .unwrap();
    store
        .upsert("docs", json!({"id": "a", "content": "hello world"}))
        .unwrap();
    store
        .upsert("docs", json!({"id": "b", "content": "foo bar"}))
        .unwrap();
    store.flush().unwrap();

    // Export via Store method (uses snapshot internally)
    let data = store.export_snapshot().unwrap();
    assert!(!data.is_empty(), "export data should not be empty");
    drop(store);

    // Import into a fresh store
    let dst_path = dir.path().join("dst");
    let db = fjall::Database::builder(&dst_path)
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store.import_snapshot(&data).unwrap();

    // Verify
    let list = store.list_collections().unwrap();
    assert_eq!(list.collections.len(), 1);
    assert_eq!(list.collections[0].name, "docs");

    let r1 = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0]["id"], "a");

    let r2 = store.search("docs", "foo", false, 10, None).unwrap();
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0]["id"], "b");
}

#[test]
fn export_empty_database() {
    let dir = TempDir::new().unwrap();
    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    let data = store.export_snapshot().unwrap();
    // Should produce valid export data (empty keyspace list)
    assert!(
        !data.is_empty(),
        "export data should have header even with no collections"
    );
}

#[test]
fn export_import_number_collection() {
    let dir = TempDir::new().unwrap();

    let src_path = dir.path().join("src");
    let db = fjall::Database::builder(&src_path)
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store
        .create_collection("nums", "number", &["val".into()])
        .unwrap();
    store
        .upsert("nums", json!({"id": 42, "val": "hello"}))
        .unwrap();
    store
        .upsert("nums", json!({"id": 99, "val": "world"}))
        .unwrap();
    store.flush().unwrap();

    let data = store.export_snapshot().unwrap();
    drop(store);

    let dst_path = dir.path().join("dst");
    let db = fjall::Database::builder(&dst_path)
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store.import_snapshot(&data).unwrap();

    let r = store.search("nums", "hello", false, 10, None).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["id"], 42);

    let r = store.search("nums", "world", false, 10, None).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["id"], 99);
}

#[test]
fn export_import_bad_magic() {
    let dir = TempDir::new().unwrap();
    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    let err = store.import_snapshot(b"garbage data").unwrap_err();
    assert!(err.to_string().contains("bad magic"), "got: {err}");
}
