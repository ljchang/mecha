//! What a persona chat runs on: the pinned persona, its system prompt, its
//! registry, and the agent configuration the assistant's levers are taken
//! out of (`docs/PERSONA-DESIGN.md` §3.3, §3.4, §8.1, §8.2).
//!
//! One `Agent` per persona version (§3.4): the system prompt is fixed at
//! construction, so a persona's prefix — tools, then system — is byte-stable
//! across every chat pinned to that version. Nothing here builds the agent
//! itself; the surface that owns a provider does, from what this returns.
//!
//! The boundary this module holds is §3.1's one test: no conversation holds
//! both the assistant's reach and a persona's voice. Three things carry it:
//!
//! - **The registry is built, not narrowed.** `RunContext::withheld` refuses
//!   a call at dispatch but still offers the tool — the request's tool list
//!   comes from `Registry::specs_for` — so a persona's tools are a fresh
//!   `Registry` holding only what [`Tool::for_persona`] declares, and nothing
//!   whose class on this install is [`Egress::Chosen`].
//! - **The system prompt is replaced, never appended to.** The assistant's
//!   is assembled with the charter, skills and learned rules folded in, and
//!   none of that belongs to a persona.
//! - **Every lever that reads an owner store is off**, decided field by field
//!   in [`agent_config`], so a lever added later is a compile error here
//!   rather than an inheritance.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::sync::Arc;

use super::{
    current, digest_of, front_matter, parse_toml, read_prose, snapshot, strip_comments,
    validate_name, validate_persona_name, Answers, Settings, Status, VersionRecord,
};
use crate::config::{AgentConfig, SecurityConfig};
use crate::tool::{Egress, Registry};

/// What every persona chat is told first, before its relationship and its
/// identity. Harness text, the same for every persona, so it sits at the
/// front of the cached prefix.
///
/// Disclosure is the harness's job, not a line the persona must say (§12.1):
/// the page marks every chat with an AI tag beside the name (the banner it
/// replaced was dropped by the owner, 2026-09-30). What stays here is the situation the model is
/// in, and one line about a sincere question — what a character says inside
/// a story is the owner's to shape (R18), a person stepping out of it to ask
/// is not.
pub const BASE: &str = "\
# A persona chat

You are playing a character that the person you are talking with wrote. \
Who you are, how you relate to them and what you want are set out below, in \
their words; speak and act as that character.

This conversation is kept apart from their assistant. You cannot see their \
mail, calendar, notes, or any other conversation, and nothing you say is sent \
anywhere on their behalf. Use only the tools you have been given here.";

/// Said only while the persona's `disclosure` switch is on, so turning it off
/// takes the claim out of the cached prefix (found on review of #407). The
/// page's AI tag is what makes it true.
pub const DISCLOSED: &str = "\
The page already tells them they are talking with an AI, so you do not need \
to say it.";

/// Said either way: what a character says inside a story is the owner's to
/// shape (R18); a person stepping out of it to ask is not.
pub const SINCERE: &str = "\
If they sincerely step outside the conversation to ask whether they are \
talking to a person, tell them the truth.";

/// A persona as one chat sees it: read from a version's snapshot, never from
/// the live folder, so editing the persona applies to new chats while an
/// open one keeps the persona — and the cached prefix — it began with (§8.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Pinned {
    pub name: String,
    pub version: u32,
    pub digest: String,
    pub settings: Settings,
    pub identity: String,
    pub motivation: String,
    /// Each named template's text as snapshotted, in the order named.
    pub relationships: Vec<(String, String)>,
}

/// Pin a new chat to the persona as it stands: approved only, a version
/// taken if its files changed since the last, and that version read back.
pub fn pin(dir: &Path, name: &str) -> Result<Pinned> {
    let (_, p) = current(dir, name)?;
    if p.state.status != Status::Approved {
        bail!("`{name}` is not approved — `mecha persona approve {name}` after reading it");
    }
    let state = snapshot(dir, name)?;
    load_version(dir, name, &state.digest)
}

