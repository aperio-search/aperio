use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_request_roundtrip() {
        let json = r#"{"id":"doc1","content":"hello world"}"#;
        let req: UpsertRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, "doc1");
        assert_eq!(req.content, "hello world");
        let _ = serde_json::to_string(&UpsertRequest {
            id: "x".into(),
            content: "y".into(),
        });
    }

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
            results: vec!["a".into(), "b".into()],
            take: 2,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"results":["a","b"],"take":2}"#);
    }

    #[test]
    fn search_response_empty() {
        let resp: SearchResponse = SearchResponse {
            results: vec![],
            take: 0,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"results":[],"take":0}"#);
    }

    #[test]
    fn create_collection_request_roundtrip() {
        let json = r#"{"name":"mycol","id_type":"string"}"#;
        let req: CreateCollectionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "mycol");
        assert_eq!(req.id_type, "string");
    }

    #[test]
    fn collection_info_serialize() {
        let info = CollectionInfo {
            name: "c".into(),
            id_type: "string".into(),
            document_count: 10,
            unique_terms: 42,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(
            json,
            r#"{"name":"c","id_type":"string","document_count":10,"unique_terms":42}"#
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
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"collections":[{"name":"a","id_type":"number"}]}"#);
    }

    #[test]
    fn suggest_params_deserialize() {
        let json = r#"{"q":"hel"}"#;
        let params: SuggestParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.q, "hel");
    }

    #[test]
    fn suggest_response_serialize() {
        let resp = SuggestResponse {
            suggestions: vec!["hello".into(), "help".into()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"suggestions":["hello","help"]}"#);
    }
}

#[derive(Serialize, Deserialize)]
pub struct UpsertRequest {
    pub id: String,
    pub content: String,
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
    pub results: Vec<String>,
    pub take: usize,
}

#[derive(Deserialize)]
pub struct SuggestParams {
    pub q: String,
}

#[derive(Serialize)]
pub struct SuggestResponse {
    pub suggestions: Vec<String>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub ok: bool,
}

#[derive(Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub id_type: String,
}

#[derive(Serialize)]
#[derive(Debug)]
pub struct CollectionCreated {
    pub name: String,
    pub id_type: String,
}

#[derive(Serialize)]
pub struct CollectionInfo {
    pub name: String,
    pub id_type: String,
    pub document_count: usize,
    pub unique_terms: usize,
}

#[derive(Serialize)]
pub struct CollectionSummary {
    pub name: String,
    pub id_type: String,
}

#[derive(Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionSummary>,
}
