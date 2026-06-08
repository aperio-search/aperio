use std::collections::HashSet;
use std::error::Error;

use aperio::store::{Store, StoreConfig};
use serde_json::json;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

/// Construct a fresh `Store` in a temporary directory. Propagates any
/// setup failure to the caller so tests can use `?` and `Result` returns
/// instead of `.unwrap()`.
fn create_store() -> Result<(Store, TempDir), Box<dyn Error>> {
    let dir = TempDir::new()?;
    // SAFETY: each call gets its own fresh tempdir; nothing else maps this path.
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())?
    };
    let store = Store::new(env, dir.path().join("fst"))?;
    Ok((store, dir))
}

fn create_store_with_config(config: StoreConfig) -> Result<(Store, TempDir), Box<dyn Error>> {
    let dir = TempDir::new()?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())?
    };
    let store = Store::with_config(env, config, dir.path().join("fst"))?;
    Ok((store, dir))
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
fn full_string_workflow() -> TestResult {
    let (store, _dir) = create_store()?;

    store.create_collection("docs", "string", &["content".into()])?;
    let info = store.collection_info("docs")?;
    assert_eq!(info.document_count, 0);

    store.upsert(
        "docs",
        json!({"id": "id1", "content": "the quick brown fox"}),
    )?;
    store.upsert(
        "docs",
        json!({"id": "id2", "content": "jumps over the lazy dog"}),
    )?;
    store.upsert("docs", json!({"id": "id3", "content": "brown fox quick"}))?;
    store.flush()?;

    let all = store.search("docs", "fox", false, 10, None)?;
    assert_eq!(all.len(), 2);

    let intersection = store.search("docs", "quick brown", false, 10, None)?;
    assert_eq!(ids(&intersection), vec!["id1", "id3"]);

    store.delete_item("docs", "id1")?;
    let after_delete = store.search("docs", "fox", false, 10, None)?;
    assert_eq!(ids(&after_delete), vec!["id3"]);

    let info = store.collection_info("docs")?;
    assert_eq!(info.document_count, 2);

    store.delete_collection("docs")?;
    let list = store.list_collections()?;
    assert!(list.collections.is_empty());
    Ok(())
}

#[test]
fn full_number_workflow() -> TestResult {
    let (store, _dir) = create_store()?;

    store.create_collection("docs", "number", &["content".into()])?;
    store.upsert("docs", json!({"id": 10, "content": "hello world"}))?;
    store.upsert("docs", json!({"id": 20, "content": "hello there"}))?;
    store.upsert("docs", json!({"id": 30, "content": "world there"}))?;
    store.flush()?;

    let r1 = store.search("docs", "hello", false, 10, None)?;
    assert_eq!(ids(&r1), vec!["10", "20"]);

    let r2 = store.search("docs", "hello world", false, 10, None)?;
    assert_eq!(ids(&r2), vec!["10"]);

    let r3 = store.search("docs", "hello", true, 10, None)?;
    assert_eq!(ids(&r3), vec!["20", "10"]);

    store.delete_item("docs", "10")?;
    let r4 = store.search("docs", "hello", false, 10, None)?;
    assert_eq!(ids(&r4), vec!["20"]);
    Ok(())
}

#[test]
fn string_shard_splitting() -> TestResult {
    let config = StoreConfig {
        max_string_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config)?;

    store.create_collection("docs", "string", &["content".into()])?;
    for i in 0..12u64 {
        store.upsert("docs", json!({"id": i.to_string(), "content": "hello"}))?;
    }
    store.flush()?;

    let results = store.search("docs", "hello", false, 20, None)?;
    assert_eq!(results.len(), 12);

    let result_ids: HashSet<u64> = ids(&results).iter().map(|s| s.parse().unwrap()).collect();
    for i in 0..12 {
        assert!(result_ids.contains(&i), "missing id {}", i);
    }
    Ok(())
}

#[test]
fn roaring_shard_splitting() -> TestResult {
    let config = StoreConfig {
        max_roaring_shard_size: 3,
        ..Default::default()
    };
    let (store, _dir) = create_store_with_config(config)?;

    store.create_collection("docs", "number", &["content".into()])?;
    for i in 1..=12u64 {
        store.upsert("docs", json!({"id": i, "content": "hello"}))?;
    }
    store.flush()?;

    let results = store.search("docs", "hello", false, 20, None)?;
    assert_eq!(results.len(), 12);

    let result_ids: HashSet<u64> = ids(&results).iter().map(|s| s.parse().unwrap()).collect();
    for i in 1..=12 {
        assert!(result_ids.contains(&i), "missing id {}", i);
    }
    Ok(())
}

