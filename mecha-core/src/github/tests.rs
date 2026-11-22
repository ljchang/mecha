//! The collector against a fake `gh`: a shell script that answers canned
//! JSON by the arguments it is called with, and logs every call. Fixtures are
//! made up (`example-lab/...`); no real account, person or repository.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory: the mecha home is `<dir>/home`, the fake at `<dir>/gh`.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mecha-github-{name}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).unwrap();
    dir
}

/// One canned answer: when `$*` matches the sh glob `pattern`, print `out`
/// on stdout and `err` on stderr and exit `code`.
struct Case {
    pattern: String,
    out: String,
    err: String,
    code: i32,
}

fn ok(pattern: &str, out: &str) -> Case {
    Case {
        pattern: pattern.into(),
        out: out.into(),
        err: String::new(),
        code: 0,
    }
}

fn fails(pattern: &str, out: &str, err: &str, code: i32) -> Case {
    Case {
        pattern: pattern.into(),
        out: out.into(),
        err: err.into(),
        code,
    }
}

/// Write the fake `gh` into `dir` and return a [`Gh`] that runs it.
fn fake(dir: &Path, cases: &[Case]) -> Gh {
    let mut script = String::from("#!/bin/sh\n");
    script.push_str(&format!(
        "printf '%s' \"$*\" | tr '\\n' ' ' >> '{d}/calls.log'; echo >> '{d}/calls.log'\n",
        d = dir.display()
    ));
    script.push_str("case \"$*\" in\n");
    for (i, c) in cases.iter().enumerate() {
        std::fs::write(dir.join(format!("{i}.out")), &c.out).unwrap();
        std::fs::write(dir.join(format!("{i}.err")), &c.err).unwrap();
        script.push_str(&format!(
            "  {p}) cat '{d}/{i}.out'; cat '{d}/{i}.err' >&2; exit {code} ;;\n",
            p = c.pattern,
            d = dir.display(),
            code = c.code
        ));
    }
    script.push_str("  *) echo \"fake gh: no case for $*\" >&2; exit 99 ;;\nesac\n");
    let bin = dir.join("gh");
    std::fs::write(&bin, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::new(bin)
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn select(dir: &Path, toml: &str) {
    let home = dir.join("home");
    std::fs::create_dir_all(super::dir(&home)).unwrap();
    std::fs::write(selection_path(&home), toml).unwrap();
}

const LAB_ALL: &str = r#"
[[owner]]
login = "example-lab"
include = "all"
forks = false
archived = false
"#;

const RATE: &str = r#"{"resources":{"core":{"limit":5000,"remaining":4990,"reset":1800000000,"used":10},"graphql":{"limit":5000,"remaining":4995,"reset":1800000000,"used":5}}}"#;

fn repo_json(full: &str, fork: bool, archived: bool) -> String {
    format!(
        r#"{{"full_name":"{full}","fork":{fork},"archived":{archived},"private":false,"default_branch":"main","pushed_at":"2026-11-01T00:00:00Z"}}"#
    )
}

/// Two pages, back to back, as `gh api --paginate` prints them: two watched
/// repositories, a fork, an archived one, and an owner the file does not name.
fn listing() -> String {
    format!(
        "[{},{},{}]\n[{},{}]\n",
        repo_json("example-lab/tool-a", false, false),
        repo_json("example-lab/fork-of-upstream", true, false),
        repo_json("example-lab/old-site", false, true),
        repo_json("example-lab/tool-b", false, false),
        repo_json("example-owner/notes", false, false),
    )
}

fn node(number: i64, title: &str, author: &str, association: &str) -> String {
    format!(
        r#"{{"number":{number},"title":"{title}","createdAt":"2026-11-10T00:00:00Z","updatedAt":"2026-11-11T00:00:00Z","authorAssociation":"{association}","author":{{"login":"{author}"}},"comments":{{"totalCount":2}},"labels":{{"nodes":[{{"name":"bug"}}]}}}}"#
    )
}

fn repo_data(stars: i64, issues: &[String], prs: &[String]) -> String {
    format!(
        r#"{{"stargazerCount":{stars},"forkCount":3,"watchers":{{"totalCount":4}},"issues":{{"totalCount":{ni},"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[{issues}]}},"pullRequests":{{"totalCount":{np},"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[{prs}]}}}}"#,
        ni = issues.len(),
        np = prs.len(),
        issues = issues.join(","),
        prs = prs.join(",")
    )
}

/// tool-a: an issue a member opened, and three pull requests — one from a
/// contributor, one from a member, one from a first-time contributor.
fn tool_a() -> String {
    repo_data(
        12,
        &[node(1, "Crash on empty input", "lab-member", "MEMBER")],
        &[
            node(7, "Fix the parser", "outside-dev", "CONTRIBUTOR"),
            node(8, "Bump the version", "lab-member", "MEMBER"),
            node(9, "Add a test", "first-timer", "FIRST_TIME_CONTRIBUTOR"),
        ],
    )
}

fn batch(repos: &[String]) -> String {
    let fields: Vec<String> = repos
        .iter()
        .enumerate()
        .map(|(i, r)| format!(r#""r{i}":{r}"#))
        .collect();
    format!(r#"{{"data":{{{}}}}}"#, fields.join(","))
}

fn run_json(id: i64, conclusion: &str, branch: &str, from: &str) -> String {
    format!(
        r#"{{"id":{id},"name":"CI","workflow_id":5,"run_number":{id},"run_attempt":1,"event":"push","status":"completed","conclusion":"{conclusion}","head_sha":"abc{id}","head_branch":"{branch}","created_at":"2026-11-17T10:00:00Z","run_started_at":"2026-11-17T10:00:01Z","updated_at":"2026-11-17T10:05:00Z","head_repository":{{"full_name":"{from}"}}}}"#
    )
}

/// tool-a's runs: two of its own on main, one from a fork's branch also
/// called main, one on another branch.
fn tool_a_runs() -> String {
    format!(
        r#"{{"total_count":4,"workflow_runs":[{},{},{},{}]}}"#,
        run_json(101, "success", "main", "example-lab/tool-a"),
        run_json(102, "failure", "main", "example-lab/tool-a"),
        run_json(103, "failure", "main", "someone-else/tool-a"),
        run_json(104, "failure", "feature", "example-lab/tool-a"),
    )
}

const NO_RUNS: &str = r#"{"total_count":0,"workflow_runs":[]}"#;

/// The cases a healthy run answers, before any test's own.
fn healthy() -> Vec<Case> {
    vec![
        ok("*'api rate_limit'*", RATE),
        ok("*'user/repos'*", &listing()),
        ok(
            "*'api graphql'*'n0=tool-a'*'n1=tool-b'*",
            &batch(&[tool_a(), repo_data(1, &[], &[])]),
        ),
        ok("*'repos/example-lab/tool-a/actions/runs'*", &tool_a_runs()),
        ok("*'repos/example-lab/tool-b/actions/runs'*", NO_RUNS),
    ]
}

/// Put `first` ahead of the healthy cases, so it answers instead.
fn with(first: Vec<Case>) -> Vec<Case> {
    first.into_iter().chain(healthy()).collect()
}

fn now() -> DateTime<Utc> {
    "2026-11-18T12:00:00Z".parse().unwrap()
}

fn db(dir: &Path) -> Connection {
    Connection::open(db_path(&dir.join("home"))).unwrap()
}

fn strings(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn kinds(conn: &Connection) -> Vec<String> {
    strings(
        conn,
        "SELECT kind || ':' || coalesce(step, '') || ':' || coalesce(repo, '') FROM collect_log WHERE kind <> 'finished' ORDER BY id",
    )
}

fn collect_and_record(dir: &Path, gh: &Gh, at: DateTime<Utc>) -> Collected {
    let home = dir.join("home");
    let c = collect(gh, &home, at);
    record(&db_path(&home), &c).unwrap();
    c
}

// ---- a healthy run ----

#[test]
fn a_healthy_run_fills_every_table_and_logs_one_finished_row() {
    let dir = scratch("healthy");
    select(&dir, LAB_ALL);
    let c = collect_and_record(&dir, &fake(&dir, &healthy()), now());
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    let conn = db(&dir);
    assert_eq!(
        strings(&conn, "SELECT repo FROM inventory ORDER BY repo").len(),
        5,
        "the inventory holds every repository the login sees, selected or not"
    );
    assert_eq!(
        strings(&conn, "SELECT repo FROM counts ORDER BY repo"),
        ["example-lab/tool-a", "example-lab/tool-b"]
    );
    let (selected, collected, kind): (i64, i64, String) = conn
        .query_row(
            "SELECT selected, collected, kind FROM collect_log",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((selected, collected, kind.as_str()), (2, 2, "finished"));
    let stars: Option<i64> = conn
        .query_row(
            "SELECT stars FROM counts WHERE repo = 'example-lab/tool-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stars, Some(12));
}

#[test]
fn a_fork_is_listed_and_never_watched() {
    let dir = scratch("fork");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let conn = db(&dir);
    let selected: bool = conn
        .query_row(
            "SELECT selected FROM inventory WHERE repo = 'example-lab/fork-of-upstream'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!selected, "forks are excluded by default (G4)");
    assert!(
        calls(&dir).iter().all(|c| !c.contains("fork-of-upstream")),
        "nothing is asked about a fork: {:?}",
        calls(&dir)
    );
}

#[test]
fn an_archived_repository_is_listed_and_never_watched() {
    let dir = scratch("archived");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT repo FROM inventory WHERE selected = 1 ORDER BY repo"
        ),
        ["example-lab/tool-a", "example-lab/tool-b"],
        "not the archived one, the fork, or an owner the file does not name"
    );
    assert!(calls(&dir).iter().all(|c| !c.contains("old-site")));
    assert!(calls(&dir).iter().all(|c| !c.contains("example-owner")));
}

#[test]
fn an_outside_pull_request_is_kept_and_a_members_is_dropped() {
    let dir = scratch("outside-pr");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT number || ' ' || kind || ' ' || author_association FROM items WHERE repo = 'example-lab/tool-a' ORDER BY number"
        ),
        [
            // An issue is kept whoever opened it; G5 is about pull requests.
            "1 issue MEMBER",
            "7 pull_request CONTRIBUTOR",
            "9 pull_request FIRST_TIME_CONTRIBUTOR",
        ],
        "the member's pull request (#8) is not an item"
    );
}

#[test]
fn only_the_default_branchs_own_runs_are_kept() {
    let dir = scratch("ci");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT id || ' ' || conclusion FROM ci_runs ORDER BY id"
        ),
        ["101 success", "102 failure"],
        "not a fork's branch named main (103), not another branch (104)"
    );
    let asked = calls(&dir);
    let ci = asked
        .iter()
        .find(|c| c.contains("tool-a/actions/runs"))
        .unwrap();
    assert!(ci.contains("branch=main"), "{ci}");
}

// ---- refused and failed, kept apart ----

#[test]
fn a_rest_rate_limit_is_its_own_kind_and_stops_rest_calls() {
    let dir = scratch("rate-rest");
    select(&dir, LAB_ALL);
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'repos/example-lab/tool-a/actions/runs'*",
            r#"{"message":"API rate limit exceeded for user ID 1.","documentation_url":"https://docs.github.com/rest"}"#,
            "gh: API rate limit exceeded for user ID 1. (HTTP 403)\n",
            1,
        )]),
    );
    collect_and_record(&dir, &gh, now());
    let conn = db(&dir);
    assert_eq!(kinds(&conn), ["rate_limited:ci:"]);
    assert!(
        calls(&dir).iter().all(|c| !c.contains("tool-b/actions")),
        "a limited REST quota is not spent further"
    );
    // GraphQL is its own quota, and was read.
    assert_eq!(
        strings(&conn, "SELECT repo FROM counts ORDER BY repo").len(),
        2
    );
}

