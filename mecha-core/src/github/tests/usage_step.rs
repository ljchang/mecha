//! Step 5's usage against the fake `gh` and a fake registry: traffic that
//! accumulates past GitHub's window, a dash for a login that cannot push,
//! release downloads per day, the package mapping, and the registries'
//! refusals and failures kept apart. Never the network.

use super::*;
use crate::github::registry::{self, Answer, Fetch};
use crate::github::usage::{
    canonical_name, conda_recipe_name, package_json_name, packages_path, parse_packages,
    parse_releases, pyproject_name, read_packages, setup_cfg_name, PackagesRead, Registry, Traffic,
    TrafficRead, TRAFFIC_WINDOW_DAYS,
};
use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
use crate::hud::Installed;
use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use std::sync::Mutex;

fn on(day: &str) -> DateTime<Utc> {
    format!("{day}T12:00:00Z").parse().unwrap()
}

fn traffic(repo: &str, clones: &[(&str, i64, i64)], views: &[(&str, i64, i64)]) -> Vec<Case> {
    vec![
        ok(
            &format!("*'repos/{repo}/traffic/clones'*"),
            &traffic_json("clones", clones),
        ),
        ok(
            &format!("*'repos/{repo}/traffic/views'*"),
            &traffic_json("views", views),
        ),
    ]
}

/// The shipped board's loader `name`, run confined against the store as a
/// refresh would, its declared shape checked, each row as a map by column.
fn board_rows(dir: &Path, name: &str) -> Vec<BTreeMap<String, Json>> {
    let board = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/github");
    let installed = Installed::load_dir(&board, "github")
        .unwrap()
        .unwrap_or_else(|r| panic!("{r}"));
    let loader = &installed.loaders()[name];
    let fetched = run_sqlite(
        &db_path(&dir.join("home")),
        loader.query(),
        loader.max_rows(),
        QUERY_TIMEOUT,
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    let rows = loader
        .shape(&fetched.columns, fetched.rows)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    rows.into_iter()
        .map(|r| fetched.columns.iter().cloned().zip(r).collect())
        .collect()
}

fn row_of<'a>(rows: &'a [BTreeMap<String, Json>], repo: &str) -> &'a BTreeMap<String, Json> {
    rows.iter()
        .find(|r| r["repo"] == json!(repo))
        .unwrap_or_else(|| panic!("no row for {repo}: {rows:?}"))
}

// ---- traffic ----

