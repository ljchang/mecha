//! The P3 page's read API. Each response is the owner's CLI read model.
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

type St = State<super::WebState>;

pub async fn list(State(state): St) -> Response {
    read(&state, &["mail", "rule", "list", "--json"]).await
}

pub async fn show(State(state): St, Path(name): Path<String>) -> Response {
    read(&state, &["mail", "rule", "show", "--json", "--", &name]).await
}

#[derive(serde::Deserialize)]
pub struct LedgerQuery {
    rule: Option<String>,
    limit: Option<usize>,
}

pub async fn ledger(State(state): St, Query(query): Query<LedgerQuery>) -> Response {
    let limit = query.limit.unwrap_or(50).min(1000).to_string();
    let mut args = vec!["mail", "rule", "ledger", "--json", "--limit", &limit];
    if let Some(rule) = &query.rule {
        args.extend(["--", rule]);
    }
    read(&state, &args).await
}

async fn read(state: &super::WebState, args: &[&str]) -> Response {
    match super::review::self_json(state, args).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, format!("{error:#}\n")).into_response(),
    }
}
