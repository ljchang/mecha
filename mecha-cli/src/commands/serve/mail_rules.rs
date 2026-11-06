//! The P3 page's API. Reads and mutations delegate to the owner's CLI.
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

// The provider enumerates pages and reads individual messages, each with a
// 30-second HTTP deadline. Allow ten minutes for the 30-day all-folder search;
// this is a bounded interactive request, not a promise every mailbox fits.
const TEST_SECONDS: u64 = 600;

/// A rule draft's body: the 1 MiB stdin budget `mecha mail rule` reads, so a
/// larger draft is refused here rather than read into serve and piped.
pub const MAX_DRAFT_BODY: usize = 1 << 20;

#[derive(serde::Deserialize)]
pub struct Draft {
    name: String,
    rule_toml: String,
}
#[derive(serde::Deserialize)]
pub struct Edit {
    rule_toml: String,
}
#[derive(serde::Deserialize)]
pub struct ModeBody {
    group: String,
    mode: String,
    digest: Option<mecha_core::mail_rules::ApprovalDigest>,
}
#[derive(serde::Deserialize)]
pub struct RowBody {
    row_id: String,
}
#[derive(serde::Deserialize)]
pub struct TestBody {
    account: Option<String>,
}

pub async fn add(State(state): St, Json(body): Json<Draft>) -> Response {
    // `--name=` binds the value whatever it begins with; the store validates it.
    let name = format!("--name={}", body.name);
    mutation(
        &state,
        &["mail", "rule", "add", "--stdin", "--json", &name],
        Some(&body.rule_toml),
        30,
    )
    .await
}
pub async fn edit(State(state): St, Path(name): Path<String>, Json(body): Json<Edit>) -> Response {
    mutation(
        &state,
        &["mail", "rule", "edit", "--stdin", "--json", "--", &name],
        Some(&body.rule_toml),
        30,
    )
    .await
}
pub async fn mode(
    State(state): St,
    Path(name): Path<String>,
    Json(body): Json<ModeBody>,
) -> Response {
    let digest = body.digest.as_ref().map(serde_json::to_string).transpose();
    let digest = match digest {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let mut args = vec![
        "mail",
        "rule",
        "mode",
        "--json",
        "--yes",
        "--require-digest",
    ];
    if let Some(digest) = &digest {
        args.extend(["--digest", digest]);
    }
    args.extend(["--", &name, &body.group, &body.mode]);
    mutation(&state, &args, None, 30).await
}
pub async fn pause(
    State(state): St,
    Path(name): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> Response {
    mutation(
        &state,
        &["mail", "rule", "pause", "--json", "--", &name],
        None,
        30,
    )
    .await
}
pub async fn pause_all(State(state): St, Json(_body): Json<serde_json::Value>) -> Response {
    mutation(&state, &["mail", "rule", "pause-all", "--json"], None, 30).await
}
pub async fn undo(State(state): St, Json(body): Json<RowBody>) -> Response {
    mutation(
        &state,
        &["mail", "rule", "undo", "--", &body.row_id],
        None,
        120,
    )
    .await
}
pub async fn wrong_match(State(state): St, Json(body): Json<RowBody>) -> Response {
    mutation(
        &state,
        &["mail", "rule", "wrong-match", "--json", "--", &body.row_id],
        None,
        30,
    )
    .await
}
pub async fn test(
    State(state): St,
    Path(name): Path<String>,
    Json(body): Json<TestBody>,
) -> Response {
    let account = body.account.as_ref().map(|a| format!("--account={a}"));
    let mut args = vec!["mail", "rule", "test", "--json"];
    if let Some(account) = &account {
        args.push(account);
    }
    args.extend(["--", &name]);
    mutation(&state, &args, None, TEST_SECONDS).await
}

async fn mutation(
    state: &super::WebState,
    args: &[&str],
    input: Option<&str>,
    seconds: u64,
) -> Response {
    match super::review::self_json_input_within(state, args, seconds, input).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => {
            if let Some(child) = error.downcast_ref::<super::review::JsonChildFailure>() {
                let mut value = child
                    .output
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                if !value.is_object() {
                    value = serde_json::json!({"report": value});
                }
                value["ok"] = serde_json::json!(false);
                if value.get("error").is_none() {
                    value["error"] =
                        serde_json::json!({"code": "child_failed", "message": child.message});
                }
                value["stderr"] = serde_json::json!(child.message);
                (StatusCode::CONFLICT, Json(value)).into_response()
            } else {
                let status = if error.to_string() == "timed out" {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, format!("{error:#}\n")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::os::unix::fs::PermissionsExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn mail_rules_routes_delegate_json_drafts_and_literal_names_to_cli_children() {
        let dir = std::env::temp_dir().join(format!("mail-rules-route-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let fake = dir.join("mecha");
        std::fs::write(&fake, "#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps({'argv':sys.argv[1:],'stdin':sys.stdin.read()}))\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let draft = "description='Literal $(text)'\n";
        let digest = serde_json::json!({"directory":"shown"});
        let cases = [
            (
                "/api/mail/rules/add",
                serde_json::json!({"name":"-literal;name", "rule_toml":draft}),
                "add",
                Some("-literal;name"),
            ),
            (
                "/api/mail/rules/notice/edit",
                serde_json::json!({"rule_toml":draft}),
                "edit",
                Some("notice"),
            ),
            (
                "/api/mail/rules/notice/mode",
                serde_json::json!({"group":"mailbox", "mode":"auto", "digest":digest}),
                "mode",
                Some("notice"),
            ),
            (
                "/api/mail/rules/notice/pause",
                serde_json::json!({}),
                "pause",
                Some("notice"),
            ),
            (
                "/api/mail/rules/pause-all",
                serde_json::json!({}),
                "pause-all",
                None,
            ),
            (
                "/api/mail/rules/undo",
                serde_json::json!({"row_id":"-literal;row"}),
                "undo",
                Some("-literal;row"),
            ),
            (
                "/api/mail/rules/wrong-match",
                serde_json::json!({"row_id":"-literal;row"}),
                "wrong-match",
                Some("-literal;row"),
            ),
            (
                "/api/mail/rules/notice/test",
                serde_json::json!({"account":"campus"}),
                "test",
                Some("notice"),
            ),
        ];
        super::super::review::JSON_CHILD_EXE
            .scope(fake, async {
                for (uri, body, verb, name) in cases {
                    let app = super::super::router(super::super::tests::test_state(), None);
                    let response = app
                        .oneshot(
                            Request::builder()
                                .method("POST")
                                .uri(uri)
                                .header("Tailscale-User-Login", "owner@example.com")
                                .header("x-mecha-request", "1")
                                .header("content-type", "application/json")
                                .body(Body::from(body.to_string()))
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(response.status(), StatusCode::OK, "{uri}");
                    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                        .await
                        .unwrap();
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    let args = value["argv"].as_array().unwrap();
                    assert_eq!(args[0], "mail");
                    assert_eq!(args[1], "rule");
                    assert_eq!(args[2], verb);
                    if let (Some(name), "add") = (name, verb) {
                        // A leading hyphen stays a value when bound with `=`.
                        assert!(args.iter().any(|v| *v == format!("--name={name}")));
                    } else if let Some(name) = name {
                        let separator = args.iter().position(|v| v == "--").unwrap();
                        assert_eq!(args[separator + 1], name);
                    }
                    if verb == "test" {
                        assert!(args.iter().any(|v| v == "--account=campus"));
                    }
                    if matches!(verb, "add" | "edit") {
                        assert_eq!(value["stdin"], draft);
                    }
                    if verb == "mode" {
                        assert!(args.iter().any(|v| v == "--require-digest"));
                        assert!(args.iter().any(|v| v == "--yes"));
                    }
                }
            })
            .await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