/// A version of `name` by digest, as a chat pinned to it renders it. The
/// snapshot is digested again on read, so a version edited on disk after it
/// was taken is refused rather than rendered as the one the chat pinned.
pub fn load_version(dir: &Path, name: &str, digest: &str) -> Result<Pinned> {
    validate_persona_name(name)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        bail!("`{digest}` is not a version digest");
    }
    let versions = dir.join(name).join("versions");
    let vdir = versions.join(digest);
    if !vdir.is_dir() {
        bail!("`{name}` has no version {digest}");
    }
    let raw_settings = read_prose(&vdir.join("persona.toml"))?;
    let settings: Settings = parse_toml(&raw_settings)
        .with_context(|| format!("reading `{name}`'s persona.toml at version {digest}"))?;
    let identity = read_prose(&vdir.join("identity.md"))?;
    let motivation = read_prose(&vdir.join("motivation.md"))?;
    let mut files = vec![
        ("persona.toml".to_string(), raw_settings),
        ("identity.md".to_string(), identity.clone()),
        ("motivation.md".to_string(), motivation.clone()),
    ];
    let mut relationships = Vec::new();
    for r in &settings.relationship.0 {
        // Validated before it becomes a path, as the live load does; the
        // re-digest below would catch a tampered name, but only after a read.
        validate_name(r).context("in the snapshot's `relationship`")?;
        let text = read_prose(&vdir.join("relationships").join(format!("{r}.md")))?;
        files.push((format!("relationships/{r}.md"), text.clone()));
        relationships.push((r.clone(), text));
    }
    if digest_of(&files) != digest {
        bail!("`{name}`'s version {digest} was changed on disk after it was taken; refusing it");
    }
    let log = std::fs::read_to_string(versions.join("log.jsonl"))
        .with_context(|| format!("reading `{name}`'s version log"))?;
    let version = log
        .lines()
        .filter_map(|l| serde_json::from_str::<VersionRecord>(l).ok())
        .filter(|r| r.digest == digest)
        .map(|r| r.version)
        .next_back()
        .with_context(|| format!("`{name}`'s version log does not name {digest}"))?;
    Ok(Pinned {
        name: name.to_string(),
        version,
        digest: digest.to_string(),
        settings,
        identity,
        motivation,
        relationships,
    })
}

/// Owner prose as a prompt holds it: comments out, blank runs folded, and
/// nothing at all when nothing is left.
fn prose(text: &str) -> String {
    let (text, _) = strip_comments(text);
    let mut out = String::new();
    let mut blank = 0;
    for line in text.trim().lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The system prompt, in cache order (§8.2): the shared base, the date
/// guidance, the relationship templates, then the persona's own identity and
/// motivation. A function of the pinned version alone, so every chat on one
/// version sends the same bytes. Everything that changes per chat — a
/// session goal, the re-anchor, recalled memory — rides in the messages.
pub fn system_prompt(p: &Pinned) -> Result<String> {
    let mut out = String::from(BASE);
    out.push_str("\n\n");
    if p.settings.safety.disclosure {
        out.push_str(DISCLOSED);
        out.push(' ');
    }
    out.push_str(SINCERE);
    out.push_str("\n\n");
    out.push_str(crate::date_context::GUIDANCE.trim_end());
    let templates: Vec<String> = p
        .relationships
        .iter()
        .map(|(name, raw)| {
            front_matter(raw)
                .map(|(_, body)| prose(&body))
                .with_context(|| format!("rendering relationship `{name}`"))
        })
        .collect::<Result<_>>()?;
    let templates: Vec<String> = templates.into_iter().filter(|t| !t.is_empty()).collect();
    if !templates.is_empty() {
        out.push_str("\n\n# How you relate to them\n\n");
        out.push_str(&templates.join("\n\n"));
    }
    let identity = prose(&p.identity);
    if !identity.is_empty() {
        out.push_str("\n\n# Who you are\n\n");
        out.push_str(&identity);
    }
    let motivation = prose(&p.motivation);
    if !motivation.is_empty() {
        out.push_str("\n\n# What you want\n\n");
        out.push_str(&motivation);
    }
    out.push('\n');
    Ok(out)
}

/// A tool the persona asked for and did not get, and why — shown to the
/// owner, never silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub tool: String,
    pub why: &'static str,
}