/// GitHub keeps fourteen days; the store keeps every day it was shown. Three
/// runs over six weeks: the first window's days survive the later reads, a
/// day that grew takes the newer number, a day an earlier answer counted is
/// never zeroed by a later answer that did not list it, and the eleven days
/// no read covered are counted missing — not zero — beside a total that
/// says since when it runs.
#[test]
fn traffic_accumulates_across_runs_past_the_window() {
    let dir = scratch("traffic-accumulates");
    select(&dir, LAB_ALL);
    let tool_a = "example-lab/tool-a";
    let run = |day: &str, clones: &[(&str, i64, i64)]| {
        let cases = with(traffic(tool_a, clones, &[(clones[0].0, 9, 2)]));
        collect_and_record(&dir, &fake(&dir, &cases), on(day));
    };
    run(
        "2026-11-01",
        &[
            ("2026-10-25", 5, 3),
            ("2026-10-30", 2, 1),
            ("2026-11-01", 1, 1),
        ],
    );
    // 11-01 was in progress at the first read and is final now; 10-30 is in
    // this window and not listed.
    run("2026-11-05", &[("2026-11-01", 4, 2), ("2026-11-04", 7, 4)]);
    // Twenty-five days later: 11-06..11-16 were in no window read.
    run("2026-11-30", &[("2026-11-20", 3, 3)]);

    let conn = db(&dir);
    let day = |d: &str| -> Option<i64> {
        conn.query_row(
            "SELECT clones FROM traffic WHERE repo = ?1 AND day = ?2",
            params![tool_a, d],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
    };
    assert_eq!(day("2026-10-19"), Some(0), "the first window's first day");
    assert_eq!(day("2026-10-25"), Some(5), "kept from a window long gone");
    assert_eq!(day("2026-10-30"), Some(2), "zeroed by a later answer");
    assert_eq!(
        day("2026-11-01"),
        Some(4),
        "a day that grew keeps the newer"
    );
    assert_eq!(day("2026-11-10"), None, "a day no read covered is no row");
    assert_eq!(day("2026-11-20"), Some(3));
    let span: i64 = conn
        .query_row(
            "SELECT CAST(julianday(max(day)) - julianday(min(day)) AS INTEGER) + 1 FROM traffic WHERE repo = ?1",
            [tool_a],
            |r| r.get(0),
        )
        .unwrap();
    assert!(span > TRAFFIC_WINDOW_DAYS * 3, "{span} days kept");

    // The board: the total since the first day read, the gap said.
    let rows = board_rows(&dir, "usage");
    let a = row_of(&rows, tool_a);
    assert_eq!(a["clones"], json!(5 + 2 + 4 + 7 + 3));
    assert_eq!(a["clones_since"], json!("2026-10-19"), "{a:?}");
    assert_eq!(a["missing_days"], json!(11), "{a:?}");
    assert_eq!(a["unique_cloner_days"], json!(3 + 1 + 2 + 4 + 3));
    assert_eq!(a["cloners_14d"], json!(3), "GitHub's own window figure");
    assert_eq!(a["traffic"], json!("read 2026-11-30"));
    assert_eq!(
        a["clones_trend"].as_str().unwrap().chars().count(),
        30,
        "a day a character"
    );
    // tool-b answered every time, empty: zeros, a real reading, with its date.
    let b = row_of(&rows, "example-lab/tool-b");
    assert_eq!(b["clones"], json!(0));
    assert_eq!(b["clones_since"], json!("2026-10-19"));
}

fn permitted(full: &str, push: bool) -> String {
    repo_json(full, false, false).replacen(
        '{',
        &format!(r#"{{"permissions":{{"admin":false,"push":{push},"pull":true}},"#),
        1,
    )
}

/// A repository the login cannot push to is not asked when its permissions
/// say so, and one GitHub refuses anyway is a refusal — `traffic_no_access`,
/// not `failed` — that neither marks it unread nor fails the run. Both show
/// a dash on the board, never a zero; and a traffic call that fails is a
/// failure, said as one.
#[test]
fn no_push_access_is_a_refusal_and_a_dash_never_zero() {
    let dir = scratch("traffic-no-access");
    select(&dir, LAB_ALL);
    let listing = format!(
        "[{},{}]\n",
        permitted("example-lab/tool-a", true),
        permitted("example-lab/tool-b", false)
    );
    let gh = fake(
        &dir,
        &with(vec![
            ok("*'user/repos'*", &listing),
            fails(
                "*'repos/example-lab/tool-a/traffic/clones'*",
                r#"{"message":"Must have push access to repository","status":"403"}"#,
                "gh: Must have push access to repository (HTTP 403)\n",
                1,
            ),
        ]),
    );
    let c = collect_and_record(&dir, &gh, now());
    assert!(
        calls(&dir).iter().all(|l| !l.contains("tool-b/traffic")),
        "the permissions said no push, and it was asked: {:?}",
        calls(&dir)
    );
    let conn = db(&dir);
    assert_eq!(
        kinds(&conn),
        ["traffic_no_access:traffic:example-lab/tool-a"]
    );
    assert_eq!(
        c.collected(),
        2,
        "a refusal of traffic is not a repository unread"
    );
    assert!(c.problems.iter().all(|p| !p.kind.fails_collection()));
    assert_eq!(
        strings(
            &conn,
            "SELECT repo || ':' || state FROM traffic_access ORDER BY repo"
        ),
        [
            "example-lab/tool-a:no_access",
            "example-lab/tool-b:no_access"
        ]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT repo || ':' || push FROM inventory ORDER BY repo"
        ),
        ["example-lab/tool-a:1", "example-lab/tool-b:0"]
    );
    let rows = board_rows(&dir, "usage");
    for repo in ["example-lab/tool-a", "example-lab/tool-b"] {
        let r = row_of(&rows, repo);
        assert_eq!(r["clones"], Json::Null, "{repo}: a dash, never 0");
        assert_eq!(r["clones_since"], Json::Null);
        assert_eq!(r["views"], Json::Null);
        assert_eq!(r["traffic"], json!("no push access"));
    }
    let findings = crate::doctor::check_github(&db_path(&dir.join("home")), now());
    let said: Vec<&str> = findings.iter().map(|f| f.summary.as_str()).collect();
    assert_eq!(
        said,
        ["GitHub refused traffic for 1 watched repository: the login cannot push to it"]
    );

    // A traffic call that fails is a failure: its own kind, its own finding.
    let dir = scratch("traffic-failed");
    select(&dir, LAB_ALL);
    let gh = fake(
        &dir,
        &with(vec![fails(
            "*'repos/example-lab/tool-a/traffic/clones'*",
            "",
            "gh: Server Error (HTTP 502)\n",
            1,
        )]),
    );
    let c = collect_and_record(&dir, &gh, now());
    assert_eq!(kinds(&db(&dir)), ["failed:traffic:example-lab/tool-a"]);
    assert_eq!(c.collected(), 1);
    let rows = board_rows(&dir, "usage");
    assert_eq!(row_of(&rows, "example-lab/tool-a")["clones"], Json::Null);
    let findings = crate::doctor::check_github(&db_path(&dir.join("home")), now());
    assert_eq!(
        findings
            .iter()
            .map(|f| f.summary.as_str())
            .collect::<Vec<_>>(),
        ["1 GitHub collection step(s) failed"]
    );
}

/// A day listed without an integer `count` or `uniques` makes the whole
/// answer unreadable, as a day without a date does: kept as unknown it would
/// be zero-filled, and unknown is never zero.
#[test]
fn a_traffic_day_without_its_numbers_is_unreadable_not_zero() {
    let dir = scratch("traffic-no-count");
    select(&dir, LAB_ALL);
    let bad = r#"{"count":5,"uniques":2,"clones":[{"timestamp":"2026-11-17T00:00:00Z","uniques":2},{"timestamp":"2026-11-18T00:00:00Z","count":5,"uniques":2}]}"#;
    let gh = fake(
        &dir,
        &with(vec![ok("*'repos/example-lab/tool-a/traffic/clones'*", bad)]),
    );
    collect_and_record(&dir, &gh, now());
    let conn = db(&dir);
    assert_eq!(kinds(&conn), ["failed:traffic:example-lab/tool-a"]);
    assert_eq!(
        strings(
            &conn,
            "SELECT day FROM traffic WHERE repo = 'example-lab/tool-a'"
        ),
        Vec::<String>::new(),
        "a day with no count was written as a zero"
    );
    let uniques_text = r#"{"count":1,"uniques":1,"views":[{"timestamp":"2026-11-18T00:00:00Z","count":1,"uniques":"one"}]}"#;
    assert_eq!(
        crate::github::usage::parse_series(uniques_text, "views"),
        None
    );
}

/// The traffic step reads at most its share a run, the never-read first and
/// then the longest unread: over runs, every repository is read.
#[test]
fn traffic_is_read_in_turn_the_longest_unread_first() {
    let dir = scratch("traffic-order");
    let db = db_path(&dir.join("home"));
    let mut c = Collected::new(now());
    c.traffic.push(Traffic {
        repo: "example-lab/tool-b".into(),
        read: TrafficRead::NoAccess { detail: "x".into() },
    });
    record(&db, &c).unwrap();
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now() + Duration::days(1));
    let order: Vec<String> = calls(&dir)
        .into_iter()
        .filter(|l| l.contains("/traffic/clones"))
        .collect();
    assert_eq!(order.len(), 2);
    assert!(
        order[0].contains("tool-a"),
        "the never-read one goes first: {order:?}"
    );
}