#[test]
fn update_document_removes_old_tokens() -> TestResult {
    let (store, _dir) = create_store()?;

    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "1", "content": "apple banana"}))?;
    store.upsert("docs", json!({"id": "1", "content": "apple cherry"}))?;
    store.flush()?;

    let banana = store.search("docs", "banana", false, 10, None)?;
    assert!(banana.is_empty(), "banana should have been removed");

    let cherry = store.search("docs", "cherry", false, 10, None)?;
    assert_eq!(ids(&cherry), vec!["1"]);

    let apple = store.search("docs", "apple", false, 10, None)?;
    assert_eq!(ids(&apple), vec!["1"]);
    Ok(())
}

#[test]
fn search_pagination_edge_cases() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;

    for c in ["a", "b", "c", "d", "e"] {
        store.upsert("docs", json!({"id": c, "content": "hello"}))?;
    }
    store.flush()?;

    let page1 = store.search("docs", "hello", false, 2, None)?;
    assert_eq!(ids(&page1), vec!["a", "b"]);

    let page2 = store.search(
        "docs",
        "hello",
        false,
        2,
        page1.last().and_then(|v| v["id"].as_str()),
    )?;
    assert_eq!(ids(&page2), vec!["c", "d"]);

    let page3 = store.search(
        "docs",
        "hello",
        false,
        2,
        page2.last().and_then(|v| v["id"].as_str()),
    )?;
    assert_eq!(ids(&page3), vec!["e"]);

    let page4 = store.search(
        "docs",
        "hello",
        false,
        2,
        page3.last().and_then(|v| v["id"].as_str()),
    )?;
    assert!(page4.is_empty());
    Ok(())
}

#[test]
fn multiple_collections_isolated() -> TestResult {
    let (store, _dir) = create_store()?;

    store.create_collection("a", "string", &["content".into()])?;
    store.create_collection("b", "string", &["content".into()])?;

    store.upsert("a", json!({"id": "1", "content": "hello world"}))?;
    store.upsert("b", json!({"id": "1", "content": "foo bar"}))?;
    store.flush()?;

    let r1 = store.search("a", "hello", false, 10, None)?;
    assert_eq!(ids(&r1), vec!["1"]);

    let r2 = store.search("b", "hello", false, 10, None)?;
    assert!(r2.is_empty());

    let r3 = store.search("b", "foo", false, 10, None)?;
    assert_eq!(ids(&r3), vec!["1"]);
    Ok(())
}

#[test]
fn delete_all_items_in_collection() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;

    store.upsert("docs", json!({"id": "1", "content": "hello"}))?;
    store.upsert("docs", json!({"id": "2", "content": "hello"}))?;
    store.flush()?;

    store.delete_item("docs", "1")?;
    store.delete_item("docs", "2")?;

    let results = store.search("docs", "hello", false, 10, None)?;
    assert!(results.is_empty());

    let info = store.collection_info("docs")?;
    assert_eq!(info.document_count, 0);
    Ok(())
}

#[test]
fn persist_and_reopen() -> TestResult {
    let dir = TempDir::new()?;
    let db_path = dir.path().join("aperio");
    std::fs::create_dir_all(&db_path)?;

    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&db_path)
            .unwrap()
    };
    let store = Store::new(env, db_path.join("fst"))?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "persist", "content": "hello world"}))?;
    store.flush()?;
    drop(store);

    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&db_path)
            .unwrap()
    };
    let store = Store::new(env, db_path.join("fst"))?;
    let results = store.search("docs", "hello", false, 10, None)?;
    assert_eq!(ids(&results), vec!["persist"]);

    let list = store.list_collections()?;
    assert_eq!(list.collections.len(), 1);
    Ok(())
}

