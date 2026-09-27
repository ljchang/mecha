//! The chat's model chip (REMOTE-SURFACE-DESIGN §14, step 5): what the router
//! can serve, and the owner's switch. Each is a `mecha model` verb run as a
//! child (D4), so the page can do nothing a terminal cannot, and the rules —
//! R1's rollback, R4's refusal, D13's wait for runs — live in one place.
//!
//! **A switch outlives the request that asked for it.** By D13 it waits, with
//! no time limit, until no run holds the model, so the route starts `mecha
//! model use` and answers at once if the child has not finished in a moment.
//! The page then reads the waiting state from `model list --json`, which
//! reports the pending switch and what it waits for, the same file every
//! surface's runs are checking.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Mutex;
use std::time::Duration;

type St = State<super::WebState>;

/// How long the route waits for `mecha model use` before answering "started".
/// A refusal (R4, a second switch, a name nothing serves) comes back well
/// inside it, so the page shows it on the tap rather than on the next read.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

/// How the last switch this page started ended, for the read after it. The
/// router says what is loaded; only this says *why* a switch did not happen
/// (R1 loaded the old model back, the load timed out).
#[derive(Clone, serde::Serialize)]
struct Outcome {
    to: String,
    ok: bool,
    message: String,
    at: chrono::DateTime<chrono::Utc>,
}

static LAST: Mutex<Option<Outcome>> = Mutex::new(None);

/// GET /api/model — `mecha model list --json`, plus how the last switch
/// started here ended.
pub async fn state(State(state): St) -> Response {
    match super::review::self_json(&state, &["model", "list", "--json"]).await {
        Ok(mut v) => {
            let last = LAST.lock().ok().and_then(|l| l.clone());
            v["last_switch"] = serde_json::to_value(last).unwrap_or_default();
            Json(v).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct UseBody {
    /// A provider entry, or a model the router serves — what `model use`
    /// takes.
    pub name: String,
    /// "Switch now": `--now`, which for a switch already waiting on this
    /// model hurries that one instead.
    #[serde(default)]
    pub now: bool,
}

/// POST /api/model/use — `mecha model use <name> --json [--now]`. Answers
/// with the child's result if it finishes within [`ANSWER_WITHIN`], and
/// `202 {"started": true}` if it is still waiting or loading.
pub async fn switch(State(_state): St, Json(body): Json<UseBody>) -> Response {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "no model named\n").into_response();
    }
    let mut cmd = tokio::process::Command::new(crate::exe::self_exe());
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    cmd.args(["model", "use", "--json"]);
    if body.now {
        cmd.arg("--now");
    }
    // `--` so a name is never read as a flag.
    cmd.arg("--").arg(&name);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawning: {e:#}\n"),
            )
                .into_response()
        }
    };
    // The child is awaited on its own task, so it is reaped and its outcome
    // recorded however long D13 makes it wait — whether or not this request
    // is still there to hear it.
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = match child.wait_with_output().await {
            Ok(out) => settle(&out),
            Err(e) => Err(format!("waiting on the switch: {e}")),
        };
        // A hurry only nudges a switch another child owns; that one records
        // how the switch ended, and this must not read as its outcome.
        let hurried = matches!(&result, Ok(v) if v["hurried"] == true);
        if !hurried {
            if let Ok(mut last) = LAST.lock() {
                *last = Some(Outcome {
                    to: name,
                    ok: result.is_ok(),
                    message: match &result {
                        Ok(v) => v["warning"].as_str().unwrap_or_default().to_string(),
                        Err(e) => e.clone(),
                    },
                    at: chrono::Utc::now(),
                });
            }
        }
        let _ = tx.send(result);
    });
    match tokio::time::timeout(ANSWER_WITHIN, rx).await {
        Ok(Ok(Ok(v))) => Json(v).into_response(),
        Ok(Ok(Err(why))) => (StatusCode::CONFLICT, format!("{why}\n")).into_response(),
        Ok(Err(_)) => (StatusCode::INTERNAL_SERVER_ERROR, "the switch task died\n").into_response(),
        Err(_) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "started": true })),
        )
            .into_response(),
    }
}

/// POST /api/model/cancel — `mecha model cancel-switch`: withdraw the switch
/// that is waiting, and the model that is loaded stays.
pub async fn cancel(State(state): St) -> Response {
    super::review::verb(&state, &["model", "cancel-switch"]).await
}

/// The child's result: its JSON on success, or what it said went wrong —
/// from its `mecha: ` line on, because a refusal (R4's names every
/// disagreeing entry) is several lines and the last alone reads as advice
/// with no subject.
fn settle(out: &std::process::Output) -> Result<serde_json::Value, String> {
    if out.status.success() {
        return serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("the switch finished but its answer did not parse: {e}"));
    }
    Err(refusal(&String::from_utf8_lossy(&out.stderr)))
}

fn refusal(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    match lines.iter().rposition(|l| l.starts_with("mecha: ")) {
        Some(i) => lines[i..]
            .join("\n")
            .trim_start_matches("mecha: ")
            .trim()
            .to_string(),
        None => lines
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map_or("the switch failed", |l| l.trim())
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::refusal;

    /// Progress lines before the error are not the error; the error's own
    /// continuation lines are.
    #[test]
    fn a_refusal_is_the_error_and_all_of_it() {
        let stderr = "waiting for 1 run(s) to finish: web chat\n\
                      mecha: refusing to load m: its preset's sampling disagrees —\n  \
                      [providers.x] temperature 0.7, preset 0.6\n\
                      Make the entry's temperature match the preset.\n";
        let r = refusal(stderr);
        assert!(r.starts_with("refusing to load m"), "{r}");
        assert!(r.contains("[providers.x]"), "{r}");
        assert!(r.ends_with("match the preset."), "{r}");
        assert!(!r.contains("waiting for"), "{r}");
    }

    #[test]
    fn a_refusal_without_the_prefix_is_its_last_line() {
        assert_eq!(refusal("one\ntwo\n\n"), "two");
        assert_eq!(refusal(""), "the switch failed");
    }
}
