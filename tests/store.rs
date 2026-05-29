use std::collections::HashSet;

use aperio::store::{Store, StoreConfig};
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

#[test]
fn full_string_workflow() {
    let (store, _dir) = create_store();

    store.create_collection("docs", "string").unwrap();
    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 0);
    assert_eq!(info.unique_terms, 0);

    store.upsert("docs", "id1", "the quick brown fox").unwrap();
    store.upsert("docs", "id2", "jumps over the lazy dog").unwrap();
    store.upsert("docs", "id3", "brown fox quick").unwrap();

    let all = store.search("docs", "fox", false, 10, None).unwrap();
    assert_eq!(all.len(), 2);

    let intersection = store
        .search("docs", "quick brown", false, 10, None)
        .unwrap();
    assert_eq!(intersection, vec!["id1", "id3"]);

    let suggest = store.suggest("docs", "br").unwrap();
    assert!(suggest.contains(&"brown".to_string()));

    store.delete_item("docs", "id1").unwrap();
    let after_delete = store.search("docs", "fox", false, 10, None).unwrap();
    assert_eq!(after_delete, vec!["id3"]);

    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 2);

    store.delete_collection("docs").unwrap();
    let list = store.list_collections().unwrap();
    assert!(list.collections.is_empty());
}

#[test]
fn full_number_workflow() {
    let (store, _dir) = create_store();

    store.create_collection("docs", "number").unwrap();
    store.upsert("docs", "10", "hello world").unwrap();
    store.upsert("docs", "20", "hello there").unwrap();
    store.upsert("docs", "30", "world there").unwrap();

    let r1 = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(r1, vec!["10", "20"]);

    let r2 = store.search("docs", "hello world", false, 10, None).unwrap();
    assert_eq!(r2, vec!["10"]);

    let r3 = store.search("docs", "hello", true, 10, None).unwrap();
    assert_eq!(r3, vec!["20", "10"]);

    store.delete_item("docs", "10").unwrap();
    let r4 = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(r4, vec!["20"]);
}

#[test]
fn string_shard_splitting() {
    let config = StoreConfig {
        max_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config);

    store.create_collection("docs", "string").unwrap();
    for i in 0..12u64 {
        store.upsert("docs", &i.to_string(), "hello").unwrap();
    }

    let results = store.search("docs", "hello", false, 20, None).unwrap();
    assert_eq!(results.len(), 12);

    let ids: HashSet<u64> = results.iter().map(|s| s.parse().unwrap()).collect();
    for i in 0..12 {
        assert!(ids.contains(&i), "missing id {}", i);
    }
}

#[test]
fn roaring_shard_splitting() {
    let config = StoreConfig {
        max_roaring_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config);

    store.create_collection("docs", "number").unwrap();
    for i in 1..=12u64 {
        store.upsert("docs", &i.to_string(), "hello").unwrap();
    }

    let results = store.search("docs", "hello", false, 20, None).unwrap();
    assert_eq!(results.len(), 12);

    let ids: HashSet<u64> = results.iter().map(|s| s.parse().unwrap()).collect();
    for i in 1..=12 {
        assert!(ids.contains(&i), "missing id {}", i);
    }
}

#[test]
fn update_document_removes_old_tokens() {
    let (store, _dir) = create_store();

    store.create_collection("docs", "string").unwrap();
    store.upsert("docs", "1", "apple banana").unwrap();
    store.upsert("docs", "1", "apple cherry").unwrap();

    let banana = store.search("docs", "banana", false, 10, None).unwrap();
    assert!(banana.is_empty(), "banana should have been removed");

    let cherry = store.search("docs", "cherry", false, 10, None).unwrap();
    assert_eq!(cherry, vec!["1"]);

    let apple = store.search("docs", "apple", false, 10, None).unwrap();
    assert_eq!(apple, vec!["1"]);
}

#[test]
fn search_pagination_edge_cases() {
    let (store, _dir) = create_store();
    store.create_collection("docs", "string").unwrap();

    for c in ["a", "b", "c", "d", "e"] {
        store.upsert("docs", c, "hello").unwrap();
    }

    let page1 = store.search("docs", "hello", false, 2, None).unwrap();
    assert_eq!(page1, vec!["a", "b"]);

    let page2 = store
        .search("docs", "hello", false, 2, page1.last().map(String::as_str))
        .unwrap();
    assert_eq!(page2, vec!["c", "d"]);

    let page3 = store
        .search("docs", "hello", false, 2, page2.last().map(String::as_str))
        .unwrap();
    assert_eq!(page3, vec!["e"]);

    let page4 = store
        .search("docs", "hello", false, 2, page3.last().map(String::as_str))
        .unwrap();
    assert!(page4.is_empty());
}

#[test]
fn suggest_deduplicates_and_limit() {
    let (store, _dir) = create_store();
    store.create_collection("docs", "string").unwrap();

    for i in 0..20u64 {
        store
            .upsert("docs", &i.to_string(), "apple banana cherry")
            .unwrap();
    }

    let suggestions = store.suggest("docs", "app").unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0], "apple");
}

#[test]
fn multiple_collections_isolated() {
    let (store, _dir) = create_store();

    store.create_collection("a", "string").unwrap();
    store.create_collection("b", "string").unwrap();

    store.upsert("a", "1", "hello world").unwrap();
    store.upsert("b", "1", "foo bar").unwrap();

    let r1 = store.search("a", "hello", false, 10, None).unwrap();
    assert_eq!(r1, vec!["1"]);

    let r2 = store.search("b", "hello", false, 10, None).unwrap();
    assert!(r2.is_empty());

    let r3 = store.search("b", "foo", false, 10, None).unwrap();
    assert_eq!(r3, vec!["1"]);
}

#[test]
fn delete_all_items_in_collection() {
    let (store, _dir) = create_store();
    store.create_collection("docs", "string").unwrap();

    store.upsert("docs", "1", "hello").unwrap();
    store.upsert("docs", "2", "hello").unwrap();

    store.delete_item("docs", "1").unwrap();
    store.delete_item("docs", "2").unwrap();

    let results = store.search("docs", "hello", false, 10, None).unwrap();
    assert!(results.is_empty());

    let info = store.collection_info("docs").unwrap();
    assert_eq!(info.document_count, 0);
    // marker keys persist in the index, so unique_terms may be >0
    // (orphaned marker keys are a known aspect of the current design)
}

#[test]
fn persist_and_reopen() {
    let dir = TempDir::new().unwrap();

    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    store.create_collection("docs", "string").unwrap();
    store.upsert("docs", "persist", "hello world").unwrap();
    drop(store);

    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Store::new(db);
    let results = store.search("docs", "hello", false, 10, None).unwrap();
    assert_eq!(results, vec!["persist"]);

    let list = store.list_collections().unwrap();
    assert_eq!(list.collections.len(), 1);
}
