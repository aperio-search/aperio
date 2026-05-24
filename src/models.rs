use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
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
    pub total: usize,
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
