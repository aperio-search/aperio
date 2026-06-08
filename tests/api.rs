use std::sync::Arc;

use aperio::routes;
use aperio::store::Store;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

fn test_app() -> (Router, TempDir, Arc<Store>) {
    let dir = TempDir::new().unwrap();
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())
            .unwrap()
    };
    let store = Arc::new(Store::new(env, dir.path().join("fst")));
    let auth = aperio::auth::AuthConfig::default();
    let dumps = dir.path().join("dumps");
    std::fs::create_dir_all(&dumps).unwrap();
    (
        routes::create_router(store.clone(), auth, Some(dumps)),
        dir,
        store,
    )
}

fn test_app_no_dumps() -> (Router, TempDir, Arc<Store>) {
    let dir = TempDir::new().unwrap();
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(dir.path())
            .unwrap()
    };
    let store = Arc::new(Store::new(env, dir.path().join("fst")));
    let auth = aperio::auth::AuthConfig::default();
    (routes::create_router(store.clone(), auth, None), dir, store)
}

fn json_request(method: Method, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", "SecretApiKey")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", "SecretApiKey")
        .body(Body::empty())
        .unwrap()
}

fn delete_request(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header("authorization", "SecretApiKey")
        .body(Body::empty())
        .unwrap()
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let body = resp.into_body();
    let collected = BodyExt::collect(body).await.unwrap();
    let bytes = collected.to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, value)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    read_body(app.clone().oneshot(req).await.unwrap()).await
}

fn noauth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn search_key_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", "PublicApiKey")
        .body(Body::empty())
        .unwrap()
}

fn search_key_json(method: Method, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", "PublicApiKey")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn status_is_public() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, noauth_get("/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true}));
}

#[tokio::test]
async fn missing_auth_returns_401() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, noauth_get("/collections")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn invalid_auth_returns_401() {
    let (app, _dir, _store) = test_app();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/collections")
        .header("authorization", "WrongKey")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn search_key_can_search() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": "1", "content": "hello world"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    let (_status, body) = send(&app, search_key_get("/collections/docs/search?q=hello")).await;
    assert_eq!(
        body["results"],
        json!([{"id": "1", "content": "hello world"}])
    );
}

#[tokio::test]
async fn search_key_cannot_admin() {
    let (app, _dir, _store) = test_app();

    let req = search_key_json(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _body) = send(&app, search_key_get("/collections")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn search_key_cannot_access_collection_named_search() {
    // Regression test: previously check_auth used path.ends_with("/search"),
    // so a collection literally named "search" was reachable with the public key.
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "search", "id_type": "string", "searchable_fields": ["content"]}),
    );
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = send(&app, search_key_get("/collections/search")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/collections/search")
        .header("authorization", "PublicApiKey")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn search_key_cannot_delete_item_named_search() {
    // Regression test: path.ends_with("/suggest") would have matched item id "suggest".
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/collections/docs/items/suggest")
        .header("authorization", "PublicApiKey")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_status() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, get_request("/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true}));
}

#[tokio::test]
async fn get_empty_collections() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, get_request("/collections")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"collections": []}));
}

#[tokio::test]
async fn create_collection() {
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "testcol", "id_type": "string", "searchable_fields": []}),
    );
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        body,
        json!({"name": "testcol", "id_type": "string", "searchable_fields": []})
    );
}

#[tokio::test]
async fn create_duplicate_collection() {
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "testcol", "id_type": "string", "searchable_fields": []}),
    );
    send(&app, req).await;
    let req2 = json_request(
        Method::POST,
        "/collections",
        json!({"name": "testcol", "id_type": "string", "searchable_fields": []}),
    );
    let (status, body) = send(&app, req2).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn create_collection_invalid_id_type() {
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "testcol", "id_type": "invalid", "searchable_fields": []}),
    );
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("invalid id_type"));
}

