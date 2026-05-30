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

fn test_app() -> (Router, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = fjall::Database::builder(dir.path())
        .cache_size(1_000_000)
        .open()
        .unwrap();
    let store = Arc::new(Store::new(db));
    (routes::create_router(store), dir)
}

fn json_request(method: Method, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn delete_request(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
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

#[tokio::test]
async fn get_status() {
    let (app, _dir) = test_app();
    let (status, body) = send(&app, get_request("/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true}));
}

#[tokio::test]
async fn get_empty_collections() {
    let (app, _dir) = test_app();
    let (status, body) = send(&app, get_request("/collections")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"collections": []}));
}

#[tokio::test]
async fn create_collection() {
    let (app, _dir) = test_app();
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
    let (app, _dir) = test_app();
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
    let (app, _dir) = test_app();
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
async fn upsert_and_search() {
    let (app, _dir) = test_app();

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

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"], json!([{"id": "1", "content": "hello world"}]));
    assert_eq!(body["take"], 20);
}

#[tokio::test]
async fn search_with_pagination() {
    let (app, _dir) = test_app();

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
    let (app, _dir) = test_app();

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
    let (app, _dir) = test_app();

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
async fn suggest_endpoint() {
    let (app, _dir) = test_app();

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

    let (_status, body) = send(&app, get_request("/collections/docs/suggest?q=hel")).await;
    assert!(
        body["suggestions"]
            .as_array()
            .unwrap()
            .contains(&json!("hello"))
    );
}

#[tokio::test]
async fn delete_item_endpoint() {
    let (app, _dir) = test_app();

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

    send(&app, delete_request("/collections/docs/items/1")).await;

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(body["results"], json!([]));
}

#[tokio::test]
async fn delete_nonexistent_item() {
    let (app, _dir) = test_app();

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
    let (app, _dir) = test_app();

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

    let (_status, body) = send(&app, get_request("/collections/docs")).await;
    assert_eq!(body["name"], "docs");
    assert_eq!(body["id_type"], "string");
    assert_eq!(body["document_count"], 1);
    assert_eq!(body["unique_terms"], 2);
    assert_eq!(body["searchable_fields"], json!(["content"]));
}

#[tokio::test]
async fn delete_collection_endpoint() {
    let (app, _dir) = test_app();

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
    let (app, _dir) = test_app();
    let (status, body) = send(&app, get_request("/nonexistent")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not found");
}

#[tokio::test]
async fn search_nonexistent_collection() {
    let (app, _dir) = test_app();
    let (status, body) = send(&app, get_request("/collections/nope/search?q=hello")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn upsert_nonexistent_collection() {
    let (app, _dir) = test_app();
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
    let (app, _dir) = test_app();

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
    let (app, _dir) = test_app();
    let req = json_request(Method::POST, "/collections", json!({}));
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upsert_with_numeric_id() {
    let (app, _dir) = test_app();

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

    let (_status, body) = send(&app, get_request("/collections/docs/search?q=hello")).await;
    assert_eq!(
        body["results"],
        json!([{"id": 42, "content": "hello world"}])
    );
}