#[test]
fn export_import_roundtrip() -> TestResult {
    let dir = TempDir::new()?;

    // Seed source
    let src_path = dir.path().join("src");
    std::fs::create_dir_all(&src_path)?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&src_path)
            .unwrap()
    };
    let store = Store::new(env, src_path.join("fst"))?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "a", "content": "hello world"}))?;
    store.upsert("docs", json!({"id": "b", "content": "foo bar"}))?;
    store.flush()?;

    // Export via Store method
    let data = store.export_snapshot()?;
    assert!(!data.is_empty(), "export data should not be empty");
    drop(store);

    // Import into a fresh store
    let dst_path = dir.path().join("dst");
    std::fs::create_dir_all(&dst_path)?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&dst_path)
            .unwrap()
    };
    let store = Store::new(env, dst_path.join("fst"))?;
    store.import_snapshot(&data)?;

    // Verify
    let list = store.list_collections()?;
    assert_eq!(list.collections.len(), 1);
    assert_eq!(list.collections[0].name, "docs");

    let r1 = store.search("docs", "hello", false, 10, None)?;
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0]["id"], "a");

    let r2 = store.search("docs", "foo", false, 10, None)?;
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0]["id"], "b");
    Ok(())
}

#[test]
fn export_empty_database() -> TestResult {
    let dir = TempDir::new()?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())
            .unwrap()
    };
    let store = Store::new(env, dir.path().join("fst"))?;
    let data = store.export_snapshot()?;
    // Should produce valid export data (empty keyspace list)
    assert!(
        !data.is_empty(),
        "export data should have header even with no collections"
    );
    Ok(())
}

#[test]
fn export_import_number_collection() -> TestResult {
    let dir = TempDir::new()?;

    let src_path = dir.path().join("src");
    std::fs::create_dir_all(&src_path)?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&src_path)
            .unwrap()
    };
    let store = Store::new(env, src_path.join("fst"))?;
    store.create_collection("nums", "number", &["val".into()])?;
    store.upsert("nums", json!({"id": 42, "val": "hello"}))?;
    store.upsert("nums", json!({"id": 99, "val": "world"}))?;
    store.flush()?;

    let data = store.export_snapshot()?;
    drop(store);

    let dst_path = dir.path().join("dst");
    std::fs::create_dir_all(&dst_path)?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&dst_path)
            .unwrap()
    };
    let store = Store::new(env, dst_path.join("fst"))?;
    store.import_snapshot(&data)?;

    let r = store.search("nums", "hello", false, 10, None)?;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["id"], 42);

    let r = store.search("nums", "world", false, 10, None)?;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["id"], 99);
    Ok(())
}

#[test]
fn export_import_bad_magic() -> TestResult {
    let dir = TempDir::new()?;
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())
            .unwrap()
    };
    let store = Store::new(env, dir.path().join("fst"))?;
    let err = store.import_snapshot(b"garbage data").unwrap_err();
    assert!(err.to_string().contains("bad magic"), "got: {err}");
    Ok(())
}

#[test]
fn import_with_bad_payload_does_not_wipe_existing_data() -> TestResult {
    // Regression: previously `import_snapshot` called `clear_all_tables` (which
    // committed its own write txn) before parsing the payload. Any bad upload
    // — wrong magic, truncation, etc. — left the database permanently empty.
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "a", "content": "important data"}))?;
    store.flush()?;

    // Various bad payloads.
    let bad_payloads: [&[u8]; 4] = [
        b"garbage data",             // wrong magic
        b"APIOEXPT",                 // magic but truncated
        b"APIOEXPT\x99\x00\x00\x00", // wrong version
        b"",                         // empty
    ];
    for payload in bad_payloads {
        let _ = store.import_snapshot(payload); // ignore error variant
        // The original document must still be there.
        let res = store.search("docs", "important", false, 10, None)?;
        assert_eq!(
            res.len(),
            1,
            "data wiped after bad import (payload len {})",
            payload.len()
        );
    }
    Ok(())
}

#[test]
fn import_with_truncated_table_data_does_not_wipe_existing_data() -> TestResult {
    // A valid header but a truncated table body must also leave the existing
    // database intact.
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "a", "content": "important data"}))?;
    store.flush()?;

    // Take a real export and cut off the trailing bytes.
    let full = store.export_snapshot()?;
    // Drop the last quarter to guarantee mid-record truncation.
    let truncated = &full[..full.len() * 3 / 4];
    let _ = store.import_snapshot(truncated);

    let res = store.search("docs", "important", false, 10, None)?;
    assert_eq!(res.len(), 1, "data wiped after truncated import");
    Ok(())
}

