use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_defaults() {
        let json = r#"{"q":"test"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.q, "test");
        assert!(params.sort.is_none());
        assert!(params.take.is_none());
        assert!(params.after.is_none());
    }

    #[test]
    fn search_params_full() {
        let json = r#"{"q":"hello","sort":"asc","take":5,"after":"10"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.q, "hello");
        assert_eq!(params.sort.as_deref(), Some("asc"));
        assert_eq!(params.take, Some(5));
        assert_eq!(params.after.as_deref(), Some("10"));
    }

    #[test]
    fn search_response_serialize() {
        let resp = SearchResponse {
            results: vec![
                serde_json::json!({"id": "a"}),
                serde_json::json!({"id": "b"}),
            ],
            take: 2,
            elapsed_ms: 0.0,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"results":[{"id":"a"},{"id":"b"}],"take":2,"elapsed_ms":0.0}"#
        );
    }

    #[test]
    fn search_response_empty() {
        let resp: SearchResponse = SearchResponse {
            results: vec![],
            take: 0,
            elapsed_ms: 1.5,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"results":[],"take":0,"elapsed_ms":1.5}"#);
    }

    #[test]
    fn create_collection_request_roundtrip() {
        let json = r#"{"name":"mycol","id_type":"string","searchable_fields":["title","body"]}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "mycol");
        assert_eq!(req.id_type, "string");
        assert_eq!(req.searchable_fields, vec!["title", "body"]);
    }

    #[test]
    fn collection_info_serialize() {
        let info = CollectionInfo {
            name: "c".into(),
            id_type: "string".into(),
            document_count: 10,
            searchable_fields: vec!["title".into()],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(
            json,
            r#"{"name":"c","id_type":"string","document_count":10,"searchable_fields":["title"]}"#
        );
    }

    #[test]
    fn status_response_serialize() {
        let json = serde_json::to_string(&StatusResponse { ok: true }).unwrap();
        assert_eq!(json, r#"{"ok":true}"#);
    }

    #[test]
    fn list_collections_response_serialize() {
        let resp = ListCollectionsResponse {
            collections: vec![CollectionSummary {
                name: "a".into(),
                id_type: "number".into(),
                searchable_fields: vec!["body".into()],
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            r#"{"collections":[{"name":"a","id_type":"number","searchable_fields":["body"]}]}"#
        );
    }

    #[test]
    fn create_collection_request_fields_default() {
        let json = r#"{"name":"mycol","id_type":"string","searchable_fields":[]}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert!(req.searchable_fields.is_empty());
    }
}

#[derive(Deserialize)]
pub struct BackupFile {
    pub name: String,
}

#[derive(Serialize)]
pub struct ExportResponse {
    pub ok: bool,
    pub size: u64,
    pub file: String,
}

#[derive(Serialize)]
pub struct ImportResponse {
    pub ok: bool,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub sort: Option<String>,
    pub take: Option<usize>,
    pub after: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<serde_json::Value>,
    pub take: usize,
    pub elapsed_ms: f64,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct QueueDepthResponse {
    pub pending: u64,
}

#[derive(Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub id_type: String,
    pub searchable_fields: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct CollectionCreated {
    pub name: String,
    pub id_type: String,
    pub searchable_fields: Vec<String>,
}

#[derive(Serialize)]
pub struct CollectionInfo {
    pub name: String,
    pub id_type: String,
    pub document_count: usize,
    pub searchable_fields: Vec<String>,
}

#[derive(Serialize)]
pub struct CollectionSummary {
    pub name: String,
    pub id_type: String,
    pub searchable_fields: Vec<String>,
}

#[derive(Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionSummary>,
}