#[test]
fn a_graphql_rate_limit_is_its_own_kind() {
    let dir = scratch("rate-graphql");
    select(&dir, LAB_ALL);
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'api graphql'*",
            r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#,
            "gh: API rate limit exceeded for user ID 1.\n",
            1,
        )]),
    );
    collect_and_record(&dir, &gh, now());
    assert_eq!(kinds(&db(&dir)), ["rate_limited:items:"]);
}

/// A rate limit met on a follow-up page stops GraphQL for the rest of the
/// run, as the batch and alias paths do: tool-b's own further page is
/// never asked for.
#[test]
fn a_rate_limit_on_a_follow_up_page_stops_graphql() {
    let dir = scratch("rate-page");
    select(&dir, LAB_ALL);
    let paged = |data: String, cursor: &str| {
        data.replacen(
            r#""hasNextPage":false,"endCursor":null"#,
            &format!(r#""hasNextPage":true,"endCursor":"{cursor}""#),
            1,
        )
    };
    let gh = fake(
        &dir,
        &with(vec![
            fails(
                "*'api graphql'*'after=c1'*",
                r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#,
                "gh: API rate limit exceeded for user ID 1.\n",
                1,
            ),
            ok(
                "*'api graphql'*'after=c2'*",
                r#"{"data":{"repository":null}}"#,
            ),
            ok(
                "*'api graphql'*'n0=tool-a'*'n1=tool-b'*",
                &batch(&[paged(tool_a(), "c1"), paged(repo_data(1, &[], &[]), "c2")]),
            ),
        ]),
    );
    collect_and_record(&dir, &gh, now());
    assert!(
        !calls(&dir).iter().any(|c| c.contains("after=c2")),
        "GraphQL was spent after GitHub refused it: {:?}",
        calls(&dir)
    );
    assert!(kinds(&db(&dir)).contains(&"rate_limited:items:example-lab/tool-a".to_string()));
}

#[test]
fn an_absent_login_is_auth_and_the_tables_keep_the_last_answer() {
    let dir = scratch("auth");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let _ = std::fs::remove_file(dir.join("calls.log"));
    // What gh 2.45 says with no login (measured): exit 4.
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'api rate_limit'*",
            "",
            "To get started with GitHub CLI, please run:  gh auth login\n\
             Alternatively, populate the GH_TOKEN environment variable with a GitHub API authentication token.\n",
            4,
        )]),
    );
    collect_and_record(&dir, &gh, now() + Duration::minutes(10));
    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT kind FROM collect_log WHERE run = '2026-11-18T12:10:00.000Z' ORDER BY id"
        ),
        ["auth", "finished"]
    );
    assert_eq!(calls(&dir).len(), 1, "nothing is asked after the preflight");
    assert_eq!(
        strings(&conn, "SELECT repo FROM inventory").len(),
        5,
        "the last inventory is kept, not emptied"
    );
    assert_eq!(
        strings(&conn, "SELECT CAST(number AS TEXT) FROM items").len(),
        3
    );
}