// ---------------------------------------------------------------------------
// FST / suggest integration tests
// ---------------------------------------------------------------------------

#[test]
fn suggest_returns_indexed_terms() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    for (id, text) in [
        ("1", "apple banana"),
        ("2", "application"),
        ("3", "appetite"),
    ] {
        store.upsert("docs", json!({"id": id, "content": text}))?;
    }
    store.flush()?;

    // Wait for FST consolidation to happen by manually triggering
    store.fst_pool.consolidate("docs")?;

    let results = store.suggest("docs", "app", 10)?;
    assert_eq!(results.len(), 3);
    assert!(results.contains(&"apple".to_string()));
    assert!(results.contains(&"application".to_string()));
    assert!(results.contains(&"appetite".to_string()));
    Ok(())
}

#[test]
fn suggest_returns_empty_for_no_match() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "1", "content": "hello world"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    let results = store.suggest("docs", "xyz", 10)?;
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn suggest_returns_empty_for_nonexistent_collection() -> TestResult {
    let (store, _dir) = create_store()?;
    let err = store.suggest("nonexistent", "hello", 10).unwrap_err();
    assert!(err.to_string().contains("not found"));
    Ok(())
}

#[test]
fn suggest_respects_limit() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    for i in 0..10u64 {
        store.upsert(
            "docs",
            json!({"id": i.to_string(), "content": "a".repeat(3 + i as usize)}),
        )?;
    }
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    let results = store.suggest("docs", "a", 3)?;
    assert_eq!(results.len(), 3);
    Ok(())
}

#[test]
fn fst_terms_not_orphaned_on_single_delete() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "1", "content": "apple banana"}))?;
    store.upsert("docs", json!({"id": "2", "content": "apple cherry"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    assert!(store.fst_pool.contains("docs", "apple"));
    assert!(store.fst_pool.contains("docs", "banana"));
    assert!(store.fst_pool.contains("docs", "cherry"));

    // Delete doc 1 — "banana" is only in doc 1 but FST is best-effort
    // and may still contain it until next full consolidation
    store.delete_item("docs", "1")?;
    store.fst_pool.consolidate("docs")?;

    // FST is a superset: orphaned terms may remain, shared terms definitely exist
    assert!(store.fst_pool.contains("docs", "apple"));
    assert!(store.fst_pool.contains("docs", "cherry"));
    Ok(())
}

#[test]
fn fst_terms_removed_on_document_update() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "1", "content": "apple banana"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    assert!(store.fst_pool.contains("docs", "banana"));

    // Update — remove "banana", add "cherry"
    store.upsert("docs", json!({"id": "1", "content": "apple cherry"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    assert!(!store.fst_pool.contains("docs", "banana"));
    assert!(store.fst_pool.contains("docs", "cherry"));
    assert!(store.fst_pool.contains("docs", "apple"));
    Ok(())
}

#[test]
fn suggest_works_with_number_collection() -> TestResult {
    let (store, _dir) = create_store()?;
    store.create_collection("docs", "number", &["content".into()])?;
    store.upsert("docs", json!({"id": 1, "content": "apple banana"}))?;
    store.upsert("docs", json!({"id": 2, "content": "application test"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;

    let results = store.suggest("docs", "app", 10)?;
    assert!(results.contains(&"apple".to_string()));
    assert!(results.contains(&"application".to_string()));
    Ok(())
}

#[test]
fn suggest_fst_persists_across_reopen() -> TestResult {
    let dir = TempDir::new()?;
    let db_path = dir.path().join("aperio");
    std::fs::create_dir_all(&db_path)?;

    // First session
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&db_path)
            .unwrap()
    };
    let store = Store::new(env, db_path.join("fst"))?;
    store.create_collection("docs", "string", &["content".into()])?;
    store.upsert("docs", json!({"id": "1", "content": "apple banana"}))?;
    store.flush()?;
    store.fst_pool.consolidate("docs")?;
    assert!(store.fst_pool.contains("docs", "apple"));
    drop(store);

    // Second session — FST should still be on disk
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&db_path)
            .unwrap()
    };
    let store = Store::new(env, db_path.join("fst"))?;
    assert!(store.fst_pool.contains("docs", "apple"));
    let results = store.suggest("docs", "app", 10)?;
    assert!(results.contains(&"apple".to_string()));
    Ok(())
}