// ---- releases ----

fn with_releases(data: String, releases: &str) -> String {
    let mut v: Json = serde_json::from_str(&data).unwrap();
    v["releases"] = serde_json::from_str(releases).unwrap();
    v.to_string()
}

fn releases_json(total: i64, assets: &[(i64, &str, i64)]) -> String {
    let nodes: Vec<String> = assets
        .iter()
        .map(|(id, name, n)| format!(r#"{{"id":"RA_{id}","name":"{name}","downloadCount":{n}}}"#))
        .collect();
    format!(
        r#"{{"totalCount":{total},"nodes":[{{"databaseId":7,"tagName":"v1.0","releaseAssets":{{"totalCount":{},"nodes":[{}]}}}}]}}"#,
        assets.len(),
        nodes.join(",")
    )
}

/// Release downloads are sampled once a day: a row per asset per day, the
/// day's latest answer standing for it, and the board reads the latest day.
/// A repository with releases and no asset shows a dash and says why; one
/// with more releases than were asked says the count is of the newest.
#[test]
fn release_downloads_are_sampled_per_day() {
    let dir = scratch("releases");
    select(&dir, LAB_ALL);
    let run = |at: DateTime<Utc>, a: i64, b: i64, total: i64| {
        let answer = batch(&[
            with_releases(
                tool_a(),
                &releases_json(total, &[(111, "tool-a.tar.gz", a), (112, "tool-a.whl", b)]),
            ),
            with_releases(repo_data(1, &[], &[]), &releases_json(1, &[])),
        ]);
        let cases = with(vec![ok("*'api graphql'*'fragment R'*", &answer)]);
        collect_and_record(&dir, &fake(&dir, &cases), at);
    };
    run(on("2026-03-10"), 40, 2, 1);
    run(on("2026-03-11"), 45, 3, 1);
    // Later the same day: the day's row is the newer answer, not a second.
    run(on("2026-03-11") + Duration::hours(3), 46, 3, 1);

    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT day || ':' || sum(downloads) FROM releases WHERE repo = 'example-lab/tool-a' GROUP BY day ORDER BY day"
        ),
        ["2026-03-10:42", "2026-03-11:49"]
    );
    let rows = board_rows(&dir, "usage");
    let a = row_of(&rows, "example-lab/tool-a");
    assert_eq!(a["release_downloads"], json!(49));
    assert_eq!(a["release_note"], json!("every release"));
    let b = row_of(&rows, "example-lab/tool-b");
    assert_eq!(
        b["release_downloads"],
        Json::Null,
        "no asset is a dash, not 0"
    );
    assert_eq!(b["release_note"], json!("no release assets"));
    assert_eq!(b["stars"], json!(1), "stars ride the counts table");

    // More releases than were asked: the count is the newest releases'.
    run(on("2026-03-12"), 50, 3, 30);
    let rows = board_rows(&dir, "usage");
    assert_eq!(
        row_of(&rows, "example-lab/tool-a")["release_note"],
        json!("newest releases only")
    );
}