#[test]
fn a_refused_token_is_auth_too() {
    let dir = scratch("auth-401");
    select(&dir, LAB_ALL);
    // What gh 2.45 says with a token GitHub refuses (measured).
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'api rate_limit'*",
            r#"{"message":"Bad credentials","documentation_url":"https://docs.github.com/rest","status":"401"}"#,
            "gh: Bad credentials (HTTP 401)\n",
            1,
        )]),
    );
    collect_and_record(&dir, &gh, now());
    assert_eq!(kinds(&db(&dir)), ["auth:preflight:"]);
}

#[test]
fn a_missing_gh_is_its_own_kind() {
    let dir = scratch("missing");
    select(&dir, LAB_ALL);
    let gh = Gh::new(dir.join("no-such-gh"));
    collect_and_record(&dir, &gh, now());
    assert_eq!(kinds(&db(&dir)), ["gh_missing:preflight:"]);
}

#[test]
fn a_spent_budget_is_out_of_time_and_starts_nothing() {
    let dir = scratch("budget");
    select(&dir, LAB_ALL);
    let gh = fake(&dir, &healthy()).with_budget(StdDuration::ZERO);
    collect_and_record(&dir, &gh, now());
    assert_eq!(kinds(&db(&dir)), ["out_of_time:preflight:"]);
    assert!(calls(&dir).is_empty());
}

#[test]
fn a_named_repository_that_is_not_found_has_vanished() {
    let dir = scratch("vanished-named");
    select(
        &dir,
        &format!("{LAB_ALL}\n[[repo]]\nname = \"example-other/borrowed\"\ninclude = true\n"),
    );
    let gh = fake(
        &dir,
        &with(vec![fails(
            "'api repos/example-other/borrowed'",
            r#"{"message":"Not Found","documentation_url":"https://docs.github.com/rest","status":"404"}"#,
            "gh: Not Found (HTTP 404)\n",
            1,
        )]),
    );
    collect_and_record(&dir, &gh, now());
    assert_eq!(
        kinds(&db(&dir)),
        ["vanished:inventory:example-other/borrowed"]
    );
}

#[test]
fn a_repository_graphql_cannot_resolve_has_vanished() {
    let dir = scratch("vanished-graphql");
    select(&dir, LAB_ALL);
    let answer = format!(
        r#"{{"data":{{"r0":{},"r1":null}},"errors":[{{"type":"NOT_FOUND","path":["r1"],"message":"Could not resolve to a Repository with the name 'example-lab/tool-b'."}}]}}"#,
        tool_a()
    );
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'api graphql'*",
            &answer,
            "gh: Could not resolve to a Repository with the name 'example-lab/tool-b'.\n",
            1,
        )]),
    );
    collect_and_record(&dir, &gh, now());
    let conn = db(&dir);
    assert_eq!(kinds(&conn), ["vanished:items:example-lab/tool-b"]);
    assert_eq!(
        strings(&conn, "SELECT repo FROM counts"),
        ["example-lab/tool-a"],
        "the batch's other repository is still read"
    );
}

#[test]
fn a_watched_repository_gone_from_the_listing_has_vanished() {
    let dir = scratch("vanished-listing");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let shorter = format!("[{}]\n", repo_json("example-lab/tool-a", false, false));
    let gh = fake(
        &dir,
        &with(vec![
            ok("*'user/repos'*", &shorter),
            ok("*'api graphql'*'n0=tool-a'*", &batch(&[tool_a()])),
        ]),
    );
    collect_and_record(&dir, &gh, now() + Duration::minutes(10));
    assert_eq!(kinds(&db(&dir)), ["vanished:inventory:example-lab/tool-b"]);
}

#[test]
fn a_listing_that_fails_keeps_the_inventory_and_still_watches_from_it() {
    let dir = scratch("listing-fails");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'user/repos'*",
            "",
            "error connecting to api.github.com\n",
            1,
        )]),
    );
    let c = collect_and_record(&dir, &gh, now() + Duration::minutes(10));
    let conn = db(&dir);
    assert_eq!(kinds(&conn), ["failed:inventory:"]);
    assert_eq!(
        c.selected,
        Some(2),
        "the last inventory decides what is watched"
    );
    assert_eq!(strings(&conn, "SELECT repo FROM inventory").len(), 5);
}

/// A listing that fails on a store with no earlier inventory leaves the
/// selection unknown, never zero: nothing said what the login can see, so
/// the doctor must not report that the selection matches nothing.
#[test]
fn a_listing_that_fails_on_a_new_store_leaves_the_selection_unknown() {
    let dir = scratch("listing-fails-fresh");
    select(&dir, LAB_ALL);
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'user/repos'*",
            "",
            "error connecting to api.github.com\n",
            1,
        )]),
    );
    let c = collect_and_record(&dir, &gh, now());
    assert_eq!(c.selected, None, "unknown, never zero");
    let selected: Option<i64> = db(&dir)
        .query_row(
            "SELECT selected FROM collect_log WHERE kind = 'finished'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(selected, None);
    let found = crate::doctor::check_github(&db_path(&dir.join("home")), now());
    assert!(
        !found.iter().any(|f| f.summary.contains("matched")),
        "{found:?}"
    );
}

#[test]
fn no_selection_file_selects_nothing_and_says_so() {
    let dir = scratch("no-selection");
    let c = collect_and_record(&dir, &fake(&dir, &healthy()), now());
    assert_eq!(c.selected, Some(0));
    let conn = db(&dir);
    assert_eq!(kinds(&conn), ["no_selection:selection:"]);
    assert_eq!(
        strings(&conn, "SELECT repo FROM inventory").len(),
        5,
        "the inventory is listed for the picker all the same"
    );
}