#[tokio::test]
async fn search_rejects_empty_q() {
    // Regression: previously an empty `q` returned 200 with empty results,
    // hiding a likely client error. The contract is `q` is required.
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for path in [
        "/collections/docs/search?q=",
        "/collections/docs/search?q=%20%20",
    ] {
        let (status, body) = send(&app, get_request(path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path={path}");
        assert!(
            body["error"]
                .as_str()
                .map(|s| s.contains("q"))
                .unwrap_or(false),
            "body should reference the q param: {body:?}"
        );
    }
}

#[tokio::test]
async fn suggest_rejects_empty_q() {
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for path in [
        "/collections/docs/suggest?q=",
        "/collections/docs/suggest?q=%20",
    ] {
        let (status, _body) = send(&app, get_request(path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path={path}");
    }
}

#[tokio::test]
async fn upsert_and_search() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": "1", "content": "hello world"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(
        body["results"],
        json!([{"id": "1", "content": "hello world"}])
    );
    assert_eq!(body["take"], 20);
}

#[tokio::test]
async fn search_with_pagination() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for id in &["a", "b", "c"] {
        let req = json_request(
            Method::POST,
            "/collections/docs/items",
            json!({"id": id, "content": "hello"}),
        );
        send(&app, req).await;
    }
    store.flush().unwrap();

    let (_status, page1) = send(
        &app,
        get_request("/collections/docs/search?q=hello&take=2&sort=asc"),
    )
    .await;
    assert_eq!(
        page1["results"],
        json!([{"id": "a", "content": "hello"}, {"id": "b", "content": "hello"}])
    );

    let after = page1["results"][1]["id"].as_str().unwrap();
    let (_status, page2) = send(
        &app,
        get_request(&format!(
            "/collections/docs/search?q=hello&take=2&sort=asc&after={}",
            after
        )),
    )
    .await;
    assert_eq!(page2["results"], json!([{"id": "c", "content": "hello"}]));
}

#[tokio::test]
async fn search_sort_asc() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for id in &["b", "a"] {
        let req = json_request(
            Method::POST,
            "/collections/docs/items",
            json!({"id": id, "content": "hello"}),
        );
        send(&app, req).await;
    }
    store.flush().unwrap();

    let (_status, body) = send(
        &app,
        get_request("/collections/docs/search?q=hello&sort=asc"),
    )
    .await;
    assert_eq!(
        body["results"],
        json!([{"id": "a", "content": "hello"}, {"id": "b", "content": "hello"}])
    );
}

#[tokio::test]
async fn search_sort_desc() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for id in &["a", "b"] {
        let req = json_request(
            Method::POST,
            "/collections/docs/items",
            json!({"id": id, "content": "hello"}),
        );
        send(&app, req).await;
    }
    store.flush().unwrap();

    let (_status, body) = send(
        &app,
        get_request("/collections/docs/search?q=hello&sort=desc"),
    )
    .await;
    assert_eq!(
        body["results"],
        json!([{"id": "b", "content": "hello"}, {"id": "a", "content": "hello"}])
    );
}

#[tokio::test]
async fn delete_item_endpoint() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": "1", "content": "hello"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    send(&app, delete_request("/collections/docs/items/1")).await;

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"], json!([]));
}

#[tokio::test]
async fn delete_nonexistent_item() {
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let (status, _body) = send(&app, delete_request("/collections/docs/items/1")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn collection_info_endpoint() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": "1", "content": "hello world"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    let (_status, body) = send(&app, get_request("/collections/docs")).await;
    assert_eq!(body["name"], "docs");
    assert_eq!(body["id_type"], "string");
    assert_eq!(body["document_count"], 1);
    assert_eq!(body["searchable_fields"], json!(["content"]));
}

#[tokio::test]
async fn delete_collection_endpoint() {
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": []}),
    );
    send(&app, req).await;
    send(&app, delete_request("/collections/docs")).await;

    let resp = send(&app, get_request("/collections")).await;
    assert_eq!(resp.1["collections"], json!([]));
}

#[tokio::test]
async fn not_found_fallback() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, get_request("/nonexistent")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not found");
}