#[test]
fn a_releases_field_that_is_not_there_is_unknown_not_none() {
    assert_eq!(parse_releases(&json!({"stargazerCount": 1})), None);
    let r = parse_releases(&json!({"releases": {"totalCount": 0, "nodes": []}})).unwrap();
    assert!(r.assets.is_empty() && r.complete);
    // An asset list cut short is not the whole count.
    let r = parse_releases(&json!({"releases": {"totalCount": 1, "nodes": [
        {"databaseId": 1, "tagName": "v1", "releaseAssets": {"totalCount": 60, "nodes": [
            {"id": "RA_2", "name": "a", "downloadCount": 5}]}}]}}))
    .unwrap();
    assert!(!r.complete);
    assert_eq!(r.assets[0].downloads, Some(5));
}

// ---- package names ----

#[test]
fn a_package_name_is_read_from_each_supported_manifest() {
    assert_eq!(
        pyproject_name("[project]\nname = \"Tool_A.core\"\nversion = \"1\"\n").as_deref(),
        Some("tool-a-core"),
        "PEP 503's form"
    );
    assert_eq!(
        pyproject_name("[tool.poetry]\nname = \"tool-a\"\n").as_deref(),
        Some("tool-a")
    );
    assert_eq!(
        pyproject_name("[build-system]\nrequires = [\"setuptools\"]\n"),
        None,
        "a pyproject without a name names nothing"
    );
    assert_eq!(
        setup_cfg_name(
            "[options]\nname = wrong\n[metadata]\n# a comment\nname = tool_a\nversion = 1\n"
        )
        .as_deref(),
        Some("tool-a")
    );
    assert_eq!(
        package_json_name(r#"{"name": "@example-lab/tool-a", "version": "1.0.0"}"#).as_deref(),
        Some("@example-lab/tool-a")
    );
    assert_eq!(
        package_json_name(r#"{"name": "tool-a-site", "private": true}"#),
        None,
        "npm will not publish a private package"
    );
    assert_eq!(
        conda_recipe_name(
            "{% set name = \"Tool-A\" %}\n{% set version = \"1.0\" %}\n\npackage:\n  name: {{ name|lower }}\n  version: {{ version }}\n"
        )
        .as_deref(),
        Some("tool-a")
    );
    assert_eq!(
        conda_recipe_name("package:\n  name: tool-a\nsource:\n  url: x\n").as_deref(),
        Some("tool-a")
    );
    assert_eq!(
        conda_recipe_name("package:\n  name: {{ name ~ '-core' }}\n"),
        None,
        "Jinja it cannot read is not guessed"
    );
    // A name outside the registry's grammar is never a name: nothing can
    // reach an address through one.
    for bad in ["../x", "a b", "x?y=1", "-lead", "trail-", "", "a/b"] {
        assert_eq!(canonical_name(Registry::Pypi, bad), None, "{bad:?}");
    }
    for bad in ["Tool", "../x", "@scope", "@scope/", "_x", "a b", "a/b/c"] {
        assert_eq!(canonical_name(Registry::Npm, bad), None, "{bad:?}");
    }
    assert_eq!(
        registry::url(Registry::Pypi, "Tool_A").as_deref(),
        Some("https://pypistats.org/api/packages/tool-a/overall?mirrors=false")
    );
    assert_eq!(registry::url(Registry::Npm, "../../x"), None);
    assert_eq!(
        registry::url(Registry::Conda, "tool-a"),
        None,
        "not counted"
    );
}

const PYPROJECT: &str = "[project]\nname = \"Tool_A\"\n";
const PACKAGE_JSON: &str = r#"{"name": "tool-a-js"}"#;

fn scanned(tool_a_files: &[(&str, &str)]) -> Vec<Case> {
    with(vec![ok(
        "*'fragment M on Repository'*",
        &batch(&[manifests(tool_a_files), manifests(&[])]),
    )])
}

/// Each repository's manifests name its packages; a repository with none
/// says it has none (never left looking unread); the owner's line wins over
/// what was detected, either way, and the detected name stays beside it for
/// the picker; a file that cannot be used leaves the mapping as it was.
#[test]
fn the_package_mapping_is_detected_and_the_owner_corrects_it() {
    let dir = scratch("packages");
    select(&dir, LAB_ALL);
    let home = dir.join("home");
    collect_and_record(
        &dir,
        &fake(
            &dir,
            &scanned(&[("pyproject", PYPROJECT), ("packagejson", PACKAGE_JSON)]),
        ),
        now(),
    );
    let conn = db(&dir);
    assert_eq!(
        strings(
            &conn,
            "SELECT repo || ':' || found FROM package_scans ORDER BY repo"
        ),
        ["example-lab/tool-a:2", "example-lab/tool-b:0"]
    );
    let mapping = |conn: &Connection| {
        strings(
            conn,
            "SELECT repo || ' ' || registry || ' ' || coalesce(name, '-') || ' ' || source || ' ' || coalesce(detected_name, '-') FROM packages ORDER BY repo, registry",
        )
    };
    assert_eq!(
        mapping(&conn),
        [
            "example-lab/tool-a npm tool-a-js detected tool-a-js",
            "example-lab/tool-a pypi tool-a detected tool-a"
        ]
    );
    let rows = board_rows(&dir, "usage");
    assert_eq!(
        row_of(&rows, "example-lab/tool-a")["packages"],
        json!("npm:tool-a-js pypi:tool-a")
    );
    assert_eq!(
        row_of(&rows, "example-lab/tool-b")["packages"],
        json!("none found"),
        "a repository with no manifest says so"
    );

    // The owner's corrections: another name on PyPI, nothing on npm.
    std::fs::write(
        packages_path(&home),
        "[[package]]\nrepo = \"Example-Lab/Tool-A\"\nregistry = \"pypi\"\nname = \"tool_a_py\"\n\n\
         [[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"npm\"\nnone = true\n",
    )
    .unwrap();
    collect_and_record(&dir, &fake(&dir, &healthy()), now() + Duration::hours(1));
    assert_eq!(
        mapping(&db(&dir)),
        [
            "example-lab/tool-a npm - owner tool-a-js",
            "example-lab/tool-a pypi tool-a-py owner tool-a"
        ],
        "the owner's line wins, and what was detected stays beside it"
    );
    let rows = board_rows(&dir, "packages");
    let npm = rows.iter().find(|r| r["registry"] == json!("npm")).unwrap();
    assert_eq!(npm["status"], json!("not published here (packages.toml)"));
    assert_eq!(npm["source"], json!("set in packages.toml"));

    // A file that cannot be used: said, and the mapping is kept.
    std::fs::write(packages_path(&home), "[[package]]\nrepo = \"x\"\n").unwrap();
    collect_and_record(&dir, &fake(&dir, &healthy()), now() + Duration::hours(2));
    let conn = db(&dir);
    assert_eq!(
        mapping(&conn),
        [
            "example-lab/tool-a npm - owner tool-a-js",
            "example-lab/tool-a pypi tool-a-py owner tool-a"
        ],
        "an unusable file rebuilt the mapping without the owner's lines"
    );
    assert_eq!(kinds(&conn), ["bad_packages:packages:"]);
    assert_eq!(
        crate::doctor::check_github(&db_path(&home), now() + Duration::hours(2))
            .iter()
            .map(|f| f.summary.as_str())
            .collect::<Vec<_>>(),
        ["the GitHub board's packages.toml cannot be used, so the package mapping is kept as it was"]
    );
}

/// A package line naming a repository the collector does not see is said —
/// its own kind, quoting the line, its own doctor finding — and is not
/// written as a row no reader reaches. It fails nothing: a line left behind
/// by a repository since deselected or renamed must not fail the unit.
#[test]
fn a_package_line_for_an_unknown_repository_is_said_not_dropped() {
    let dir = scratch("packages-unmatched");
    select(&dir, LAB_ALL);
    let home = dir.join("home");
    std::fs::create_dir_all(super::super::dir(&home)).unwrap();
    std::fs::write(
        packages_path(&home),
        "[[package]]\nrepo = \"example-lab/toola\"\nregistry = \"pypi\"\nname = \"tool-a-py\"\n\n\
         [[package]]\nrepo = \"Example-Lab/Tool-B\"\nregistry = \"npm\"\nname = \"tool-b-js\"\n",
    )
    .unwrap();
    let c = collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let conn = db(&dir);
    assert_eq!(
        kinds(&conn),
        ["package_unmatched:packages:example-lab/toola"],
        "the known repository's line, in another case, is not unmatched"
    );
    assert!(c.problems[0].detail.contains("\"example-lab/toola\""));
    assert!(c.problems.iter().all(|p| !p.kind.fails_collection()));
    assert_eq!(c.collected(), 2);
    assert_eq!(
        strings(
            &conn,
            "SELECT repo || ' ' || registry || ' ' || name FROM packages ORDER BY repo"
        ),
        ["example-lab/tool-b npm tool-b-js"],
        "an orphan row was written for the unknown line"
    );
    assert_eq!(
        crate::doctor::check_github(&db_path(&home), now())
            .iter()
            .map(|f| f.summary.as_str())
            .collect::<Vec<_>>(),
        [
            "a package line names example-lab/toola, which the collector does not see — the \
          correction does nothing"
        ]
    );
}

#[test]
fn the_owners_package_file_says_what_it_means_or_is_refused() {
    for (bad, why) in [
        (
            "[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"pypi\"\n",
            "needs a name",
        ),
        (
            "[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"pypi\"\nname = \"x\"\nnone = true\n",
            "together",
        ),
        (
            "[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"npm\"\nname = \"../x\"\n",
            "not a npm package name",
        ),
        (
            "[[package]]\nrepo = \"tool-a\"\nregistry = \"pypi\"\nname = \"x\"\n",
            "owner/name",
        ),
        (
            "[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"cargo\"\nname = \"x\"\n",
            "unknown variant",
        ),
        (
            "[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"pypi\"\nname = \"x\"\n\
             [[package]]\nrepo = \"example-lab/TOOL-A\"\nregistry = \"pypi\"\nnone = true\n",
            "twice",
        ),
        ("[[package]]\nrepo = \"example-lab/tool-a\"\nregistry = \"pypi\"\nname = \"x\"\nurl = \"y\"\n", "unknown field"),
    ] {
        let e = parse_packages(bad).unwrap_err();
        assert!(e.contains(why), "{bad:?}: {e}");
    }
    assert!(parse_packages("").unwrap().is_empty());
    assert!(matches!(
        read_packages(Path::new("/nonexistent/packages.toml")),
        PackagesRead::Absent
    ));
}

/// Manifests are read again only when the default branch has been pushed
/// since the last read (or a month has passed), never every run.
#[test]
fn manifests_are_read_again_only_after_a_push() {
    let dir = scratch("rescan");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let _ = std::fs::remove_file(dir.join("calls.log"));
    collect_and_record(&dir, &fake(&dir, &healthy()), now() + Duration::hours(1));
    assert!(
        calls(&dir).iter().all(|l| !l.contains("fragment M")),
        "an unchanged repository was read again"
    );
    let pushed = listing().replacen(
        r#""full_name":"example-lab/tool-a","fork":false,"archived":false,"private":false,"default_branch":"main","pushed_at":"2026-11-01T00:00:00Z""#,
        r#""full_name":"example-lab/tool-a","fork":false,"archived":false,"private":false,"default_branch":"main","pushed_at":"2026-11-18T09:00:00Z""#,
        1,
    );
    assert_ne!(pushed, listing());
    let _ = std::fs::remove_file(dir.join("calls.log"));
    let cases = with(vec![
        ok("*'user/repos'*", &pushed),
        ok(
            "*'fragment M on Repository'*",
            &batch(&[manifests(&[("pyproject", PYPROJECT)])]),
        ),
    ]);
    collect_and_record(&dir, &fake(&dir, &cases), now() + Duration::hours(2));
    let scans: Vec<String> = calls(&dir)
        .into_iter()
        .filter(|l| l.contains("fragment M"))
        .collect();
    assert_eq!(scans.len(), 1);
    assert!(scans[0].contains("n0=tool-a") && !scans[0].contains("tool-b"));
    assert_eq!(
        strings(&db(&dir), "SELECT name FROM package_detected"),
        ["tool-a"]
    );
}

/// A store from before step 5 has no `push` column; the first run after the
/// upgrade reads its inventory anyway, so the fallback selection and the
/// vanished check do not silently go empty for a run.
#[test]
fn an_inventory_from_before_push_is_still_read() {
    let dir = scratch("old-inventory");
    let db = db_path(&dir.join("home"));
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE inventory (
               repo TEXT PRIMARY KEY, owner TEXT NOT NULL, name TEXT NOT NULL,
               fork INTEGER NOT NULL, archived INTEGER NOT NULL, private INTEGER NOT NULL,
               default_branch TEXT, pushed_at TEXT,
               listed INTEGER NOT NULL, selected INTEGER NOT NULL, seen_at TEXT NOT NULL);
             INSERT INTO inventory VALUES ('example-lab/tool-a', 'example-lab', 'tool-a',
               0, 0, 0, 'main', NULL, 1, 1, '2026-11-01T00:00:00.000Z');",
        )
        .unwrap();
    let prior = prior_inventory(&db);
    assert_eq!(prior.len(), 1);
    assert_eq!(prior[0].0.push, None);
}

// ---- the registries ----

/// A registry that answers by address, and remembers what it was asked.
struct FakeRegistry {
    answers: Vec<(&'static str, u16, String)>,
    asked: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Fetch for FakeRegistry {
    async fn get(&self, url: &str) -> Result<(u16, String), String> {
        self.asked.lock().unwrap().push(url.to_string());
        match self.answers.iter().find(|(u, _, _)| url.contains(u)) {
            Some((_, status, body)) => Ok((*status, body.clone())),
            None => Err("no answer".into()),
        }
    }
}

fn pypi_days(days: &[(&str, i64)]) -> String {
    let data: Vec<Json> = days
        .iter()
        .map(|(d, n)| json!({"category": "without_mirrors", "date": d, "downloads": n}))
        .collect();
    json!({"data": data, "package": "x", "type": "overall_downloads"}).to_string()
}

fn npm_days(days: &[(&str, i64)]) -> String {
    let data: Vec<Json> = days
        .iter()
        .map(|(d, n)| json!({"day": d, "downloads": n}))
        .collect();
    json!({"downloads": data, "start": "x", "end": "y", "package": "x"}).to_string()
}

/// A store holding a mapping: two public repositories and a private one.
fn mapped_store(dir: &Path) -> PathBuf {
    let db = db_path(&dir.join("home"));
    let conn = open_store(&db).unwrap();
    for (repo, private) in [
        ("example-lab/tool-a", 0),
        ("example-lab/tool-b", 0),
        ("example-lab/secret", 1),
    ] {
        let name = repo.split_once('/').unwrap().1;
        conn.execute(
            "INSERT INTO inventory (repo, owner, name, fork, archived, private, listed, selected, seen_at)
             VALUES (?1, 'example-lab', ?2, 0, 0, ?3, 1, 1, '2026-11-18T00:00:00.000Z')",
            params![repo, name, private],
        )
        .unwrap();
    }
    for (repo, registry, name, source) in [
        ("example-lab/tool-a", "pypi", "tool-a", "detected"),
        ("example-lab/tool-a", "npm", "tool-a-js", "detected"),
        ("example-lab/tool-b", "pypi", "tool-b", "detected"),
        ("example-lab/tool-b", "npm", "tool-c-js", "detected"),
        ("example-lab/tool-b", "conda", "tool-b", "detected"),
        ("example-lab/secret", "pypi", "secret-pkg", "detected"),
        ("example-lab/secret", "npm", "secret-js", "owner"),
    ] {
        conn.execute(
            "INSERT INTO packages (repo, registry, name, source, manifest, detected_name, at)
             VALUES (?1, ?2, ?3, ?4, 'x', ?3, '2026-11-18T00:00:00.000Z')",
            params![repo, registry, name, source],
        )
        .unwrap();
    }
    db
}

/// A rate limit is a refusal: its own kind, once, and that registry is not
/// asked again this run. A server error is a failure, its own kind. A
/// registry that has no such package has answered, and nothing is logged. A
/// name detected in a private repository is never sent anywhere; one the
/// owner set is. Conda is mapped and not counted.
#[test]
fn a_registry_refusal_and_a_registry_failure_are_logged_apart() {
    let dir = scratch("registries");
    let db = mapped_store(&dir);
    let todo = registry::due(&db).unwrap();
    assert_eq!(
        todo.iter()
            .map(|(r, n)| format!("{}:{n}", r.as_str()))
            .collect::<Vec<_>>(),
        [
            "npm:secret-js",
            "npm:tool-a-js",
            "npm:tool-c-js",
            "pypi:tool-a",
            "pypi:tool-b"
        ],
        "a private repository's detected name was due, or conda was"
    );
    let fake = FakeRegistry {
        answers: vec![
            (
                "/last-month/tool-a-js",
                200,
                npm_days(&[("2026-11-16", 4), ("2026-11-17", 6)]),
            ),
            ("/last-month/tool-c-js", 503, String::new()),
            (
                "/last-month/secret-js",
                404,
                r#"{"error":"not found"}"#.into(),
            ),
            ("/packages/tool-a/", 429, String::new()),
            ("/packages/tool-b/", 200, pypi_days(&[("2026-11-17", 9)])),
        ],
        asked: Mutex::new(Vec::new()),
    };
    let pass = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(registry::run(&fake, &todo, registry::BUDGET));
    let asked = fake.asked.lock().unwrap().clone();
    assert!(
        asked.iter().all(|u| !u.contains("/packages/tool-b/")),
        "a registry that refused was asked again: {asked:?}"
    );
    assert!(asked.iter().all(|u| !u.contains("secret-pkg")));
    assert!(
        asked.iter().any(|u| u.contains("secret-js")),
        "the owner's name"
    );
    assert_eq!(pass.deferred, 1);
    registry::record_pass(&db, now(), &pass).unwrap();

    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        kinds(&conn),
        ["registry_failed:registry:", "registry_refused:registry:"]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT registry || ':' || name || ':' || status FROM registry_checks ORDER BY registry, name"
        ),
        [
            "npm:secret-js:not_found",
            "npm:tool-a-js:counted",
            "npm:tool-c-js:failed",
            "pypi:tool-a:refused"
        ]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT name || ':' || day || ':' || downloads FROM registry_daily ORDER BY day"
        ),
        ["tool-a-js:2026-11-16:4", "tool-a-js:2026-11-17:6"]
    );
    // The next run asks the longest unchecked first: pypi's tool-b, never
    // asked, ahead of every name that was.
    let todo = registry::due(&db).unwrap();
    assert_eq!(todo[0], (Registry::Pypi, "tool-b".to_string()));

    // The doctor: two findings, never one merged.
    let found: Vec<String> = crate::doctor::check_github(&db, now())
        .into_iter()
        .map(|f| f.summary)
        .collect();
    assert_eq!(
        found,
        [
            "a package registry refused the GitHub collector",
            "1 package registry count failed"
        ]
    );
}