/// A persona's registry and what was left out of it.
pub struct PersonaTools {
    pub registry: Registry,
    pub refused: Vec<Refused>,
}

/// Build the persona's registry from `pool`, the tools this install has
/// (§3.3): each name the persona asks for is looked up, turned into its
/// persona form by the tool's own declaration, and admitted only if that
/// form is not [`Egress::Chosen`] *now*, on this install's configuration.
/// `answers = "files"` (§10.4) leaves out every tool that returns
/// third-party content — out of the registry, so the model is never offered
/// it.
pub fn registry_for(pool: &Registry, settings: &Settings) -> PersonaTools {
    // No folder name, but the settings' display name and character still
    // reach each tool's persona form, as [`registry_as`] passes them.
    registry_as(pool, "", settings)
}

/// [`registry_for`], for the persona named `name`: each tool is asked for
/// its form for *this* persona ([`crate::tool::Tool::for_persona_as`]), so
/// `image_generate` knows that "self" is the persona's linked character
/// (§8.6). The one a live chat is built with.
pub fn registry_as(pool: &Registry, name: &str, settings: &Settings) -> PersonaTools {
    let who = crate::tool::PersonaSelf {
        name: name.to_string(),
        display: settings.display.trim().to_string(),
        character: settings.character.clone().filter(|c| !c.trim().is_empty()),
    };
    let mut registry = Registry::new();
    let mut refused = Vec::new();
    for name in &settings.tools.allow {
        let refuse = |why| Refused {
            tool: name.clone(),
            why,
        };
        let Some(tool) = pool.get(name) else {
            refused.push(refuse("not available on this install"));
            continue;
        };
        let Some(form) = Arc::clone(tool).for_persona_as(&who) else {
            refused.push(refuse("a persona may never have it"));
            continue;
        };
        let caps = form.capabilities();
        // `>=`, not `==`: exact today, and still exact if a class is ever
        // added above `Chosen`.
        if caps.egress >= Egress::Chosen {
            refused.push(refuse(
                "here it could send to a destination the model names (for web_search: \
                 no backend with a fixed destination is configured)",
            ));
            continue;
        }
        if settings.files.answers == Answers::Files && caps.untrusted_input {
            refused.push(refuse("this persona answers from its files only"));
            continue;
        }
        registry.insert(form);
    }
    PersonaTools { registry, refused }
}

/// The assistant's agent configuration with the persona's system prompt in
/// place of its own, and every lever that reads an owner store switched off.
///
/// **An exhaustive destructure, on purpose.** A lever added to `AgentConfig`
/// is a compile error here until someone decides whether a persona chat may
/// have it — the alternative, `..base`, would hand the next lever that reads
/// the charter or the session corpus to every persona by default.
pub fn agent_config(base: &AgentConfig, system: String) -> AgentConfig {
    let AgentConfig {
        system_prompt: _,
        system_prompt_file: _,
        max_turns,
        max_tokens,
        effort,
        thinking,
        cache_prompt,
        force_final_answer,
        max_output_tokens,
        max_cost_usd,
        compact_at_tokens,
        timezone,
        compact_keep_recent,
        loop_guard,
        boredom: _,
        compact_validate,
        step_escalation: _,
        step_checks: _,
        goal_guidance: _,
        predictive_compaction,
        carried_state,
        sensors_in_brief: _,
        appraisals_in_brief: _,
        situation_brief: _,
        past_appraisals: _,
        success_examples: _,
        contrast_evidence: _,
    } = base.clone();
    AgentConfig {
        system_prompt: Some(system),
        system_prompt_file: None,
        // The run's mechanics carry over: limits, model settings, caching,
        // compaction, the clock's zone, the loop guard.
        max_turns,
        max_tokens,
        effort,
        thinking,
        cache_prompt,
        force_final_answer,
        max_output_tokens,
        max_cost_usd,
        compact_at_tokens,
        timezone,
        compact_keep_recent,
        loop_guard,
        compact_validate,
        predictive_compaction,
        carried_state,
        // Coaching for task work: a conversation is not going nowhere.
        boredom: false,
        // Plan steps and their checks: task machinery a chat does not run.
        step_escalation: false,
        step_checks: false,
        // Read the owner's charter, board, sensors and session corpus.
        goal_guidance: false,
        sensors_in_brief: false,
        appraisals_in_brief: false,
        situation_brief: false,
        past_appraisals: false,
        success_examples: false,
        contrast_evidence: false,
    }
}