#[tokio::test]
async fn search_nonexistent_collection() {
    let (app, _dir, _store) = test_app();
    let (status, body) = send(&app, get_request("/collections/nope/search?q=hello")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn upsert_nonexistent_collection() {
    let (app, _dir, _store) = test_app();
    let req = json_request(
        Method::POST,
        "/collections/nope/items",
        json!({"id": "1", "content": "hello"}),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_collections_after_create() {
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "a", "id_type": "string", "searchable_fields": []}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "b", "id_type": "number", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let (_status, body) = send(&app, get_request("/collections")).await;
    let cols = body["collections"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
    let names: Vec<&str> = cols.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"b"));
}

#[tokio::test]
async fn create_collection_missing_fields() {
    let (app, _dir, _store) = test_app();
    let req = json_request(Method::POST, "/collections", json!({}));
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upsert_with_numeric_id() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "number", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": 42, "content": "hello world"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(
        body["results"],
        json!([{"id": 42, "content": "hello world"}])
    );
}

// ---------------------------------------------------------------------------
// Bulk ingest endpoint tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bulk_ingest_string() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items/bulk",
        json!([
            {"id": "a", "content": "hello world"},
            {"id": "b", "content": "foo bar"},
            {"id": "c", "content": "hello bar"},
        ]),
    );
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["count"], 3);

    store.flush().unwrap();
    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn bulk_ingest_number() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "number", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items/bulk",
        json!([
            {"id": 1, "content": "hello"},
            {"id": 2, "content": "world"},
        ]),
    );
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);

    store.flush().unwrap();
    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn bulk_ingest_nonexistent_collection() {
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections/nope/items/bulk",
        json!([{"id": "1", "content": "hello"}]),
    );
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_ingest_missing_id() {
    let (app, _dir, _store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items/bulk",
        json!([{"content": "hello"}]),
    );
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_ingest_empty() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(Method::POST, "/collections/docs/items/bulk", json!([]));
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);

    store.flush().unwrap();
    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn bulk_ingest_requires_main_key() {
    let (app, _dir, _store) = test_app();

    let req = search_key_json(
        Method::POST,
        "/collections/docs/items/bulk",
        json!([{"id": "1", "content": "hello"}]),
    );
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Backup endpoint tests
// ---------------------------------------------------------------------------

fn main_key_post(path: &str, body: Value) -> Request<Body> {
    json_request(Method::POST, path, body)
}

fn empty_post(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", "SecretApiKey")
        .body(Body::from("{}"))
        .unwrap()
}

#[tokio::test]
async fn export_endpoint_main_key() {
    let (app, dir, store) = test_app();

    let req = main_key_post(
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;
    let req = main_key_post(
        "/collections/docs/items",
        json!({"id": "1", "content": "hello world"}),
    );
    send(&app, req).await;
    store.flush().unwrap();

    // Export (no body — auto-generated filename)
    let req = empty_post("/backup/export");
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert!(body["size"].as_u64().unwrap_or(0) > 0);
    let file = body["file"].as_str().unwrap().to_string();
    assert!(
        file.ends_with(".aperio"),
        "expected '.aperio' extension, got {file}"
    );

    // Verify the file exists in the dumps folder
    let dumps_path = dir.path().join("dumps").join(&file);
    assert!(
        dumps_path.exists(),
        "export file should exist at {dumps_path:?}"
    );

    // Verify the file can be imported into a fresh store
    let import_dir = TempDir::new().unwrap();
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(import_dir.path())
            .unwrap()
    };
    let store = Store::new(env, import_dir.path().join("fst"));
    let data = std::fs::read(&dumps_path).unwrap();
    store.import_snapshot(&data).unwrap();

    let list = store.list_collections().unwrap();
    assert_eq!(list.collections.len(), 1);
    assert_eq!(list.collections[0].name, "docs");
}

#[tokio::test]
async fn export_and_import_roundtrip_via_endpoint() {
    let dir = TempDir::new().unwrap();
    let dumps = dir.path().join("dumps");
    std::fs::create_dir_all(&dumps).unwrap();

    // First app (source)
    let src_path = dir.path().join("src");
    std::fs::create_dir_all(&src_path).unwrap();
    let env1 = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&src_path)
            .unwrap()
    };
    let store1 = Arc::new(Store::new(env1, src_path.join("fst")));
    let auth1 = aperio::auth::AuthConfig::default();
    let app1 = routes::create_router(store1.clone(), auth1, Some(dumps.clone()));

    let req = main_key_post(
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app1, req).await;
    let req = main_key_post(
        "/collections/docs/items",
        json!({"id": "a", "content": "hello world"}),
    );
    send(&app1, req).await;
    store1.flush().unwrap();

    // Export
    let req = empty_post("/backup/export");
    let (status, body) = send(&app1, req).await;
    assert_eq!(status, StatusCode::OK);
    let file = body["file"].as_str().unwrap().to_string();

    // Second app (destination) — uses same dumps folder
    let dst_path = dir.path().join("dst");
    std::fs::create_dir_all(&dst_path).unwrap();
    let env2 = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024)
            .max_dbs(4)
            .open(&dst_path)
            .unwrap()
    };
    let store2 = Arc::new(Store::new(env2, dst_path.join("fst")));
    let auth2 = aperio::auth::AuthConfig::default();
    let app2 = routes::create_router(store2, auth2, Some(dumps));

    let req = main_key_post("/backup/import", json!({"name": file}));
    let (status, body) = send(&app2, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);

    let (_status, body) = send(&app2, get_request("/collections")).await;
    assert_eq!(body["collections"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn export_requires_main_key() {
    let (app, _dir, _store) = test_app();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/backup/export")
        .header("content-type", "application/json")
        .header("authorization", "PublicApiKey")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn import_requires_main_key() {
    let (app, _dir, _store) = test_app();
    let req = search_key_json(Method::POST, "/backup/import", json!({"name": "any.bin"}));
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn import_nonexistent_file_returns_error() {
    let (app, _dir, _store) = test_app();
    let req = main_key_post("/backup/import", json!({"name": "nope.aperio"}));
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn import_rejects_path_traversal() {
    // Regression: previously `dumps.join(&name)` accepted "../" segments,
    // letting a caller read (and then attempt to import) any file the
    // process could open.
    let (app, _dir, _store) = test_app();
    for bad in [
        "../etc/passwd",
        "../../etc/passwd",
        "/etc/passwd",
        "foo/bar.aperio",
        "foo\\bar.aperio",
        "..",
    ] {
        let req = main_key_post("/backup/import", json!({ "name": bad }));
        let (status, _body) = send(&app, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "name={bad}");
    }
}

#[tokio::test]
async fn export_without_dumps_folder_returns_error() {
    let (app, _dir, _store) = test_app_no_dumps();
    let req = main_key_post("/backup/export", json!({}));
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn import_without_dumps_folder_returns_error() {
    let (app, _dir, _store) = test_app_no_dumps();
    let req = main_key_post("/backup/import", json!({"name": "any.bin"}));
    let (status, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Suggest endpoint tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn suggest_returns_terms() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    for (id, text) in [("1", "apple banana"), ("2", "application")] {
        let req = json_request(
            Method::POST,
            "/collections/docs/items",
            json!({"id": id, "content": text}),
        );
        send(&app, req).await;
    }
    store.flush().unwrap();
    store.fst_pool.consolidate("docs").unwrap();

    let (_status, body) = send(&app, get_request("/collections/docs/suggest?q=app")).await;
    let results = body["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r == "apple"));
    assert!(results.iter().any(|r| r == "application"));
}

#[tokio::test]
async fn suggest_returns_empty_with_no_match() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    store
        .upsert("docs", json!({"id": "1", "content": "hello world"}))
        .unwrap();
    store.flush().unwrap();
    store.fst_pool.consolidate("docs").unwrap();

    let (_status, body) = send(&app, get_request("/collections/docs/suggest?q=xyz")).await;
    assert_eq!(body["results"], json!([]));
}

#[tokio::test]
async fn suggest_on_nonexistent_collection() {
    let (app, _dir, _store) = test_app();
    let (status, _body) = send(&app, get_request("/collections/nope/suggest?q=hello")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn suggest_with_search_key() {
    let (app, _dir, store) = test_app();

    let req = json_request(
        Method::POST,
        "/collections",
        json!({"name": "docs", "id_type": "string", "searchable_fields": ["content"]}),
    );
    send(&app, req).await;

    let req = json_request(
        Method::POST,
        "/collections/docs/items",
        json!({"id": "1", "content": "apple banana"}),
    );
    send(&app, req).await;
    store.flush().unwrap();
    store.fst_pool.consolidate("docs").unwrap();

    let (_status, body) = send(&app, search_key_get("/collections/docs/suggest?q=app")).await;
    let results = body["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r == "apple"));
}

#[tokio::test]
async fn suggest_requires_auth() {
    let (app, _dir, _store) = test_app();
    let (status, _body) = send(&app, noauth_get("/collections/docs/suggest?q=hello")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