#[test]
fn a_selection_that_does_not_parse_selects_nothing_and_says_so() {
    let dir = scratch("bad-selection");
    select(&dir, "[[owner]]\nlogin = \"example-lab\"\nforkz = true\n");
    let c = collect_and_record(&dir, &fake(&dir, &healthy()), now());
    assert_eq!(c.selected, Some(0));
    assert_eq!(kinds(&db(&dir)), ["bad_selection:selection:"]);
    assert!(calls(&dir).iter().all(|c| !c.contains("graphql")));
}

// ---- the body: read, hashed, never written ----

/// The listing asks for each body; the store keeps its hash and never the
/// text, and the serialized shape of an item carries no body either.
#[test]
fn a_body_is_hashed_into_the_store_and_never_written() {
    let dir = scratch("body");
    select(&dir, LAB_ALL);
    let body = "Steps: run it on an empty file. It crashes. A marker: example-marker-7.";
    let issue = node(1, "Crash on empty input", "outside-dev", "NONE").replacen(
        '{',
        &format!("{{\"body\":{},", serde_json::json!(body)),
        1,
    );
    let cases = with(vec![ok(
        "*'api graphql'*'n0=tool-a'*'n1=tool-b'*",
        &batch(&[repo_data(1, &[issue], &[]), repo_data(1, &[], &[])]),
    )]);
    let c = collect_and_record(&dir, &fake(&dir, &cases), now());
    assert!(
        calls(&dir)
            .iter()
            .any(|l| l.contains("api graphql") && l.contains("title body ")),
        "the listing does not ask for bodies"
    );
    let item = &c.repos[0].items[0];
    assert_eq!(item.body.as_deref(), Some(body));
    assert_eq!(item.body_hash, Some(summary::body_hash(body)));
    assert!(!serde_json::to_string(item)
        .unwrap()
        .contains("example-marker-7"));
    let conn = db(&dir);
    assert_eq!(
        strings(&conn, "SELECT body_hash FROM items WHERE number = 1"),
        [summary::body_hash(body)]
    );
    let dump = std::fs::read(db_path(&dir.join("home"))).unwrap();
    assert!(
        !String::from_utf8_lossy(&dump).contains("example-marker-7"),
        "the body was written to the store"
    );
}

// ---- the answer's size, capped at the door ----

/// An answer past the cap is refused whole, never parsed cut short — even
/// where a prefix of it would parse (the first of `--paginate`'s pages).
#[test]
fn an_answer_past_the_cap_is_refused_not_read_cut_short() {
    let dir = scratch("too-large");
    let big = "x".repeat(8 * 1024);
    let page_one = format!("[{}]\n", repo_json("example-lab/tool-a", false, false));
    let page_two = format!(r#"[{{"full_name":"example-lab/{big}"}}]"#);
    let cases = vec![
        ok("*'user/repos'*", &format!("{page_one}{page_two}")),
        ok(
            "*'api graphql'*",
            &format!(r#"{{"data":{{"r0":"{big}"}}}}"#),
        ),
        ok("*'api rate_limit'*", RATE),
    ];
    let gh = fake(&dir, &cases).with_answer_cap(page_one.len() + 16);
    assert_eq!(
        gh.rest(&["api", "user/repos", "--paginate"]),
        Err(GhError::TooLarge(page_one.len() + 16)),
        "a prefix that parses was taken for the answer"
    );
    assert_eq!(
        gh.graphql("query { x }", &[]),
        Err(GhError::TooLarge(page_one.len() + 16))
    );
    // Under the cap, the same plumbing answers.
    assert!(gh.rate().is_ok());
    let p = GhError::TooLarge(ANSWER_BYTES_MAX).problem(Step::Items, None);
    assert_eq!(p.kind, Kind::Failed);
    assert!(p.detail.contains("32 MiB"), "{}", p.detail);
}

#[test]
fn the_drain_keeps_the_cap_and_reads_the_rest_away() {
    let text = vec![b'a'; 200_000];
    let (kept, over) = drain_bounded(&text[..], 1000);
    assert_eq!((kept.len(), over), (1000, true));
    let (kept, over) = drain_bounded(&text[..], 200_000);
    assert_eq!((kept.len(), over), (200_000, false));
}

/// A listing too large with its bodies is asked again without them: the
/// items land, unhashed, the repositories count as read in full, and the
/// summaries' loss is logged — the collection never pays for a long body.
#[test]
fn a_listing_too_large_with_bodies_is_read_again_without_them() {
    let dir = scratch("bodiless");
    select(&dir, LAB_ALL);
    let long = "a pasted log line\n".repeat(8_000);
    let issue = node(1, "Crash on empty input", "lab-member", "MEMBER").replacen(
        '{',
        &format!("{{\"body\":{},", serde_json::json!(long)),
        1,
    );
    let bodied = batch(&[repo_data(12, &[issue], &[]), repo_data(1, &[], &[])]);
    // The bodied query answers too large; the body-less one falls through
    // to the healthy batch.
    // A first, ordinary run lands the items; then item 1 has a known hash
    // and is unchanged since, and item 7 has a known hash but was updated
    // since (its stored updated_at is older than the listing's).
    collect_and_record(&dir, &fake(&dir, &healthy()), now() - Duration::hours(1));
    {
        let conn = db(&dir);
        conn.execute("UPDATE items SET body_hash = 'kept' WHERE number = 1", [])
            .unwrap();
        conn.execute(
            "UPDATE items SET body_hash = 'stale', updated_at = '2000-01-01T00:00:00Z' WHERE number = 7",
            [],
        )
        .unwrap();
    }
    let _ = std::fs::remove_file(dir.join("calls.log"));
    let cases = with(vec![ok("*'api graphql'*'title body '*", &bodied)]);
    let gh = fake(&dir, &cases).with_answer_cap(64 * 1024);
    let c = collect_and_record(&dir, &gh, now());

    let graphql: Vec<String> = calls(&dir)
        .into_iter()
        .filter(|l| l.contains("api graphql"))
        .collect();
    assert_eq!(graphql.len(), 2, "{graphql:?}");
    assert!(graphql[0].contains("title body "));
    assert!(!graphql[1].contains(" body "), "the retry asked for bodies");

    let items: Vec<i64> = c
        .repos
        .iter()
        .flat_map(|r| &r.items)
        .map(|i| i.number)
        .collect();
    assert_eq!(
        items,
        [1, 7, 9],
        "the body-less listing's items did not land"
    );
    assert!(c
        .repos
        .iter()
        .flat_map(|r| &r.items)
        .all(|i| i.body_hash.is_none()));
    assert_eq!(
        summary::unread(&c),
        3,
        "unread items are not counted waiting"
    );
    assert_eq!(
        c.collected(),
        2,
        "a body-less listing is a listing read in full"
    );
    let conn = db(&dir);
    // Unchanged since: its hash, and so its summary, stands. Updated since:
    // unknown, never the old body's. Never seen with a body: unknown.
    assert_eq!(
        strings(
            &conn,
            "SELECT number || ':' || coalesce(body_hash, '-') FROM items ORDER BY number"
        ),
        ["1:kept", "7:-", "9:-"]
    );
    assert_eq!(
        kinds(&conn),
        [
            "summary_skipped:items:example-lab/tool-a",
            "summary_skipped:items:example-lab/tool-b"
        ]
    );
    assert!(
        c.problems.iter().all(|p| p.kind.is_summary()),
        "{:?}",
        c.problems
    );
}

// ---- retention and closing ----

fn items_run(at: DateTime<Utc>, repo: &str, numbers: &[i64], complete: bool) -> Collected {
    let mut c = Collected::new(at);
    c.repos.push(RepoItems {
        repo: repo.into(),
        counts: Counts::default(),
        items: numbers
            .iter()
            .map(|&n| Item {
                number: n,
                kind: ItemKind::Issue,
                title: format!("issue {n}"),
                author: None,
                author_association: None,
                created_at: None,
                updated_at: None,
                labels: Vec::new(),
                comments: None,
                body: None,
                body_hash: None,
            })
            .collect(),
        complete,
    });
    c
}

#[test]
fn an_item_no_longer_open_is_kept_thirty_days_then_pruned() {
    let dir = scratch("retention-items");
    let db = db_path(&dir.join("home"));
    let day = |d: i64| now() - Duration::days(d);
    let mut first = items_run(day(40), "example-lab/tool-a", &[1, 2, 3], true);
    // A repository that stopped being collected: last seen 40 days ago.
    first.repos.push(
        items_run(day(40), "example-lab/tool-b", &[4], true)
            .repos
            .remove(0),
    );
    record(&db, &first).unwrap();
    record(
        &db,
        &items_run(day(31), "example-lab/tool-a", &[2, 3], true),
    )
    .unwrap();
    record(&db, &items_run(day(29), "example-lab/tool-a", &[3], true)).unwrap();
    record(&db, &items_run(now(), "example-lab/tool-a", &[3], true)).unwrap();
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        strings(
            &conn,
            "SELECT repo || ' #' || number || ' ' || coalesce(gone_at, 'open') FROM items ORDER BY repo, number"
        ),
        [
            "example-lab/tool-a #2 2026-10-20T12:00:00.000Z",
            "example-lab/tool-a #3 open",
        ],
        "#1 went 31 days ago and #4 was last seen 40 days ago; #2 went 29 days ago"
    );
}

#[test]
fn an_incomplete_listing_marks_nothing_closed() {
    let dir = scratch("incomplete");
    let db = db_path(&dir.join("home"));
    record(
        &db,
        &items_run(
            now() - Duration::hours(1),
            "example-lab/tool-a",
            &[1, 2],
            true,
        ),
    )
    .unwrap();
    record(&db, &items_run(now(), "example-lab/tool-a", &[2], false)).unwrap();
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        strings(
            &conn,
            "SELECT CAST(number AS TEXT) FROM items WHERE gone_at IS NULL ORDER BY number"
        )
        .len(),
        2,
        "#1 was not seen, but the listing was not read to its end"
    );
}