#[test]
fn a_registry_answer_is_classified_by_status_then_shape() {
    assert_eq!(
        registry::classify(Registry::Npm, 200, &npm_days(&[("2026-11-17", 3)])),
        Answer::Counted(vec![(NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(), 3)])
    );
    assert_eq!(
        registry::classify(
            Registry::Pypi,
            200,
            &json!({"data": [
                {"category": "with_mirrors", "date": "2026-11-17", "downloads": 50},
                {"category": "without_mirrors", "date": "2026-11-17", "downloads": 20}]})
            .to_string()
        ),
        Answer::Counted(vec![(NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(), 20)]),
        "mirrors are not installs"
    );
    assert!(matches!(
        registry::classify(Registry::Npm, 200, "<html>"),
        Answer::Failed(_)
    ));
    assert_eq!(registry::classify(Registry::Npm, 404, ""), Answer::NotFound);
    assert!(matches!(
        registry::classify(Registry::Pypi, 429, ""),
        Answer::Refused(_)
    ));
    assert!(matches!(
        registry::classify(Registry::Pypi, 403, ""),
        Answer::Refused(_)
    ));
    assert!(matches!(
        registry::classify(Registry::Pypi, 502, ""),
        Answer::Failed(_)
    ));
    assert!(matches!(
        registry::classify(Registry::Pypi, 302, ""),
        Answer::Failed(_)
    ));
}

