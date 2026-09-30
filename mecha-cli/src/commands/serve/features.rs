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
            // A row that did not serialise would reach the page with no
            // `id`, which `isShown` reads as shown: never swallowed.
            let mut v = serde_json::to_value(&row).expect("a feature row serialises");
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
        // Whether its guard has landed, which the web keys a refusal's
        // consequences on (a flat card, a pane sent home).
        assert_eq!(image["gated"], true);
        assert_eq!(row(&body, "frontdoor")["gated"], false);
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
        let panes = map("PANE_FEATURE");
        assert!(
            views.len() >= 5 && queues.len() >= 5 && panes.len() >= 3,
            "{views:?} {queues:?} {panes:?}"
        );
        for (key, id) in views.iter().chain(&queues).chain(&panes) {
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

        // And every id written inline in a component — the buttons and tabs
        // most of this hiding is done by — for the same reason: `'frontdor'`
        // would leave its tab up with the feature off and every suite green
        // (review of #449, pass 2). `opens` takes a view, held to the map.
        let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src");
        let mut sources = vec![web.join("App.svelte")];
        sources.extend(
            std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "svelte")),
        );
        let literals = |text: &str, call: &str| -> Vec<String> {
            text.match_indices(call)
                .filter_map(|(i, _)| {
                    let rest = &text[i + call.len()..];
                    Some(rest[..rest.find('\'')?].to_string())
                })
                .collect()
        };
        let (mut ids, mut used_views) = (0, 0);
        for path in &sources {
            let text = std::fs::read_to_string(path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy();
            for id in literals(&text, "isShown(features.rows, '") {
                assert!(
                    Feature::parse(&id).is_some(),
                    "{name}: `{id}` names no feature"
                );
                ids += 1;
            }
            for view in literals(&text, "opens(features.rows, '") {
                assert!(
                    views.iter().any(|(v, _)| *v == view),
                    "{name}: `{view}` is not a view VIEW_FEATURE maps"
                );
                used_views += 1;
            }
        }
        assert!(ids >= 9 && used_views >= 2, "{ids} ids, {used_views} views");
    }

    /// Home keeps the Questions card and the workflows line whatever a
    /// switch says — so each place they land must open even when its view's
    /// feature is hidden, or the tap bounces back to Home and the waiting
    /// thing has no door (review of #449). Every `view/sub` destination Home
    /// names inside a feature's view is in `OPENS_ANYWAY`, except a queue
    /// card's own, which goes flat when its feature is off (step 3).
    #[test]
    fn every_place_home_lands_opens_whatever_its_view_s_switch_says() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/lib");
        let js = std::fs::read_to_string(root.join("features.js")).unwrap();
        let home = std::fs::read_to_string(root.join("Home.svelte")).unwrap();
        let line = js
            .lines()
            .find(|l| l.starts_with("export const OPENS_ANYWAY = ["))
            .expect("OPENS_ANYWAY is one line in features.js");
        let quoted = |text: &str| -> Vec<String> {
            text.split('\'')
                .skip(1)
                .step_by(2)
                .map(String::from)
                .collect()
        };
        let opens = quoted(line);
        let views: Vec<String> = {
            let start = js.find("export const VIEW_FEATURE = {").unwrap();
            let block = &js[start..start + js[start..].find("};").unwrap()];
            block
                .lines()
                .skip(1)
                .filter_map(|l| l.trim().split_once(':').map(|(k, _)| k.to_string()))
                .collect()
        };
        // Every `'view/sub'` literal, read at each quote on its own: Home's
        // comments carry apostrophes, so quotes do not pair across the file.
        let lower = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase());
        let landings: Vec<String> = home
            .match_indices('\'')
            .filter_map(|(i, _)| {
                let rest = &home[i + 1..];
                let lit = &rest[..rest.find('\'')?];
                let (v, sub) = lit.split_once('/')?;
                (lower(v) && lower(sub)).then(|| lit.to_string())
            })
            .collect();
        assert!(
            landings.iter().any(|l| l == "tasks/waiting"),
            "the Questions card's landing was not found: {landings:?}"
        );
        // A queue card with a feature of its own goes flat when that
        // feature is off (its pane's routes answer `feature_off`), so its
        // landing need not open anyway: the queue names in QUEUE_FEATURE, and
        // Home's `queueTargets` maps each to its landing.
        let flat_when_off: Vec<String> = {
            let start = js.find("export const QUEUE_FEATURE = {").unwrap();
            let block = &js[start..start + js[start..].find("};").unwrap()];
            let queues: Vec<&str> = block
                .lines()
                .skip(1)
                .filter_map(|l| l.trim().split_once(':').map(|(k, _)| k.trim_matches('\'')))
                .collect();
            let start = home.find("const queueTargets = {").unwrap();
            let targets = &home[start..start + home[start..].find("};").unwrap()];
            targets
                .lines()
                .filter_map(|l| {
                    let (k, v) = l.trim().trim_end_matches(',').split_once(": ")?;
                    queues
                        .contains(&k.trim_matches('\''))
                        .then(|| v.trim_matches('\'').to_string())
                })
                .collect()
        };
        assert!(
            flat_when_off.iter().any(|l| l == "library/candidates"),
            "{flat_when_off:?}"
        );
        for l in &landings {
            let (view, _) = l.split_once('/').unwrap();
            if views.iter().any(|v| v == view) && !flat_when_off.contains(l) {
                assert!(
                    opens.contains(l),
                    "Home lands on `{l}`, inside a feature's view, and OPENS_ANYWAY does not list it"
                );
            }
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
