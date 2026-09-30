//! `GET /api/features` — which optional parts of mecha are on, for the web
//! app to show and hide by (FEATURES-DESIGN.md §4.2 item 3, §9 step 2).
//!
//! The same rows as `mecha features --json`, from the same registry: the nav,
//! Home's cards and Settings → Features read `shown` and `next` rather than
//! restating which states hide. Two things only this route adds:
//!
//! - **The global file is re-read on every request.** It is config plus the
//!   disk — no socket, ever, since several servers here start the moment they
//!   are asked (§4.3) — so a flip made with `mecha features enable` reaches the
//!   page on its next load, with no restart.
//! - **`pending`: the file and this process disagree.** `serve` loaded its
//!   switches once, at start, and its chat's tools and connections are built
//!   from them, so a switch flipped since is true on disk and not yet here.
//!   Only `serve` knows what it loaded, so the comparison is made here — an
//!   annotation on the row, never a sixth state (§4.2).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mecha_core::config::Config;
use mecha_core::feature::{self, Facts, Feature};

type St = State<super::WebState>;

/// The features whose switch `config` has on, read once when `serve` starts.
pub(super) fn switched_at_start(config: &Config) -> Arc<Vec<Feature>> {
    Arc::new(
        Feature::ALL
            .iter()
            .copied()
            .filter(|f| feature::switched_on(config, *f))
            .collect(),
    )
}

/// GET /api/features.
pub(super) async fn list(State(state): St) -> Response {
    let loaded = Arc::clone(&state.features_at_start);
    let read = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let home = mecha_core::work::mecha_home()?;
        let cfg = Config::load_global()?;
        Ok(body(&Facts::read(&home, &cfg), &loaded))
    })
    .await;
    match read {
        Ok(Ok(body)) => Json(body).into_response(),
        // A config edited since start that no longer loads is its own
        // answer, never an empty list: the page shows everything and says
        // why, rather than hiding tabs off a file it could not read.
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the configuration could not be read: {e:#}\n"),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response(),
    }
}

/// The rows, each with whether this process loaded a different switch.
pub(super) fn body(facts: &Facts, loaded: &[Feature]) -> serde_json::Value {
    let features: Vec<serde_json::Value> = feature::all(facts)
        .into_iter()
        .map(|row| {
            let pending = feature::switched_on(&facts.config, row.id) != loaded.contains(&row.id);
            let mut v = serde_json::to_value(&row).unwrap_or_default();
            v["pending"] = pending.into();
            v
        })
        .collect();
    serde_json::json!({
        "features": features,
        "unknown_switches": feature::unknown_switches(&facts.config),
    })
}

/// For tests: the body for `cfg` read against `home`, as the route reads it.
#[cfg(test)]
pub(super) fn body_at(
    home: &std::path::Path,
    cfg: &Config,
    loaded: &[Feature],
) -> serde_json::Value {
    body(&Facts::read(home, cfg), loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
        body["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no row {id}"))
    }

    /// The page reads `shown`, `next` and `pending` off each row; `pending`
    /// is the file against what this process loaded, both directions.
    #[test]
    fn a_row_says_whether_it_is_shown_what_turns_it_on_and_whether_serve_lags() {
        let mut cfg = Config::default();
        cfg.features.0.insert("personas".into(), true);
        cfg.features.0.insert("web".into(), true);
        let facts = Facts {
            config: cfg.clone(),
            ..Facts::default()
        };
        // Loaded with web alone: personas was switched on since.
        let body = body(&facts, &[Feature::Web]);
        let personas = row(&body, "personas");
        assert_eq!(personas["state"], "on");
        assert_eq!(personas["shown"], true);
        assert_eq!(personas["pending"], true);
        assert_eq!(row(&body, "web")["pending"], false);
        let image = row(&body, "image");
        assert_eq!(image["shown"], false);
        assert_eq!(image["next"], "mecha features enable image");
        assert_eq!(image["pending"], false);
        // Switched off since start: pending the other way.
        let body = super::body(&facts, &[Feature::Web, Feature::Image]);
        assert_eq!(row(&body, "image")["pending"], true);
        assert!(body["unknown_switches"].as_array().unwrap().is_empty());
    }

    /// The page's maps name features by id, and an id the rows do not carry
    /// reads as shown (`isShown`, fail-open on purpose) — so a renamed or
    /// misspelled id would silently stop hiding its tab. Every id in
    /// `features.js`'s two maps must be a registry id, and every queue it
    /// names must be one Home labels, or the card it gates is not the card
    /// it thinks.
    #[test]
    fn every_feature_the_web_app_names_is_a_registry_id() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/lib");
        let js = std::fs::read_to_string(root.join("features.js")).unwrap();
        let home = std::fs::read_to_string(root.join("Home.svelte")).unwrap();
        let map = |name: &str| -> Vec<(String, String)> {
            let start = js
                .find(&format!("export const {name} = {{"))
                .unwrap_or_else(|| panic!("{name} is not in features.js"));
            let block = &js[start..start + js[start..].find("};").unwrap()];
            block
                .lines()
                .skip(1)
                .filter_map(|l| {
                    let (k, v) = l.trim().trim_end_matches(',').split_once(": ")?;
                    let unquote = |s: &str| s.trim_matches(|c| c == '\'' || c == '"').to_string();
                    Some((unquote(k), unquote(v)))
                })
                .collect()
        };
        let views = map("VIEW_FEATURE");
        let queues = map("QUEUE_FEATURE");
        assert!(
            views.len() >= 5 && queues.len() >= 5,
            "{views:?} {queues:?}"
        );
        for (key, id) in views.iter().chain(&queues) {
            assert!(
                Feature::parse(id).is_some(),
                "`{key}: {id}` names no feature"
            );
        }
        for (queue, _) in &queues {
            assert!(
                home.contains(&format!("'{queue}': ")),
                "Home.svelte does not label the queue `{queue}`"
            );
        }
    }

    /// `/api/features` opens no socket (FEATURES-DESIGN.md "How to know it
    /// works"). Every address a switched-on feature names points at a
    /// listener here, and the listener must see no connection: a page load
    /// that probed would start the OCR model or the image server.
    #[test]
    fn reading_the_features_connects_to_nothing() {
        // `Facts::read` reads `$MECHA_MAIL_DIR`, which other tests move.
        let _lock = crate::testenv::lock();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cfg: Config = toml::from_str(&format!(
            r#"
            [features]
            web = true
            search = true
            documents = true
            image = true
            voice = true
            incognito = true

            [[search]]
            kind = "searxng"
            base_url = "{url}"

            [documents]
            ocr = true
            ocr_url = "{url}"

            [image]
            url = "{url}"
            "#
        ))
        .unwrap();
        let home =
            std::env::temp_dir().join(format!("mecha-features-nosock-{}", std::process::id()));
        let body = body_at(&home, &cfg, &[]);
        assert_eq!(row(&body, "search")["state"], "on", "{body:#}");
        assert_eq!(row(&body, "ocr")["state"], "on", "{body:#}");
        assert_eq!(row(&body, "image")["state"], "on", "{body:#}");
        // The negative is not vacuous: the listener does accept.
        let probe = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let (first, _) = listener
            .accept()
            .expect("the listener accepts a connection");
        assert_eq!(first.peer_addr().unwrap(), probe.local_addr().unwrap());
        assert!(
            listener.accept().is_err(),
            "reading the features connected to {url}"
        );
    }
}