#[test]
fn only_the_collections_own_problems_fail_it() {
    for k in Kind::ALL {
        let aside = matches!(
            k,
            Kind::SummarySkipped
                | Kind::SummaryFailed
                | Kind::TrafficNoAccess
                | Kind::RegistryRefused
                | Kind::RegistryFailed
                | Kind::PackageUnmatched
        );
        assert_eq!(k.fails_collection(), !aside, "{k:?}");
        assert_eq!(Kind::parse(k.as_str()), Some(k));
    }
}

/// Running out of the pass's minute is not a registry failure: nothing was
/// asked, nothing refused — it is counted deferred and logs no problem
/// (review of #912).
#[tokio::test]
async fn a_spent_registry_budget_is_deferred_not_failed() {
    let fetch = FakeRegistry {
        answers: vec![],
        asked: Mutex::new(Vec::new()),
    };
    let todo = vec![
        (Registry::Pypi, "tool-a".to_string()),
        (Registry::Npm, "tool-b".to_string()),
    ];
    let pass = registry::run(&fetch, &todo, std::time::Duration::ZERO).await;
    assert_eq!(pass.deferred, 2);
    assert!(pass.problems.is_empty(), "{:?}", pass.problems);
    assert!(fetch.asked.lock().unwrap().is_empty());
}

