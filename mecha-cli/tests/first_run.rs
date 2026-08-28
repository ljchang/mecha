//! What a brand-new user actually walks into.
//!
//! **Nothing is more expensive than an install that does not work**, and it
//! is the one path the rest of the suite cannot see: every unit test starts
//! from a `Config` somebody constructed, and every integration test in
//! `mecha-core` starts from a fixture. Neither runs the binary the way a
//! person runs it on the day they first hear about this — no config, no
//! credentials, no stores, nothing on disk at all.
//!
//! So these drive the **real binary** against an isolated `MECHA_HOME`, and
//! assert the first-run contract rather than any one message: that it starts,
//! that it says what it needs in a shape a script can read, that the
//! credential-free commands work with nothing configured, and that a person
//! who says "no thanks" to an integration is not asked again forever.
//!
//! Three rules these keep, each of which is why a previous version of some
//! test elsewhere was useless:
//!
//! - **No network, ever.** The default provider is not `local`, so `setup`'s
//!   one HTTP call (`GET /props`) is not on this path. A first-run test that
//!   needed a server would be skipped on exactly the machines it is for.
//! - **Nothing depends on the developer's machine.** Provider credentials are
//!   removed from the child's environment, and every assertion about
//!   *status* is confined to steps whose answer comes from `MECHA_HOME`
//!   (which is empty and ours) rather than from `PATH` (which is not) — a
//!   contributor with `mecha-mail` installed and one without must both pass.
//! - **The home is unique per test.** These write files; sharing a directory
//!   would make them order-dependent, which is the kind of flake that gets a
//!   suite ignored.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A `MECHA_HOME` nobody else is using, plus a working directory that does
/// **not** contain it — removed when the test ends.
///
/// The two are siblings on purpose. `work.rs` refuses a workspace containing
/// the mecha home, because a path jail rooted over `~/.mecha` would cover the
/// OAuth tokens, every transcript and the learning store — so a test that
/// left the child in whatever directory cargo happened to run it from would
/// pass or fail on where `TMPDIR` points, which is not a property of the code
/// under test. Naming the working directory also makes these tests run the
/// way the docs tell a person to run mecha: from a project directory.
struct Home {
    root: PathBuf,
    home: PathBuf,
    work: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Home {
        let root =
            std::env::temp_dir().join(format!("mecha-first-run-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, work) = (root.join("home"), root.join("work"));
        std::fs::create_dir_all(&home).expect("creating an isolated MECHA_HOME");
        std::fs::create_dir_all(&work).expect("creating a workspace beside it");
        Home { root, home, work }
    }
    fn path(&self) -> &Path {
        &self.home
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Run the real `mecha` binary against `home`.
///
/// `CARGO_BIN_EXE_mecha` is cargo's own path to the binary this crate built,
/// so there is no question of testing a stale install — the thing under test
/// is the thing that was just compiled, which is the distinction
/// `CLAUDE.md`'s "a fresh mtime is not a fresh build" rule is about.
fn mecha(home: &Home, args: &[&str]) -> Output {
    run(home, args, false)
}

/// The same, with a credential in the environment — for the one test whose
/// subject is what a *finished* install looks like.
fn mecha_with_key(home: &Home, args: &[&str]) -> Output {
    run(home, args, true)
}

fn run(home: &Home, args: &[&str], with_key: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mecha"));
    cmd.args(args)
        .current_dir(&home.work)
        .env("MECHA_HOME", home.path())
        // A developer's own key would make `provider-credential` disappear
        // and the test pass for the wrong reason on their machine and fail
        // in CI. Removed rather than blanked: an empty value is a
        // *configured* empty on some code paths.
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        // Nothing here should ever open an editor. If a code path tries,
        // `true` exits 0 having touched nothing, so the test reports the
        // wrong outcome rather than hanging a CI job forever.
        .env("VISUAL", "true")
        .env("EDITOR", "true");
    if with_key {
        // Never used to reach anything: `setup` only checks that the
        // variable the provider names resolves, and the default provider is
        // not `local`, so nothing on this path opens a socket.
        cmd.env("ANTHROPIC_API_KEY", "not-a-real-key");
    }
    cmd.output().expect("running the mecha binary")
}

fn steps(out: &Output) -> Vec<serde_json::Value> {
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("`mecha setup --json` did not emit JSON ({e}):\n{text}"))
}

fn step<'a>(steps: &'a [serde_json::Value], id: &str) -> &'a serde_json::Value {
    steps
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no step `{id}` in {steps:#?}"))
}

/// The whole point: a person with nothing configured runs one command and is
/// told what to do, in a shape both they and a script can read.
#[test]
fn a_fresh_install_says_what_it_needs() {
    let home = Home::new("needs");
    let out = mecha(&home, &["setup", "--json"]);
    assert!(
        out.status.success(),
        "`mecha setup --json` failed on a fresh install:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let steps = steps(&out);

    // Every step a new install is entitled to be told about. Asserted by id
    // rather than by count, so adding one is not a test change and dropping
    // one is.
    for id in [
        "provider-credential",
        "mail",
        "docs",
        "slack",
        "graph",
        "charter",
    ] {
        let s = step(&steps, id);
        assert!(
            s["title"].as_str().is_some_and(|t| !t.is_empty()),
            "`{id}` has no title"
        );
        assert!(
            s["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "`{id}` says nothing about itself"
        );
    }

    // With no key in the environment and no local server configured, the
    // one step that blocks every other must be named first — a new user
    // wiring up mail against a provider that cannot answer is an hour spent
    // on the wrong end.
    assert_eq!(
        steps[0]["id"], "provider-credential",
        "the blocking step comes first: {steps:#?}"
    );
}

/// The gap this whole change closes: a fresh install was never told the
/// charter exists. `doctor` returns early on a file that is absent (right —
/// never having written one is not a fault), so nothing named the feature
/// to the person it is for.
#[test]
fn a_fresh_install_is_told_about_its_charter() {
    let home = Home::new("charter-offer");
    let charter = step(&steps(&mecha(&home, &["setup", "--json"])), "charter").clone();

    assert_eq!(charter["status"], "missing");
    assert_eq!(
        charter["remedy"]["argv"],
        serde_json::json!(["mecha", "charter", "edit"])
    );
    // The offer describes what a charter is; it must not read as a
    // suggested priority. A priority mecha proposed is one a model could
    // later argue from.
    let detail = charter["detail"].as_str().unwrap();
    assert!(
        detail.contains("in your own words"),
        "the offer should say whose words these are: {detail}"
    );
}

/// `mecha charter edit` creates the template and **authors no priority**.
/// This is the invariant every charter surface exists to keep, asserted here
/// against the real binary rather than against a helper.
#[test]
fn charter_edit_creates_a_template_that_holds_no_priorities() {
    let home = Home::new("charter-edit");
    let path = home.path().join("charter.toml");
    assert!(!path.exists(), "the fixture starts with no charter");

    // `$EDITOR` is `true`: it exits 0 having changed nothing, so what lands
    // on disk is exactly what mecha wrote and nothing a person typed.
    let out = mecha(&home, &["charter", "edit"]);
    assert!(
        out.status.success(),
        "`mecha charter edit` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let body = std::fs::read_to_string(&path).expect("the template was created");
    for line in body.lines() {
        assert!(
            line.trim().is_empty() || line.trim_start().starts_with('#'),
            "mecha wrote an uncommented line into a charter: {line:?}"
        );
    }

    // And a run would get nothing from it — a template, not a starter set of
    // opinions. `mecha charter` must say so rather than failing.
    let show = mecha(&home, &["charter"]);
    assert!(
        show.status.success(),
        "`mecha charter` failed on a template-only file:\n{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let json = mecha(&home, &["charter", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).expect("charter --json");
    assert_eq!(v["lines"].as_array().map(Vec::len), Some(0));
    assert_eq!(v["exists"], true, "the file is there, it just has no lines");
}

/// A charter that does not parse must fail loudly rather than degrading a
/// person to un-chartered runs silently.
#[test]
fn a_charter_that_does_not_load_exits_nonzero_and_says_why() {
    let home = Home::new("charter-broken");
    // `[[lines]]`, not `[[line]]` — the typo `deny_unknown_fields` turns
    // into a load error rather than a silently empty charter.
    std::fs::write(home.path().join("charter.toml"), "[[lines]]\nid = \"a\"\n").unwrap();

    let out = mecha(&home, &["charter"]);
    assert!(!out.status.success(), "a broken charter must not exit 0");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("did not load"),
        "say what happened: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Even failing, `--json` puts the error in the payload — a scripted
    // consumer gets the reason, not just a status code.
    let json = mecha(&home, &["charter", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).expect("charter --json");
    assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()));

    // And setup reports it as a fault rather than as something absent.
    assert_eq!(
        step(&steps(&mecha(&home, &["setup", "--json"])), "charter")["status"],
        "wrong"
    );
}

/// Declining is remembered, and a declined step stops being outstanding —
/// the difference between a finished install and a permanent defect list.
#[test]
fn a_declined_step_is_remembered_and_stops_being_outstanding() {
    let home = Home::new("decline");
    let before = steps(&mecha(&home, &["setup", "--json"]));
    assert_eq!(step(&before, "slack")["status"], "missing");
    assert!(step(&before, "slack")["remedy"].is_object());

    std::fs::write(
        home.path().join("setup-declined.json"),
        r#"{"declined": ["slack"]}"#,
    )
    .unwrap();

    let after = steps(&mecha(&home, &["setup", "--json"]));
    let slack = step(&after, "slack");
    assert_eq!(slack["status"], "declined");
    assert!(
        slack["remedy"].is_null(),
        "a remedy is an offer, and this one has been answered"
    );
    assert_eq!(
        slack["detail"],
        step(&before, "slack")["detail"],
        "a decline changes whether a step is asked for, never what it says"
    );
}

/// The end-to-end shape of the feature: once every outstanding step has been
/// answered — done or declined — a scripted `mecha setup` exits **0**.
///
/// Before `Declined` existed this was unreachable for anybody who did not
/// want all four integrations, so a setup check in somebody's own CI was
/// permanently red over choices they had already made.
#[test]
fn setup_exits_zero_once_everything_outstanding_has_been_answered() {
    let home = Home::new("all-declined");

    let out = mecha_with_key(&home, &["setup"]);
    assert!(
        !out.status.success(),
        "a fresh install has outstanding work and must exit non-zero"
    );

    // A credential is in the environment, so what remains outstanding is
    // exactly the optional half — which is the state this test is about.
    // Which steps those are depends on the machine (`mecha-mail` may or may
    // not be on PATH), so they are read off the run rather than hardcoded.
    let missing: Vec<String> = steps(&mecha_with_key(&home, &["setup", "--json"]))
        .iter()
        .filter(|s| s["status"] == "missing")
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!missing.is_empty(), "a fresh install has missing steps");
    assert!(
        !missing.contains(&"provider-credential".to_string()),
        "a key is in the environment, so the provider step is satisfied"
    );
    std::fs::write(
        home.path().join("setup-declined.json"),
        serde_json::json!({ "declined": missing }).to_string(),
    )
    .unwrap();

    let out = mecha_with_key(&home, &["setup"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "an install whose every open question has been answered must exit 0:\n{text}"
    );
    assert!(text.contains("Nothing outstanding"), "and say so:\n{text}");
    // The way back is printed rather than left to be discovered.
    assert!(
        text.contains("--undecline"),
        "a decision nobody can find their way back out of is one to hesitate over:\n{text}"
    );
}

/// **The step that makes everything else work cannot be declined**, not even
/// by editing the store by hand.
///
/// Found by running the flow: `declinable` was inferred from "missing", and a
/// provider with no credential is missing — so declining everything reported
/// `Nothing outstanding.` on an install that could not answer a prompt. The
/// worst possible failure for this feature, because it turns a checklist that
/// nags into a checklist that lies.
#[test]
fn a_credential_cannot_be_declined_even_by_editing_the_store() {
    let home = Home::new("undeclinable");
    std::fs::write(
        home.path().join("setup-declined.json"),
        r#"{"declined": ["provider-credential", "mail", "docs", "slack", "graph", "charter"]}"#,
    )
    .unwrap();

    let s = steps(&mecha(&home, &["setup", "--json"]));
    assert_eq!(
        step(&s, "provider-credential")["status"],
        "missing",
        "a credential is not a feature somebody can decline"
    );
    // The genuinely optional ones still honour it, so this cannot pass on a
    // decline that never worked at all.
    assert_eq!(step(&s, "slack")["status"], "declined");

    // And the install is still reported as unfinished.
    assert!(
        !mecha(&home, &["setup"]).status.success(),
        "an install that cannot answer a prompt is not a finished one"
    );
}

/// The offer text is prose, not a wrapped source literal. A run of spaces in
/// the middle of a sentence reads as a bug to the one person least able to
/// tell it is cosmetic.
#[test]
fn no_step_detail_carries_its_source_indentation() {
    let home = Home::new("prose");
    for s in steps(&mecha(&home, &["setup", "--json"])) {
        let detail = s["detail"].as_str().unwrap();
        assert!(
            !detail.contains("   "),
            "`{}` carries a run of spaces from its source literal: {detail:?}",
            s["id"]
        );
    }
}

/// `--undecline` puts a step back, so "never" is a preference rather than a
/// door that locks behind you.
#[test]
fn undecline_puts_a_step_back() {
    let home = Home::new("undecline");
    std::fs::write(
        home.path().join("setup-declined.json"),
        r#"{"declined": ["slack", "docs"]}"#,
    )
    .unwrap();

    assert!(mecha(&home, &["setup", "--undecline", "slack"])
        .status
        .success());
    let s = steps(&mecha(&home, &["setup", "--json"]));
    assert_eq!(step(&s, "slack")["status"], "missing");
    assert_eq!(step(&s, "docs")["status"], "declined", "one at a time");

    assert!(mecha(&home, &["setup", "--undecline", "all"])
        .status
        .success());
    let s = steps(&mecha(&home, &["setup", "--json"]));
    assert_eq!(step(&s, "docs")["status"], "missing");
}

/// An unreadable decline store is a read failure, not an empty list — and it
/// is said out loud, because a checklist that had quietly stopped honouring
/// your answers is the worse half of this feature.
#[test]
fn an_unreadable_decline_store_says_so_rather_than_forgetting_silently() {
    let home = Home::new("decline-broken");
    std::fs::write(home.path().join("setup-declined.json"), "{not json").unwrap();

    let out = mecha(&home, &["setup"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("could not be read"),
        "a store that could not be read must not read as an empty one:\n{text}"
    );
}

/// The cheapest end-to-end check there is: the binary starts and the tool
/// registry builds with **no provider configured at all**. This is the
/// command a new user is pointed at first, so it must work before anything
/// else does.
#[test]
fn tools_runs_with_nothing_configured() {
    let home = Home::new("tools");
    let out = mecha(&home, &["tools"]);
    assert!(
        out.status.success(),
        "`mecha tools` must work before any credential exists:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json = mecha(&home, &["tools", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).expect("tools --json");
    // Named rather than "some array somewhere": an `||` over two shapes
    // passes whichever one the code drifts to, which is a test that stops
    // asserting the moment it would matter.
    let tools = v.as_array().expect("`mecha tools --json` emits an array");
    assert!(!tools.is_empty(), "the registry came back empty");
    assert!(
        tools.iter().any(|t| t["name"] == "fs_read"),
        "the builtin tools are what work without a provider: {tools:#?}"
    );
}

/// `doctor` reads every store in one pass, and on a fresh install every one
/// of them is absent. Absent must not read as broken, or the first thing a
/// new user sees is a wall of alarm about stores nobody has created yet.
#[test]
fn doctor_is_quiet_about_stores_that_do_not_exist_yet() {
    let home = Home::new("doctor");
    let out = mecha(&home, &["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("`mecha doctor --json` did not emit JSON ({e})"));
    // `unwrap_or_default()` here would turn a shape change into a silent
    // pass, since an empty list satisfies the assertion below trivially.
    let findings = v.as_array().expect("`mecha doctor --json` emits an array");
    let broken: Vec<_> = findings
        .iter()
        .filter(|f| f["severity"] == "broken")
        .collect();
    assert!(
        broken.is_empty(),
        "a fresh install has nothing broken about it, only things absent: {broken:#?}"
    );
}

/// And the trap a new user is most likely to walk into unaided: starting
/// mecha from the directory that holds `~/.mecha`.
///
/// It must **refuse**, because a path jail rooted there covers the OAuth
/// tokens, every session transcript and the learning store. Asserted here
/// rather than only in `work.rs`'s unit test because the thing that matters
/// is that a person meets a sentence explaining it, not that a function
/// returns `Err`.
#[test]
fn a_workspace_containing_the_mecha_home_is_refused_with_a_reason() {
    let home = Home::new("jail");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mecha"));
    let out = cmd
        .args(["tools"])
        // The parent of both: it contains the mecha home, which is exactly
        // the shape `mecha chat` from `$HOME` used to have.
        .current_dir(&home.root)
        .env("MECHA_HOME", home.path())
        .env_remove("ANTHROPIC_API_KEY")
        .output()
        .expect("running the mecha binary");

    assert!(!out.status.success(), "a jail over ~/.mecha must not start");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("contains the mecha home"),
        "explain it rather than just failing: {said}"
    );
    assert!(
        said.contains("project directory"),
        "and say what to do instead: {said}"
    );
}
