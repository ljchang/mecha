//! The settings page's backend: the charter, the learned rules, and the
//! voice stack's health, behind the same owner guard as everything else.
//!
//! What may be *written* from here is decided by who owns the consequence,
//! and the answer is: exactly one thing. The charter is the owner's own
//! document, read by every run and writable by "the owner with a text
//! editor" (`docs/GOAL-SYSTEM-DESIGN.md` §11) — a validated save from the
//! owner's authenticated page is that, with a different editor. Everything
//! else on the page is a read: learned rules mutate through their own gated
//! verbs (`mecha rules retire` stages through proposals; nothing here
//! shortcuts that), and the voice stack is configured where it runs.
//! Deliberately absent, and not as an oversight: anything whose edit widens
//! security posture — `[sandbox]`, `[security]`, `[outbox]` routing — stays
//! in `config.toml` where a diff reviews it, on `names_guarded_setting`'s
//! own list of the boundaries that always reach a human.
//!
//! Two rules on the charter write, both structural:
//!
//! - **A save is validated by the same reader every run loads through**
//!   (`Charter::parse`, which `Charter::load` itself delegates to), and an
//!   invalid document is refused with the parse error — it never reaches
//!   disk. The TUI's `/charter` accepts an invalid save and reports it,
//!   because there the file was edited *in place* by `$EDITOR` and the
//!   damage is already done; here the bytes are still ours to refuse, so
//!   refusing is strictly better than the warning.
//! - **Temp-sibling-and-rename**, the store convention: a browser that
//!   disconnects mid-request must not leave half a charter where every
//!   future run's priorities live.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

type St = State<super::WebState>;

/// The one shape both the GET and a successful save return, so the page
/// never has to merge two descriptions of the same file.
fn charter_state() -> Json<serde_json::Value> {
    let path = match mecha_core::charter::Charter::default_path() {
        Ok(p) => p,
        Err(e) => {
            return Json(serde_json::json!({ "error": format!("{e:#}") }));
        }
    };
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let body = match mecha_core::charter::Charter::load(&path) {
        Ok(charter) => serde_json::json!({
            "path": path,
            "exists": path.is_file(),
            "raw": raw,
            "lines": charter.lines().iter().map(|l| serde_json::json!({
                "id": l.id,
                "text": l.text,
            })).collect::<Vec<_>>(),
            "char_count": charter.char_count(),
            "over_budget": charter.over_budget(),
            "budget": mecha_core::charter::CHARTER_CHAR_BUDGET,
        }),
        // A broken charter is a state the page must show, not a 500: the
        // TUI's rule that the failure is the headline, one surface over.
        Err(e) => serde_json::json!({
            "path": path,
            "exists": path.is_file(),
            "raw": raw,
            "parse_error": format!("{e:#}"),
        }),
    };
    Json(body)
}

/// GET /api/settings/charter
pub async fn charter(State(_state): St) -> Json<serde_json::Value> {
    charter_state()
}

#[derive(Deserialize)]
pub struct CharterSave {
    raw: String,
}

/// A charter is a handful of lines under a 2,000-character rendered budget;
/// a body orders of magnitude past that is not an edit of one, whatever it
/// parses as. Refused before the parser sees it, so a runaway paste cannot
/// cost a TOML parse of arbitrary input.
const MAX_CHARTER_BYTES: usize = 64 * 1024;

/// POST /api/settings/charter — validate, then write, in that order.
pub async fn charter_save(State(_state): St, Json(body): Json<CharterSave>) -> Response {
    if body.raw.len() > MAX_CHARTER_BYTES {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{} bytes is not a charter — the whole rendered budget is {} characters\n",
                body.raw.len(),
                mecha_core::charter::CHARTER_CHAR_BUDGET
            ),
        )
            .into_response();
    }
    // The same reader every run loads through. A document this refuses
    // never reaches disk, which is the property the module doc names.
    if let Err(e) = mecha_core::charter::Charter::parse(&body.raw) {
        return (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}\n")).into_response();
    }
    let path = match mecha_core::charter::Charter::default_path() {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
        }
    }
    // Temp-sibling-and-rename: same directory, so the rename cannot cross a
    // filesystem, and a crash between the two leaves the old charter whole.
    let tmp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    let write = std::fs::write(&tmp, &body.raw).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
    }
    charter_state().into_response()
}

/// GET /api/settings/rules — the learned-rule roster with its ledger
/// tallies, exactly what the TUI's `/learning` reads. A read: retiring goes
/// through `mecha rules retire`'s own staged path, and nothing here offers a
/// shortcut around it.
pub async fn rules(State(_state): St) -> Response {
    match super::self_cli_json(&["rules", "list", "--json"], false).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

/// GET /api/settings/voice — is the stack there at all, and where this
/// process would send an offer. Read-only: the voice worker is configured
/// where it runs (`scripts/voice/`), and a reachability answer is what a
/// settings page can honestly own from here.
pub async fn voice(State(state): St) -> Json<serde_json::Value> {
    let target = state.offer_target.as_ref().map(|t| t.as_str().to_string());
    let reachable = match &target {
        None => None,
        Some(url) => Some(probe(url).await),
    };
    Json(serde_json::json!({
        // None = voice is not wired on this serve at all — a different fact
        // from "wired and down", and the page shows them differently.
        "offer_target": target,
        "worker_reachable": reachable,
    }))
}

/// One cheap TCP-level probe: any HTTP answer at all means the worker
/// process is up, and nothing more is claimed. The offer endpoint itself is
/// deliberately not exercised — a probe that opened WebRTC sessions to find
/// out would be a load test wearing a health check's clothes.
async fn probe(url: &str) -> bool {
    let base = url.strip_suffix("/api/offer").unwrap_or(url).to_string();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get(base).send().await.is_ok()
}

#[cfg(test)]
mod tests {
    /// The refusal order is load-bearing: an oversized body must be refused
    /// before the parser runs, and an invalid document must never reach
    /// disk. Exercised through `Charter::parse` directly, because the
    /// handler's other half is filesystem plumbing the charter tests in
    /// `mecha-core` already cover.
    #[test]
    fn an_invalid_charter_is_refused_by_the_same_reader_runs_use() {
        let dup = r#"
[[line]]
id = "a"
text = "first"
[[line]]
id = "a"
text = "second"
"#;
        let e = mecha_core::charter::Charter::parse(dup)
            .unwrap_err()
            .to_string();
        assert!(e.contains("more than once"), "{e}");

        let typod = r#"
[[lines]]
id = "a"
text = "first"
"#;
        assert!(mecha_core::charter::Charter::parse(typod).is_err());
    }
}