/// A listing cut short in a real run — a further page whose repository
/// came back null, as one does when it goes private between the batch and
/// the page — is a `failed` row naming the repo, and the repo is not counted
/// as read in full (review of #898). Unknown is never clean.
#[test]
fn a_listing_cut_short_is_said_and_not_counted_read_in_full() {
    let dir = scratch("cut-short");
    select(&dir, LAB_ALL);
    let more = tool_a().replacen(
        r#""hasNextPage":false,"endCursor":null"#,
        r#""hasNextPage":true,"endCursor":"c1""#,
        1,
    );
    let gh = fake(
        &dir,
        &with(vec![
            ok(
                "*'api graphql'*'after=c1'*",
                r#"{"data":{"repository":null}}"#,
            ),
            ok(
                "*'api graphql'*'n0=tool-a'*'n1=tool-b'*",
                &batch(&[more, repo_data(1, &[], &[])]),
            ),
        ]),
    );
    let c = collect_and_record(&dir, &gh, now());
    let cut: Vec<_> = c
        .problems
        .iter()
        .filter(|p| p.kind == Kind::Failed && p.repo.as_deref() == Some("example-lab/tool-a"))
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", c.problems);
    assert!(cut[0].detail.contains("cut short"), "{}", cut[0].detail);
    assert_eq!(c.collected(), 1, "tool-a was not read in full");
    assert_eq!(kinds(&db(&dir)), ["failed:items:example-lab/tool-a"]);
}

#[test]
fn workflow_runs_are_kept_ninety_days() {
    let dir = scratch("retention-ci");
    let db = db_path(&dir.join("home"));
    let mut c = Collected::new(now());
    for (id, days) in [(1, 91), (2, 89)] {
        c.ci.push(CiRun {
            id,
            repo: "example-lab/tool-a".into(),
            workflow: Some("CI".into()),
            workflow_id: None,
            run_number: None,
            attempt: None,
            event: None,
            status: None,
            conclusion: Some("failure".into()),
            head_sha: None,
            branch: Some("main".into()),
            created_at: ts(now() - Duration::days(days)),
            started_at: None,
            updated_at: None,
        });
    }
    record(&db, &c).unwrap();
    let conn = Connection::open(&db).unwrap();
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM ci_runs")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids, [2]);
}

#[test]
fn counts_are_written_once_a_day() {
    let dir = scratch("counts");
    let db = db_path(&dir.join("home"));
    let run = |at: DateTime<Utc>, stars: i64| {
        let mut c = items_run(at, "example-lab/tool-a", &[], true);
        c.repos[0].counts.stars = Some(stars);
        c
    };
    record(&db, &run(now(), 10)).unwrap();
    record(&db, &run(now() + Duration::minutes(10), 11)).unwrap();
    record(&db, &run(now() + Duration::days(1), 12)).unwrap();
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        strings(&conn, "SELECT day || ' ' || stars FROM counts ORDER BY day"),
        ["2026-11-18 10", "2026-11-19 12"],
        "today's row, once written, is kept"
    );
}

// ---- the selection ----

fn repo(full: &str, fork: bool, archived: bool) -> Repo {
    let (owner, name) = split_full_name(full).unwrap();
    Repo {
        full_name: full.into(),
        owner: owner.into(),
        name: name.into(),
        fork,
        archived,
        private: false,
        default_branch: Some("main".into()),
        pushed_at: None,
        listed: true,
    }
}

