//! One request to a local server, asking it what it is actually serving.
//!
//! `Sandbox::preflight` is the precedent and the argument is the same one
//! level up: a configured sandbox that does not work is worse than none,
//! because `shell` declares narrower capabilities when confined and the
//! interlock believes the claim. Config here makes three claims a run then
//! narrows around, and until now nothing checked any of them:
//!
//! - **`context_window`** decides the compaction threshold, the tool-output
//!   budget, the fuel gauge and what overflow recovery expects. `-c` is
//!   divided across slots, so the right value is `-c / -np` and the wrong
//!   one is `-c` — which is the same number until `-np` moves off 1, which
//!   is exactly what makes it easy to write down wrong.
//! - **`vision`** decides whether an image is put in front of the model or
//!   rendered as its own filename.
//! - **`model`** decides nothing at all on this backend — llama-server
//!   ignores the request's `model` field — but it decides what every session
//!   record, scorecard and price calculation *says* was answering.
//!
//! **Warn, never refuse.** A mismatch makes a run compact at the wrong
//! moment or quietly not send a picture; neither is a reason to refuse to
//! start, and a preflight that can stop a working machine from booting is
//! one people disable. That is the opposite of the sandbox's bargain, where
//! falling through means running unconfined.
//!
//! The comparison is a pure function over a struct, tested without a server.
//! The network call is the thin part.

use crate::config::ProviderConfig;
use serde::Deserialize;

/// The subset of llama-server's `GET /props` this cares about.
///
/// `#[serde(default)]` throughout: this is another program's output across
/// versions, and a field that moved should cost a check, never a parse
/// failure that takes the warning down with it.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct Props {
    #[serde(default)]
    pub model_alias: Option<String>,
    #[serde(default)]
    pub total_slots: Option<u64>,
    #[serde(default)]
    pub modalities: Modalities,
    #[serde(default)]
    pub default_generation_settings: GenerationSettings,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub vision: bool,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct GenerationSettings {
    /// The **per-slot** context, which is what `context_window` must equal.
    /// llama-server has already done the `-c / -np` division here, which is
    /// what makes reading it cheaper and more correct than reimplementing
    /// the arithmetic.
    #[serde(default)]
    pub n_ctx: Option<u64>,
}

/// Ask a local server what it is serving. `None` when it did not answer in
/// the shape expected — an endpoint that is not llama-server, or is not up.
///
/// Deliberately silent on failure: a provider that is merely not running yet
/// must not print a warning on every start of a machine that does not use it.
pub async fn fetch(base_url: &str) -> Option<Props> {
    let url = format!("{}/props", base_url.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let body = http.get(&url).send().await.ok()?;
    if !body.status().is_success() {
        return None;
    }
    body.json::<Props>().await.ok()
}

/// What config claims against what is served. Empty means they agree.
///
/// Pure, so the interesting half is unit-tested without a model on the
/// machine — the same split `compact.rs` uses, and for the same reason:
/// getting this wrong is silent.
pub fn disagreements(name: &str, cfg: &ProviderConfig, props: &Props) -> Vec<String> {
    let mut out = Vec::new();

    if let (Some(declared), Some(served)) =
        (cfg.context_window, props.default_generation_settings.n_ctx)
    {
        if declared != served {
            let slots = props.total_slots.unwrap_or(1);
            let hint = if slots > 1 {
                format!(
                    " The server has {slots} slots and divides `-c` evenly across them, so the \
                     value to write down is `-c / {slots}` and not `-c`."
                )
            } else {
                String::new()
            };
            out.push(format!(
                "[providers.{name}] context_window = {declared}, but the server is serving \
                 {served} tokens per slot.{hint} The compaction threshold, the tool-output \
                 budget and the fuel gauge are all derived from the configured number, so a \
                 stale one is worse than none."
            ));
        }
    }

    // **Both directions, and they fail differently.** This is the check that
    // would have caught a multimodal model served with no projector for as
    // long as anyone cared to look.
    match (cfg.vision_enabled(), props.modalities.vision) {
        (true, false) => out.push(format!(
            "[providers.{name}] vision = true, but the server reports no vision. Every image \
             will silently arrive as a line of text naming the file. A vision model is two \
             files: the weights, and a projector that `--mmproj` must name. `--mmproj-auto` \
             only fires for `-hf` downloads, so a server started with `-m <path>` gets nothing \
             from it."
        )),
        (false, true) => out.push(format!(
            "[providers.{name}] is serving a vision model — the projector is loaded and paid \
             for in memory — but `vision` is not set, so no image will ever be sent to it. Set \
             `vision = true`."
        )),
        _ => {}
    }

    // Not an error, and worth saying anyway: llama-server ignores the
    // request's `model` field, so naming one is not selecting it — only
    // deciding what gets recorded. A session, a scorecard and a price that
    // all name the wrong model are wrong quietly and forever.
    if let (Some(declared), Some(served)) = (cfg.model.as_deref(), props.model_alias.as_deref()) {
        if declared != served {
            out.push(format!(
                "[providers.{name}] model = {declared:?}, but the server is serving \
                 {served:?}. llama-server ignores the request's `model` field, so this does \
                 not change which weights answer — it changes what every session record and \
                 scorecard says answered."
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ProviderConfig {
        let mut c = crate::config::Config::default()
            .providers
            .get("anthropic")
            .cloned()
            .unwrap();
        c.kind = "local".into();
        c.model = None;
        c.api_key_env = None;
        c
    }

    fn props(n_ctx: u64, slots: u64, vision: bool) -> Props {
        Props {
            model_alias: None,
            total_slots: Some(slots),
            modalities: Modalities { vision },
            default_generation_settings: GenerationSettings { n_ctx: Some(n_ctx) },
        }
    }

    #[test]
    fn agreement_is_silent() {
        let mut c = cfg();
        c.context_window = Some(32768);
        assert!(disagreements("local", &c, &props(32768, 1, false)).is_empty());
    }

    /// The `-c / -np` trap: the two numbers are equal until `-np` moves off
    /// 1, which is what makes it easy to write down wrong and impossible to
    /// notice.
    #[test]
    fn a_context_window_naming_c_rather_than_c_over_np_is_caught_with_the_arithmetic() {
        let mut c = cfg();
        c.context_window = Some(262144);
        let found = disagreements("local", &c, &props(65536, 4, false));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("65536"), "{}", found[0]);
        assert!(found[0].contains("`-c / 4`"), "{}", found[0]);
    }

    /// The bug this whole module was written for, in the direction nobody
    /// looks: the model has eyes and nothing is using them.
    #[test]
    fn a_vision_model_served_with_no_one_configured_to_use_it_is_reported() {
        let c = cfg(); // vision unset, and `local` defaults to false
        let found = disagreements("local", &c, &props(8192, 1, true));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("vision = true"), "{}", found[0]);
    }

    /// And the direction that looks like the feature working.
    #[test]
    fn vision_declared_against_a_text_only_server_says_mmproj() {
        let mut c = cfg();
        c.vision = Some(true);
        let found = disagreements("local", &c, &props(8192, 1, false));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("--mmproj"), "{}", found[0]);
        assert!(
            found[0].contains("silently"),
            "the failure is silent, and the warning has to say so: {}",
            found[0]
        );
    }

    /// A field llama-server stops sending must cost a check, never the
    /// warning that would have named it.
    #[test]
    fn a_props_body_missing_everything_parses_and_reports_nothing() {
        let parsed: Props = serde_json::from_str("{}").unwrap();
        let mut c = cfg();
        c.context_window = Some(32768);
        assert!(disagreements("local", &c, &parsed).is_empty());
    }
}