/// Release samples older than the keep window go at the next write; the
/// board reads thirty days of them (review of #912).
#[test]
fn release_samples_past_the_keep_window_are_pruned() {
    let dir = scratch("release-keep");
    select(&dir, LAB_ALL);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    let old = (now() - Duration::days(RELEASE_KEEP_DAYS + 1))
        .format("%Y-%m-%d")
        .to_string();
    let recent = (now() - Duration::days(5)).format("%Y-%m-%d").to_string();
    {
        let conn = db(&dir);
        for day in [&old, &recent] {
            conn.execute(
                "INSERT INTO releases (repo, day, asset_id, release_id, tag, asset, downloads, at)
                 VALUES ('example-lab/tool-a', ?1, 1, 1, 'v1', 'tool.tar.gz', 3, ?1)",
                [day],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO release_reads (repo, day, releases, assets, complete, at)
             VALUES ('example-lab/tool-z', ?1, 1, 1, 1, ?1)",
            [&old],
        )
        .unwrap();
    }
    collect_and_record(&dir, &fake(&dir, &healthy()), now() + Duration::minutes(5));
    let days = strings(
        &db(&dir),
        "SELECT day FROM releases WHERE asset_id = 1 ORDER BY day",
    );
    assert_eq!(days, [recent]);
    assert!(
        strings(
            &db(&dir),
            "SELECT day FROM release_reads WHERE repo = 'example-lab/tool-z'"
        )
        .is_empty(),
        "the read records are pruned with their samples"
    );
}

/// The release query names only fields GitHub's schema has. The fake `gh`
/// answers anything, so it could not catch `databaseId` on `ReleaseAsset`,
/// which failed every batch of the first real run (2026-11-22).
#[test]
fn the_release_query_asks_only_for_fields_github_has() {
    let q = crate::github::usage::RELEASE_FIELDS;
    let assets = &q[q.find("releaseAssets").unwrap()..];
    let asset_nodes = &assets[assets.find("nodes {").unwrap() + 7..];
    let asset_fields: Vec<&str> = asset_nodes[..asset_nodes.find('}').unwrap()]
        .split_whitespace()
        .collect();
    let known = |ty: &str| {
        crate::github::usage::RELEASE_SCHEMA
            .iter()
            .find(|(t, _)| *t == ty)
            .unwrap()
            .1
    };
    for f in &asset_fields {
        assert!(
            known("ReleaseAsset").contains(f),
            "ReleaseAsset has no field {f}"
        );
    }
    let release_nodes = &q[q.find("nodes {").unwrap() + 7..q.find("releaseAssets").unwrap()];
    for f in release_nodes.split_whitespace() {
        assert!(known("Release").contains(&f), "Release has no field {f}");
    }
}