#[test]
fn a_repo_line_decides_first_either_way() {
    let s = parse_selection(
        r#"
[[owner]]
login = "Example-Lab"

[[owner]]
login = "example-owner"
include = "listed"

[[repo]]
name = "example-lab/old-course-site"
include = false

[[repo]]
name = "example-lab/kept-fork"
include = true

[[repo]]
name = "example-owner/notes"
include = true
"#,
    )
    .unwrap();
    assert!(
        s.selects(&repo("example-lab/tool-a", false, false)),
        "case-insensitive owner"
    );
    assert!(!s.selects(&repo("example-lab/old-course-site", false, false)));
    assert!(!s.selects(&repo("example-lab/a-fork", true, false)));
    assert!(
        s.selects(&repo("example-lab/kept-fork", true, false)),
        "a line overrides G4"
    );
    assert!(!s.selects(&repo("example-lab/dusty", false, true)));
    assert!(s.selects(&repo("example-owner/notes", false, false)));
    assert!(
        !s.selects(&repo("example-owner/other", false, false)),
        "listed: only lines"
    );
    assert!(!s.selects(&repo("example-stranger/thing", false, false)));
}

#[test]
fn a_selection_file_says_what_it_means_or_is_refused() {
    for (bad, why) in [
        (
            "[[owner]]\nlogin = \"example-lab\"\nforkz = true\n",
            "an unknown key",
        ),
        (
            "[[repo]]\nname = \"example-lab/tool-a\"\n",
            "a line without include",
        ),
        (
            "[[repo]]\nname = \"example-lab/..\"\ninclude = true\n",
            "`..` as a name",
        ),
        (
            "[[repo]]\nname = \"example-lab/a%2fb\"\ninclude = true\n",
            "an escape",
        ),
        ("[[owner]]\nlogin = \"a/b\"\n", "a path for a login"),
        (
            "[[owner]]\nlogin = \"x\"\n[[owner]]\nlogin = \"X\"\n",
            "an owner twice",
        ),
        (
            "[[owner]]\nlogin = \"x\"\ninclude = \"some\"\n",
            "an include outside the set",
        ),
    ] {
        assert!(parse_selection(bad).is_err(), "{why} must be refused");
    }
    assert!(parse_selection("").unwrap().owners.is_empty());
}

// ---- classification ----

fn ran(code: i32, stdout: &str, stderr: &str) -> Ran {
    Ran {
        code: Some(code),
        stdout: stdout.into(),
        stderr: stderr.into(),
    }
}

#[test]
fn a_failure_is_classified_by_what_gh_said() {
    assert!(matches!(
        classify(&ran(
            4,
            "",
            "To get started with GitHub CLI, please run:  gh auth login"
        )),
        GhError::Auth(_)
    ));
    assert!(matches!(
        classify(&ran(1, "", "gh: Bad credentials (HTTP 401)")),
        GhError::Auth(_)
    ));
    assert!(matches!(
        classify(&ran(
            1,
            r#"{"message":"API rate limit exceeded for user ID 1."}"#,
            "gh: API rate limit exceeded for user ID 1. (HTTP 403)"
        )),
        GhError::RateLimited(_)
    ));
    assert!(matches!(
        classify(&ran(
            1,
            r#"{"message":"You have exceeded a secondary rate limit."}"#,
            "gh: You have exceeded a secondary rate limit. (HTTP 403)"
        )),
        GhError::RateLimited(_)
    ));
    assert!(matches!(
        classify(&ran(1, "", "gh: Too Many Requests (HTTP 429)")),
        GhError::RateLimited(_)
    ));
    assert!(matches!(
        classify(&ran(1, "", "gh: Not Found (HTTP 404)")),
        GhError::NotFound(_)
    ));
    // A 403 that is not about quota is a failure, not a rate limit.
    assert!(matches!(
        classify(&ran(
            1,
            r#"{"message":"Resource not accessible by integration"}"#,
            "gh: Resource not accessible by integration (HTTP 403)"
        )),
        GhError::Failed(_)
    ));
}

/// Every call shares one budget, and it sits inside the unit's start timeout
/// with a minute left for the write: a bound per call adds up past it, and
/// systemd would kill the run before it wrote anything. The summary pass's
/// budget comes after the collection's, so both fit, with the minute.
#[test]
fn the_budgets_fit_inside_the_units_start_timeout() {
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts");
    let unit = std::fs::read_to_string(scripts.join("mecha-github-collect.service")).unwrap();
    let secs: u64 = unit
        .lines()
        .find_map(|l| l.strip_prefix("TimeoutStartSec="))
        .and_then(|v| v.trim().strip_suffix('s'))
        .and_then(|v| v.parse().ok())
        .expect("the unit states TimeoutStartSec in seconds");
    assert!(
        RUN_BUDGET + summary::BUDGET + StdDuration::from_secs(60) <= StdDuration::from_secs(secs)
    );
    assert!(
        unit.lines()
            .any(|l| l.trim() == "ExecStart=%h/.cargo/bin/mecha github collect"),
        "the unit runs the collector"
    );
    // The cadence is the timer's — once a day, the owner's ruling G10 — never
    // a constant here.
    let timer = std::fs::read_to_string(scripts.join("mecha-github-collect.timer")).unwrap();
    assert!(timer.lines().any(|l| l.starts_with("OnCalendar=")));
}

// ---- the log, read back ----

#[test]
fn the_log_says_how_long_a_problem_has_lasted() {
    let dir = scratch("streak");
    let db = db_path(&dir.join("home"));
    let at = |m: i64| now() + Duration::minutes(m);
    let with_problem = |t: DateTime<Utc>, kind: Kind| {
        let mut c = Collected::new(t);
        c.problems.push(Problem {
            kind,
            step: Step::Preflight,
            repo: None,
            detail: "x".into(),
        });
        c
    };
    record(&db, &with_problem(at(0), Kind::RateLimited)).unwrap();
    record(&db, &Collected::new(at(10))).unwrap();
    record(&db, &with_problem(at(20), Kind::Auth)).unwrap();
    record(&db, &with_problem(at(30), Kind::Auth)).unwrap();
    let mut gone = with_problem(at(40), Kind::Auth);
    gone.problems.push(Problem {
        kind: Kind::Vanished,
        step: Step::Inventory,
        repo: Some("example-lab/tool-b".into()),
        detail: "x".into(),
    });
    record(&db, &gone).unwrap();
    let report = read_log(&db, Duration::days(7), at(40)).unwrap();
    let last = report.last.unwrap();
    assert_eq!(last.run, at(40));
    assert_eq!(last.since.get("auth"), Some(&(at(20), 3)));
    assert_eq!(last.since.get("vanished"), Some(&(at(40), 1)));
    assert_eq!(
        report.vanished,
        [("example-lab/tool-b".to_string(), at(40))]
    );
    // These runs never reached the selection: unknown, never a zero.
    assert_eq!(last.selected, None);
}

#[test]
fn an_empty_log_reads_as_no_run() {
    let dir = scratch("empty-log");
    let db = db_path(&dir.join("home"));
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute_batch(SCHEMA)
        .unwrap();
    assert_eq!(
        read_log(&db, Duration::days(7), now()).unwrap(),
        LogReport::default()
    );
}