/// The security settings a persona chat runs under: the install's, with the
/// leak guard lifted (D11). A persona registry holds no `Chosen` sender, and
/// its `web_search` is always the blind path, so what the guard would stop
/// after a recall is only a blind search — which the owner ruled should keep
/// working. The query can still carry what the persona remembers to the
/// configured search backends; that is the cost D11 chose.
pub fn security(base: &SecurityConfig) -> SecurityConfig {
    SecurityConfig {
        block_sends_after_private: false,
        ..base.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::search::{Depth, SearchBackend, SearchChain, SearchResponse, WebSearch};
    use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    /// A tool with whatever declaration and class a case needs.
    struct Stub {
        name: &'static str,
        eligible: bool,
        caps: Capabilities,
    }

    #[async_trait]
    impl Tool for Stub {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn capabilities(&self) -> Capabilities {
            self.caps
        }
        fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
            self.eligible.then_some(self as Arc<dyn Tool>)
        }
        async fn call(&self, _: Value, _: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("ok"))
        }
    }

    /// A tool that records who it was built for.
    struct Asks(Mutex<Option<crate::tool::PersonaSelf>>);

    #[async_trait]
    impl Tool for Asks {
        fn name(&self) -> &str {
            "image_generate"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
            Some(self as Arc<dyn Tool>)
        }
        fn for_persona_as(
            self: Arc<Self>,
            who: &crate::tool::PersonaSelf,
        ) -> Option<Arc<dyn Tool>> {
            *self.0.lock().unwrap() = Some(who.clone());
            Some(self as Arc<dyn Tool>)
        }
        async fn call(&self, _: Value, _: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("ok"))
        }
    }

    /// The live chat's registry tells each tool which persona it serves, so
    /// `image_generate` can draw "self" as the linked character (§8.6); a
    /// blank character is none.
    #[test]
    fn a_persona_registry_says_who_the_persona_is() {
        let asks = Arc::new(Asks(Mutex::new(None)));
        let mut pool = Registry::new();
        pool.insert(Arc::clone(&asks) as Arc<dyn Tool>);
        let mut s = settings(&["image_generate"], Answers::Open);
        s.display = " Maya ".into();
        s.character = Some("maya".into());
        let built = registry_as(&pool, "maya", &s);
        assert_eq!(names(&built.registry), ["image_generate"]);
        assert_eq!(
            asks.0.lock().unwrap().clone(),
            Some(crate::tool::PersonaSelf {
                name: "maya".into(),
                display: "Maya".into(),
                character: Some("maya".into()),
            })
        );
        s.character = Some("  ".into());
        registry_as(&pool, "maya", &s);
        assert_eq!(asks.0.lock().unwrap().as_ref().unwrap().character, None);
    }

    fn settings(allow: &[&str], answers: Answers) -> Settings {
        let mut s: Settings = toml::from_str("").unwrap();
        s.tools.allow = allow.iter().map(|s| s.to_string()).collect();
        s.files.answers = answers;
        s
    }

    fn names(r: &Registry) -> Vec<String> {
        r.iter().map(|t| t.name().to_string()).collect()
    }

    #[test]
    fn the_registry_holds_only_declared_tools_that_cannot_aim() {
        let mut pool = Registry::new();
        for (name, eligible, caps) in [
            ("drawing", true, Capabilities::default()),
            ("mailish", false, Capabilities::default().private()),
            ("aimed", true, Capabilities::default().sends()),
            (
                "searchy",
                true,
                Capabilities::default().untrusted().sends_blind(),
            ),
        ] {
            pool.insert(Arc::new(Stub {
                name,
                eligible,
                caps,
            }));
        }
        let all = ["drawing", "mailish", "aimed", "searchy", "missing"];
        let open = registry_for(&pool, &settings(&all, Answers::Open));
        assert_eq!(names(&open.registry), vec!["drawing", "searchy"]);
        let why: Vec<(&str, &str)> = open
            .refused
            .iter()
            .map(|r| (r.tool.as_str(), r.why))
            .collect();
        assert_eq!(why.len(), 3, "{why:?}");
        assert!(why[0].0 == "mailish" && why[0].1.contains("never"));
        assert!(why[1].0 == "aimed" && why[1].1.contains("destination"));
        assert!(why[2].0 == "missing" && why[2].1.contains("not available"));
        // Files-only leaves out what returns third-party content — from the
        // registry itself, so it is never offered.
        let files = registry_for(&pool, &settings(&all, Answers::Files));
        assert_eq!(names(&files.registry), vec!["drawing"]);
        assert!(files
            .refused
            .iter()
            .any(|r| r.tool == "searchy" && r.why.contains("files only")));
    }

    /// Which backend was asked, at what depth, in order.
    type Asked = Arc<Mutex<Vec<(String, Depth)>>>;

    /// A search backend that records the depth it was asked for.
    struct Backend {
        id: &'static str,
        quick: Egress,
        deep: Egress,
        asked: Asked,
    }

    #[async_trait]
    impl SearchBackend for Backend {
        fn id(&self) -> &str {
            self.id
        }
        async fn search(&self, _q: &str, _limit: usize, depth: Depth) -> Result<SearchResponse> {
            self.asked
                .lock()
                .unwrap()
                .push((self.id.to_string(), depth));
            Ok(SearchResponse {
                answer: Some(format!("from {}", self.id)),
                ..SearchResponse::default()
            })
        }
        fn egress(&self, depth: Depth) -> Egress {
            match depth {
                Depth::Quick => self.quick,
                Depth::Deep => self.deep,
            }
        }
    }

    fn chain(kinds: &[(&'static str, Egress, Egress)]) -> (Arc<SearchChain>, Asked) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let backends: Vec<Box<dyn SearchBackend>> = kinds
            .iter()
            .map(|(id, quick, deep)| {
                Box::new(Backend {
                    id,
                    quick: *quick,
                    deep: *deep,
                    asked: Arc::clone(&asked),
                }) as Box<dyn SearchBackend>
            })
            .collect();
        (Arc::new(SearchChain::new(backends)), asked)
    }

    /// §3.3's fixture test, over the real tools: the owner-store half is the
    /// declaration, checked against an explicit list because `Capabilities`
    /// has no axis to derive it from; the egress half is each persona form's
    /// class on the configuration at hand — a blind, a chosen and a mixed
    /// `[[search]]` chain.
    #[test]
    fn every_real_tool_declares_as_the_design_lists() {
        const ELIGIBLE: [&str; 5] = [
            "document_read",
            "image_generate",
            "image_library",
            "image_view",
            "web_search",
        ];
        let dir = scratch();
        let mut pool = Registry::new().with_builtins(
            &crate::config::ToolsConfig::default(),
            Arc::new(crate::sandbox::Sandbox::new(Default::default())),
        );
        let (blind, _) = chain(&[("searxng", Egress::Blind, Egress::Blind)]);
        pool.insert(Arc::new(WebSearch::new(Arc::clone(&blind))));
        pool.insert(Arc::new(crate::search::WebOpen::new(Arc::new(
            crate::search::ResultLedger::new(),
        ))));
        pool.insert(Arc::new(
            crate::imagegen::ImageGenerate::new(crate::imagegen::ImageConfig {
                url: "http://127.0.0.1:8188".into(),
                ..Default::default()
            })
            .unwrap(),
        ));
        pool.insert(Arc::new(crate::tool::image_view::ImageView));
        pool.insert(Arc::new(crate::tool::image_library::ImageLibrary::new(
            dir.clone(),
        )));
        pool.insert(Arc::new(
            crate::tool::image_library::ImageLibraryPropose::new(dir.clone()),
        ));
        pool.insert(Arc::new(crate::tool::recall::Recall::new(
            dir.join("t.jsonl"),
        )));
        pool.insert(Arc::new(crate::tool::document::DocumentRead::new(
            crate::document::Extractor::new(
                crate::document::DocumentsConfig {
                    ocr: false,
                    cache: false,
                    ..Default::default()
                },
                None,
            )
            .unwrap(),
        )));
        assert!(
            pool.len() >= 15,
            "the pool should hold every constructible tool"
        );
        for tool in pool.iter() {
            let name = tool.name();
            let form = Arc::clone(tool).for_persona();
            assert_eq!(
                form.is_some(),
                ELIGIBLE.contains(&name),
                "`{name}`'s persona declaration disagrees with the design's list"
            );
            if let Some(form) = form {
                assert!(
                    form.capabilities().egress <= Egress::Blind,
                    "`{name}`'s persona form can aim"
                );
                assert_eq!(form.name(), name);
            }
        }
        // Egress, on each chain: a chosen-only chain makes web_search Chosen,
        // and the registry refuses it; blind and mixed chains admit it.
        let ask = settings(&["web_search"], Answers::Open);
        for (kinds, admitted) in [
            (vec![("searxng", Egress::Blind, Egress::Blind)], true),
            (vec![("aimed", Egress::Chosen, Egress::Chosen)], false),
            (
                vec![
                    ("aimed", Egress::Chosen, Egress::Chosen),
                    ("exa", Egress::Blind, Egress::Chosen),
                ],
                true,
            ),
        ] {
            let (c, _) = chain(&kinds);
            let mut p = Registry::new();
            p.insert(Arc::new(WebSearch::new(c)));
            let built = registry_for(&p, &ask);
            assert_eq!(
                built.registry.get("web_search").is_some(),
                admitted,
                "{kinds:?}"
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// `document_read` is a persona's when the owner lists it and the persona
    /// answers from anything (owner ruling, 2026-09-30). A files-only persona
    /// withholds it with the web tools: a document's words are third-party
    /// content, and the refusal says why rather than dropping it silently.
    #[test]
    fn document_read_is_given_when_listed_and_withheld_from_a_files_only_persona() {
        let mut pool = Registry::new();
        pool.insert(Arc::new(crate::tool::document::DocumentRead::new(
            crate::document::Extractor::new(
                crate::document::DocumentsConfig {
                    ocr: false,
                    cache: false,
                    ..Default::default()
                },
                None,
            )
            .unwrap(),
        )));
        let open = registry_for(&pool, &settings(&["document_read"], Answers::Open));
        assert!(open.registry.get("document_read").is_some());
        assert!(open.refused.is_empty(), "{:?}", open.refused);
        // Not listed: not given. Eligible is not the same as granted.
        let unlisted = registry_for(&pool, &settings(&["web_search"], Answers::Open));
        assert!(unlisted.registry.get("document_read").is_none());
        let files = registry_for(&pool, &settings(&["document_read"], Answers::Files));
        assert!(files.registry.get("document_read").is_none());
        assert_eq!(files.refused.len(), 1);
        assert!(
            files.refused[0].why.contains("files only"),
            "{:?}",
            files.refused
        );
    }

    /// On a mixed chain the persona's web_search reaches only the blind
    /// backend, at quick depth, on a clean conversation, even when asked for
    /// deep — the state in which the assistant's would use the whole chain.
    #[tokio::test]
    async fn a_persona_search_is_blind_and_quick_whatever_it_asks() {
        let (c, asked) = chain(&[
            ("aimed", Egress::Chosen, Egress::Chosen),
            ("exa", Egress::Blind, Egress::Chosen),
        ]);
        let mut pool = Registry::new();
        pool.insert(Arc::new(WebSearch::new(c)));
        let built = registry_for(&pool, &settings(&["web_search"], Answers::Open));
        let tool = built.registry.get("web_search").unwrap();
        assert!(tool.input_schema()["properties"].get("depth").is_none());
        let clean = crate::agent::Taint::default();
        let ctx = ToolCtx {
            taint: Some(clean),
            ..ToolCtx::default()
        };
        let out = tool
            .call(json!({"query": "kelp", "depth": "deep"}), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            *asked.lock().unwrap(),
            vec![("exa".to_string(), Depth::Quick)]
        );
        // Control: the assistant's form on the same chain and taint takes
        // the chain in order, so the test is not passing on the chain alone.
        asked.lock().unwrap().clear();
        let (c2, asked2) = chain(&[
            ("aimed", Egress::Chosen, Egress::Chosen),
            ("exa", Egress::Blind, Egress::Chosen),
        ]);
        WebSearch::new(c2)
            .call(json!({"query": "kelp", "depth": "deep"}), &ctx)
            .await
            .unwrap();
        assert_eq!(asked2.lock().unwrap()[0].0, "aimed");
    }

    #[test]
    fn a_chat_renders_the_version_it_pinned() {
        let dir = scratch();
        let mut n = new("mara");
        n.relationships = vec!["colleague".into()];
        super::super::create(&dir, &no_lib(), n).unwrap();
        fill_core(&dir, "mara");
        let pinned = pin(&dir, "mara").unwrap();
        assert_eq!(pinned.version, 2);
        let prompt = system_prompt(&pinned).unwrap();
        // Cache order: base, date, relationship, identity.
        let at = |s: &str| {
            prompt
                .find(s)
                .unwrap_or_else(|| panic!("{s} missing:\n{prompt}"))
        };
        assert!(at("# A persona chat") < at("## What day it is"));
        assert!(at("## What day it is") < at("# How you relate to them"));
        assert!(at("You are a colleague") < at("# Who you are"));
        assert!(at("# Who you are") < at("A marine ecologist"));
        // No front matter, no comments, and nothing the owner's assistant has.
        for absent in ["+++", "tools =", "<!--", "starter shipped", "charter"] {
            assert!(!prompt.contains(absent), "`{absent}` in:\n{prompt}");
        }
        // An edit makes a new version; the pinned one still renders as it was.
        let id = dir.join("mara/identity.md");
        let text = std::fs::read_to_string(&id).unwrap();
        std::fs::write(&id, text.replace("distrusts", "enjoys")).unwrap();
        let again = load_version(&dir, "mara", &pinned.digest).unwrap();
        assert_eq!(system_prompt(&again).unwrap(), prompt);
        let newer = pin(&dir, "mara").unwrap();
        assert_eq!(newer.version, 3);
        assert!(system_prompt(&newer)
            .unwrap()
            .contains("enjoys easy answers"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// The "page already tells them" sentence rides only while disclosure is
    /// on; the sincere-question line rides either way.
    #[test]
    fn the_disclosure_sentence_follows_the_switch() {
        let dir = scratch();
        super::super::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let on = system_prompt(&pin(&dir, "mara").unwrap()).unwrap();
        assert!(on.contains(DISCLOSED) && on.contains(SINCERE));
        let toml = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        std::fs::write(
            &toml,
            text.replace("disclosure = true", "disclosure = false"),
        )
        .unwrap();
        let off = system_prompt(&pin(&dir, "mara").unwrap()).unwrap();
        assert!(!off.contains(DISCLOSED), "{off}");
        assert!(off.contains(SINCERE));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_version_edited_on_disk_is_refused() {
        let dir = scratch();
        super::super::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let pinned = pin(&dir, "mara").unwrap();
        let snap = dir
            .join("mara/versions")
            .join(&pinned.digest)
            .join("identity.md");
        std::fs::write(&snap, "## Core\nSomeone else.\n").unwrap();
        let e = load_version(&dir, "mara", &pinned.digest).unwrap_err();
        assert!(format!("{e}").contains("changed on disk"), "{e}");
        assert!(load_version(&dir, "mara", "../../x").is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    /// A snapshot naming a relationship that is a path is refused before
    /// any read — not only by the re-digest after one (review of #409).
    #[test]
    fn a_snapshot_relationship_is_a_name_not_a_path() {
        let dir = scratch();
        super::super::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let pinned = pin(&dir, "mara").unwrap();
        let toml = dir
            .join("mara/versions")
            .join(&pinned.digest)
            .join("persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        std::fs::write(
            &toml,
            format!("relationship = [\"../../../../etc/passwd\"]\n{text}"),
        )
        .unwrap();
        let e = load_version(&dir, "mara", &pinned.digest).unwrap_err();
        assert!(
            format!("{e:#}").contains("snapshot's `relationship`"),
            "{e:#}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// `registry_for` refuses `egress >= Chosen`; that is exact while
    /// `Chosen` is the top class. The `match` is exhaustive, so a class
    /// added to `Egress` is a compile error here until someone places it —
    /// a list of the three today could not fail (review of #409).
    #[test]
    fn chosen_is_the_widest_egress_class() {
        fn rank(e: Egress) -> u8 {
            match e {
                Egress::None => 0,
                Egress::Blind => 1,
                Egress::Chosen => 2,
            }
        }
        for e in [Egress::None, Egress::Blind, Egress::Chosen] {
            assert!(e <= Egress::Chosen);
            assert!(rank(e) <= rank(Egress::Chosen));
            // `Ord` agrees with the rank, so `>=` means what it says.
            assert_eq!(e >= Egress::Chosen, rank(e) >= rank(Egress::Chosen));
        }
    }

    #[test]
    fn an_unapproved_persona_cannot_be_pinned() {
        let dir = scratch();
        let p = super::super::create(
            &dir,
            &no_lib(),
            super::super::NewPersona {
                name: "ada".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.state.status, Status::Candidate);
        assert!(format!("{}", pin(&dir, "ada").unwrap_err()).contains("not approved"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_config_turns_off_what_reads_the_owners_stores() {
        let base = AgentConfig {
            system_prompt: Some("assistant + charter + rules".into()),
            goal_guidance: true,
            situation_brief: true,
            past_appraisals: true,
            success_examples: true,
            contrast_evidence: true,
            sensors_in_brief: true,
            appraisals_in_brief: true,
            boredom: true,
            step_checks: true,
            step_escalation: true,
            max_turns: 7,
            cache_prompt: true,
            ..AgentConfig::default()
        };
        let c = agent_config(&base, "persona".into());
        assert_eq!(c.system_prompt.as_deref(), Some("persona"));
        assert!(c.system_prompt_file.is_none());
        assert!(
            !(c.goal_guidance
                || c.situation_brief
                || c.past_appraisals
                || c.success_examples
                || c.contrast_evidence
                || c.sensors_in_brief
                || c.appraisals_in_brief
                || c.boredom
                || c.step_checks
                || c.step_escalation)
        );
        assert_eq!((c.max_turns, c.cache_prompt), (7, true));
        let s = security(&SecurityConfig {
            block_sends_after_private: true,
            ..SecurityConfig::default()
        });
        assert!(!s.block_sends_after_private);
    }
}