// ---- what a board may link to (§4) ----

/// A board may link to a repository the collector saw *and* the selection
/// includes now: not the fork, the archived one or another owner's, all in
/// the inventory; not a repository the inventory never held, even of a
/// selected owner; and nothing at all once the file stops selecting, or is
/// gone, or there is no store.
#[test]
fn a_board_links_only_to_inventory_the_selection_includes() {
    let dir = scratch("watched");
    let home = dir.join("home");
    assert!(Watched::read(&home).is_empty(), "no store, no links");
    select(&dir, LAB_ALL);
    assert!(
        Watched::read(&home).is_empty(),
        "a selection without an inventory links nothing"
    );
    collect_and_record(&dir, &fake(&dir, &healthy()), now());

    let w = Watched::read(&home);
    assert!(w.contains("example-lab", "tool-a"));
    assert!(
        w.contains("Example-Lab", "TOOL-A"),
        "GitHub's names are case-insensitive"
    );
    assert!(w.contains("example-lab", "tool-b"));
    for (owner, repo) in [
        ("example-lab", "fork-of-upstream"),
        ("example-lab", "old-site"),
        ("example-owner", "notes"),
        // An owner with `include = "all"` names no repository: this one is
        // of the right owner and was never seen.
        ("example-lab", "never-listed"),
        ("stranger-org", "tool-a"),
    ] {
        assert!(!w.contains(owner, repo), "{owner}/{repo}");
    }

    // The selection is read now, not as it was at the last collection.
    select(
        &dir,
        &format!("{LAB_ALL}\n[[repo]]\nname = \"example-lab/tool-b\"\ninclude = false\n"),
    );
    let w = Watched::read(&home);
    assert!(w.contains("example-lab", "tool-a"));
    assert!(
        !w.contains("example-lab", "tool-b"),
        "excluded since the last run"
    );

    select(&dir, "this is not toml [");
    assert!(
        Watched::read(&home).is_empty(),
        "an unusable file selects nothing"
    );
    std::fs::remove_file(selection_path(&home)).unwrap();
    assert!(
        Watched::read(&home).is_empty(),
        "an absent file selects nothing"
    );
}

// ---- the board shipped under scripts/hud/github ----

/// The board shipped under `scripts/hud/github/` validates as written, and
/// every one of its loaders runs — confined, as a refresh would — against a
/// store this collector wrote, and passes its own declared shape. And the
/// "new in a day / a week" windows read GitHub's RFC 3339 times as times: an
/// issue opened thirty hours ago is in the week and not in the day (compared
/// against `datetime('now', …)`'s space-separated form it was in both).
#[test]
fn the_shipped_board_validates_and_its_loaders_read_the_collector() {
    use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
    use crate::hud::Installed;

    let board = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/github");
    let installed = Installed::load_dir(&board, "github")
        .unwrap()
        .unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(installed.loaders().len(), 4);

    let dir = scratch("shipped-board");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), Utc::now());
    let path = db_path(&dir.join("home"));
    let thirty_hours_ago = (Utc::now() - Duration::hours(30))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    db(&dir)
        .execute(
            "INSERT INTO items (repo, number, kind, title, author, author_association,
               created_at, updated_at, labels, comments, first_seen, last_seen, gone_at)
             VALUES ('example-lab/tool-a', 99, 'issue', 'A new one', 'someone', 'NONE',
               ?1, ?1, '[]', 0, ?1, ?1, NULL)",
            [&thirty_hours_ago],
        )
        .unwrap();

    // One workflow that went red after a success, its latest run cancelled:
    // a cancelled run decides nothing, so it is still red, since the failure.
    // One reading of the clock: `at(40)` when a row is written and `at(40)`
    // when it is expected must be the same second, which two `now()` calls a
    // second boundary apart are not (seen once under the full suite).
    let base = Utc::now();
    let at = |hours: i64| {
        (base - Duration::hours(hours))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let conn = db(&dir);
    for (id, conclusion, hours) in [
        (9001, "success", 50),
        (9002, "failure", 40),
        (9003, "failure", 30),
        (9004, "cancelled", 20),
    ] {
        conn.execute(
            "INSERT INTO ci_runs (id, repo, workflow, workflow_id, run_number, attempt, event,
               status, conclusion, head_sha, branch, created_at, seen_at)
             VALUES (?1, 'example-lab/tool-a', 'Board test', 777, ?1, 1, 'push',
               'completed', ?2, 'abc', 'main', ?3, ?3)",
            rusqlite::params![id, conclusion, at(hours)],
        )
        .unwrap();
    }
    // And a workflow whose success and failure share a second, the failure
    // the later run: red, since that failure, in either insert order.
    let same = at(10);
    for (id, conclusion) in [(9102, "failure"), (9101, "success")] {
        conn.execute(
            "INSERT INTO ci_runs (id, repo, workflow, workflow_id, run_number, attempt, event,
               status, conclusion, head_sha, branch, created_at, seen_at)
             VALUES (?1, 'example-lab/tool-a', 'Same second', 778, ?1, 1, 'push',
               'completed', ?2, 'abc', 'main', ?3, ?3)",
            rusqlite::params![id, conclusion, same],
        )
        .unwrap();
    }
    // And, in a repository nothing else turns red, a workflow GitHub gave no
    // id: green, then red. The headline counts its repository, so the table
    // must show it — `=` on a NULL id dropped it from every join.
    for (id, conclusion, hours) in [(9201, "success", 60), (9202, "failure", 45)] {
        conn.execute(
            "INSERT INTO ci_runs (id, repo, workflow, workflow_id, run_number, attempt, event,
               status, conclusion, head_sha, branch, created_at, seen_at)
             VALUES (?1, 'example-lab/tool-b', 'No id', NULL, ?1, 1, 'push',
               'completed', ?2, 'abc', 'main', ?3, ?3)",
            rusqlite::params![id, conclusion, at(hours)],
        )
        .unwrap();
    }
    drop(conn);

    for (name, loader) in installed.loaders() {
        assert_eq!(loader.source(), "github", "{name}");
        let fetched = run_sqlite(&path, loader.query(), loader.max_rows(), QUERY_TIMEOUT)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        loader
            .shape(&fetched.columns, fetched.rows)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    // The New table's typed reference (§4, step 3) over the loader's own
    // output: the watched repository's item is linkable, and nothing else is.
    let spec = installed.spec();
    assert_eq!(
        spec.github_refs("new_items").len(),
        1,
        "the New table has the reference"
    );
    let items = &installed.loaders()["new_items"];
    let fetched = run_sqlite(&path, items.query(), items.max_rows(), QUERY_TIMEOUT).unwrap();
    let dataset = crate::hud::Dataset {
        name: "new_items".into(),
        generated_at: Utc::now(),
        generation: 1,
        loader_digest: items.digest(),
        columns: items.columns().to_vec(),
        rows: items.shape(&fetched.columns, fetched.rows).unwrap(),
        external: true,
    };
    let watched = Watched::read(&dir.join("home"));
    let linkable =
        crate::hud::link::github_linkable(spec, &dataset, &|o, r| watched.contains(o, r));
    assert!(
        linkable[0]
            .allowed
            .contains(&("example-lab".into(), "tool-a".into(), 99)),
        "{linkable:?}"
    );
    assert!(
        linkable[0]
            .allowed
            .iter()
            .all(|(o, r, _)| watched.contains(o, r)),
        "{linkable:?}"
    );

    let failing = &installed.loaders()["failing_now"];
    let fetched = run_sqlite(&path, failing.query(), 200, QUERY_TIMEOUT).unwrap();
    let col = |name: &str| fetched.columns.iter().position(|c| c == name).unwrap();
    let red: Vec<_> = fetched
        .rows
        .iter()
        .filter(|r| r[col("workflow")] == serde_json::json!("Board test"))
        .collect();
    assert_eq!(
        red.len(),
        1,
        "a latest cancelled run hid a red workflow: {:?}",
        fetched.rows
    );
    assert_eq!(
        red[0][col("red_since")],
        serde_json::json!(at(40)),
        "since the first failure"
    );
    let tied: Vec<_> = fetched
        .rows
        .iter()
        .filter(|r| r[col("workflow")] == serde_json::json!("Same second"))
        .collect();
    assert_eq!(
        tied.len(),
        1,
        "a same-second tie was decided by scan order: {:?}",
        fetched.rows
    );
    assert_eq!(
        tied[0][col("red_since")],
        serde_json::json!(same),
        "{tied:?}"
    );
    let unnamed: Vec<_> = fetched
        .rows
        .iter()
        .filter(|r| r[col("workflow")] == serde_json::json!("No id"))
        .collect();
    assert_eq!(
        unnamed.len(),
        1,
        "a red workflow with no id is missing from the table: {:?}",
        fetched.rows
    );
    assert_eq!(
        unnamed[0][col("repo")],
        serde_json::json!("example-lab/tool-b")
    );
    assert_eq!(unnamed[0][col("red_since")], serde_json::json!(at(45)));
    assert_eq!(
        unnamed[0][col("last_green")],
        serde_json::json!(at(60)),
        "its last success was found"
    );
    let red_repos: std::collections::BTreeSet<String> = fetched
        .rows
        .iter()
        .map(|r| r[col("repo")].as_str().unwrap().to_string())
        .collect();

    // The New view reads the summary of the body GitHub holds now, says why
    // where there is none, and never shows an old body's summary.
    let conn = db(&dir);
    for (n, hash) in [(99, "h99"), (1, "h1"), (7, "h7-now")] {
        conn.execute(
            "UPDATE items SET body_hash = ?2 WHERE repo = 'example-lab/tool-a' AND number = ?1",
            rusqlite::params![n, hash],
        )
        .unwrap();
    }
    for (n, hash, outcome, text, kind) in [
        (
            99,
            "h99",
            "summarised",
            Some("Asks for a flag to skip the cache."),
            Some("request"),
        ),
        (1, "h1", "refused", None, None),
        (
            7,
            "h7-before",
            "summarised",
            Some("The old body's summary."),
            Some("bug"),
        ),
    ] {
        conn.execute(
            "INSERT INTO summaries (repo, number, body_hash, outcome, summary, kind, at)
             VALUES ('example-lab/tool-a', ?1, ?2, ?3, ?4, ?5, '2026-11-20T00:00:00Z')",
            rusqlite::params![n, hash, outcome, text, kind],
        )
        .unwrap();
    }
    drop(conn);
    let new_items = &installed.loaders()["new_items"];
    let fetched = run_sqlite(&path, new_items.query(), 500, QUERY_TIMEOUT).unwrap();
    let col = |name: &str| fetched.columns.iter().position(|c| c == name).unwrap();
    let of = |n: i64| {
        fetched
            .rows
            .iter()
            .find(|r| r[col("number")] == serde_json::json!(n))
            .map(|r| (r[col("about")].clone(), r[col("summary")].clone()))
            .unwrap()
    };
    assert_eq!(
        of(99),
        (
            serde_json::json!("request"),
            serde_json::json!("Asks for a flag to skip the cache.")
        )
    );
    assert_eq!(
        of(1),
        (
            serde_json::Value::Null,
            serde_json::json!("(no summary: the model's answer was refused)")
        )
    );
    assert_eq!(
        of(7),
        (
            serde_json::Value::Null,
            serde_json::json!("(not summarised yet)")
        ),
        "an edited body showed the old body's summary"
    );
    assert_eq!(
        of(9),
        (
            serde_json::Value::Null,
            serde_json::json!("(not summarised yet)")
        )
    );

    let headline = &installed.loaders()["headline"];
    let fetched = run_sqlite(&path, headline.query(), 1, QUERY_TIMEOUT).unwrap();
    let col = |name: &str| fetched.columns.iter().position(|c| c == name).unwrap();
    let row = &fetched.rows[0];
    assert_eq!(
        row[col("failing")].as_i64().unwrap(),
        red_repos.len() as i64,
        "the headline and the table disagree about which repositories are red: \
         {row:?} vs {red_repos:?}"
    );
    assert_eq!(row[col("new_day")], serde_json::json!(0), "{row:?}");
    assert_eq!(row[col("new_week")], serde_json::json!(1), "{row:?}");
    assert!(
        !row[col("collected_at")].is_null(),
        "the collector's finish is on the board"
    );

    // A later run that collected nothing (a revoked login) writes its own
    // `finished` row; the board's freshness stays at the last run that read.
    let fresh = row[col("collected_at")].clone();
    let refused = fake(
        &dir,
        &with(vec![fails(
            "*'api rate_limit'*",
            "",
            "To get started with GitHub CLI, please run:  gh auth login\n",
            4,
        )]),
    );
    collect_and_record(&dir, &refused, Utc::now() + Duration::minutes(5));
    let fetched = run_sqlite(&path, headline.query(), 1, QUERY_TIMEOUT).unwrap();
    assert_eq!(
        fetched.rows[0][col("collected_at")],
        fresh,
        "a run that read nothing made the board look fresh"
    );
}

/// GitHub's times are RFC 3339 (`2026-11-18T12:00:00Z`) and SQLite's
/// `datetime('now', …)` writes `2026-11-18 12:00:00`: compared as strings the
/// `T` sorts after the space, so every window built that way is a day wide
/// at its edge. Both of #902's window findings were this one idiom.
#[test]
fn no_shipped_loader_compares_github_times_against_datetime() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/github/loaders");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("datetime('now'"),
            "{}: compare against strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ...)",
            path.display()
        );
    }
}
