//! Step 6's picker against the fake `gh`: select-all on an owner writes the
//! file the collector then obeys (the step's acceptance), the per-repository
//! lines round-trip, the owner's comments survive, a stale page and a result
//! the collector would refuse write nothing, an unreadable store or file is
//! said, and package corrections round-trip through step 5's reader. Never
//! the network; fixtures are made up.

use super::*;
use crate::github::picker::{
    self, save_packages, save_selection, FileState, PackageChange, PackageState, PackageTo,
    Refusal, SelectionChange, StoreView, View,
};
use crate::github::usage::{packages_path, read_packages, PackagesRead, Registry};

const LAB_LISTED: &str = "[[owner]]\nlogin = \"example-lab\"\ninclude = \"listed\"\n";

fn home(dir: &Path) -> PathBuf {
    dir.join("home")
}

/// A first collection under `toml`, so the store holds the inventory the
/// page lists from.
fn collected(name: &str, toml: &str) -> PathBuf {
    let dir = scratch(name);
    select(&dir, toml);
    collect_and_record(&dir, &fake(&dir, &healthy()), now());
    dir
}

fn owner<'a>(v: &'a View, login: &str) -> &'a picker::OwnerView {
    v.owners
        .iter()
        .find(|o| o.login == login)
        .unwrap_or_else(|| panic!("no owner {login} in {:?}", v.owners))
}

fn repo<'a>(v: &'a View, full: &str) -> &'a picker::RepoView {
    v.owners
        .iter()
        .flat_map(|o| &o.repos)
        .find(|r| r.full_name == full)
        .unwrap_or_else(|| panic!("no repository {full}"))
}

fn included(v: &View) -> Vec<String> {
    v.owners
        .iter()
        .flat_map(|o| &o.repos)
        .filter(|r| r.included == Some(true))
        .map(|r| r.full_name.clone())
        .collect()
}

fn base(home: &Path) -> Option<String> {
    picker::view(home).selection.digest
}

fn selection_text(home: &Path) -> String {
    std::fs::read_to_string(selection_path(home)).unwrap()
}

// ---- the acceptance ----

/// §7 step 6: select-all on an owner writes the file the collector then
/// obeys — the collector's own reader over the written bytes, the run it
/// drives, and the set a board's links are checked against.
#[test]
fn select_all_on_an_owner_writes_the_file_the_collector_then_obeys() {
    let dir = collected("picker-accept", LAB_LISTED);
    let home = home(&dir);
    let before = picker::view(&home);
    assert_eq!(
        included(&before),
        Vec::<String>::new(),
        "nothing is selected yet"
    );
    assert_eq!(owner(&before, "example-lab").repos.len(), 4);
    assert_eq!(owner(&before, "example-owner").repos.len(), 1);

    save_selection(
        &home,
        before.selection.digest.as_deref(),
        &[SelectionChange::SelectAll {
            login: "example-lab".into(),
        }],
    )
    .unwrap();

    // The collector's reader, over the file as written.
    let SelectionRead::Read(s) = read_selection(&selection_path(&home)) else {
        panic!("the collector cannot read what the page wrote");
    };
    let inventory = prior_inventory(&db_path(&home));
    let mut chosen: Vec<&str> = inventory
        .iter()
        .filter(|(r, _)| s.selects(r))
        .map(|(r, _)| r.full_name.as_str())
        .collect();
    chosen.sort();
    assert_eq!(
        chosen,
        ["example-lab/tool-a", "example-lab/tool-b"],
        "every repository of the owner, less its fork and its archived one (G4)"
    );

    // The collector's run under it watches exactly those.
    let c = collect_and_record(&dir, &fake(&dir, &healthy()), now());
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    assert_eq!(c.selected, Some(2));
    let mut watched: Vec<String> = c
        .inventory
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(_, sel)| *sel)
        .map(|(r, _)| r.full_name.clone())
        .collect();
    watched.sort();
    assert_eq!(watched, ["example-lab/tool-a", "example-lab/tool-b"]);
    assert!(calls(&dir).iter().all(|c| !c.contains("fork-of-upstream")));

    // And the set a board's issue links are checked against (§4).
    let w = Watched::read(&home);
    assert!(w.contains("example-lab", "tool-a") && w.contains("example-lab", "tool-b"));
    assert!(!w.contains("example-lab", "fork-of-upstream"));
    assert!(!w.contains("example-lab", "old-site"));
    assert!(!w.contains("example-owner", "notes"));

    // The page now says what the collector did.
    let after = picker::view(&home);
    assert!(repo(&after, "example-lab/tool-a").collected);
    assert_eq!(
        included(&after),
        ["example-lab/tool-a", "example-lab/tool-b"]
    );
}

/// Select-all removes the owner's exclusions — or it would not select all —
/// and keeps a line that takes a fork; select-none removes the owner's lines
/// either way and selects nothing of it.
#[test]
fn select_all_drops_the_owners_exclusions_and_select_none_drops_its_inclusions() {
    let dir = collected(
        "picker-all-none",
        "[[owner]]\nlogin = \"example-lab\"\ninclude = \"listed\"\n\n\
         [[repo]]\nname = \"example-lab/tool-b\"\ninclude = false\n\n\
         [[repo]]\nname = \"example-lab/fork-of-upstream\"\ninclude = true\n\n\
         [[repo]]\nname = \"example-owner/notes\"\ninclude = false\n",
    );
    let home = home(&dir);
    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::SelectAll {
            login: "example-lab".into(),
        }],
    )
    .unwrap();
    let v = picker::view(&home);
    assert_eq!(
        included(&v),
        [
            "example-lab/fork-of-upstream",
            "example-lab/tool-a",
            "example-lab/tool-b"
        ]
    );
    assert!(
        selection_text(&home).contains("example-owner/notes"),
        "another owner's line is not this owner's to remove"
    );

    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::SelectNone {
            login: "example-lab".into(),
        }],
    )
    .unwrap();
    let v = picker::view(&home);
    assert_eq!(included(&v), Vec::<String>::new());
    let text = selection_text(&home);
    assert!(!text.contains("example-lab/"), "{text}");
    assert!(text.contains("include = \"listed\""), "{text}");
}

/// The owner's all-or-listed switch undone gives the hand-made inclusions
/// back: select-all and then select-listed is where the owner started, and
/// only the explicit select-none clears the owner's lines (review of #913).
#[test]
fn all_then_listed_gives_the_owners_inclusions_back_and_only_none_clears_them() {
    let dir = collected(
        "picker-all-listed",
        "[[owner]]\nlogin = \"example-lab\"\ninclude = \"listed\"\n\n\
         [[repo]]\nname = \"example-lab/tool-b\"\ninclude = true # picked by hand\n\n\
         [[repo]]\nname = \"example-lab/fork-of-upstream\"\ninclude = true\n",
    );
    let home = home(&dir);
    let start = included(&picker::view(&home));
    assert_eq!(
        start,
        ["example-lab/fork-of-upstream", "example-lab/tool-b"]
    );
    let lab = |op: fn(String) -> SelectionChange| [op("example-lab".into())];

    save_selection(
        &home,
        base(&home).as_deref(),
        &lab(|login| SelectionChange::SelectAll { login }),
    )
    .unwrap();
    assert_eq!(
        included(&picker::view(&home)),
        [
            "example-lab/fork-of-upstream",
            "example-lab/tool-a",
            "example-lab/tool-b"
        ]
    );
    save_selection(
        &home,
        base(&home).as_deref(),
        &lab(|login| SelectionChange::SelectListed { login }),
    )
    .unwrap();
    assert_eq!(included(&picker::view(&home)), start, "the undo is an undo");
    let text = selection_text(&home);
    assert!(text.contains("include = \"listed\""), "{text}");
    assert!(text.contains("include = true # picked by hand"), "{text}");

    save_selection(
        &home,
        base(&home).as_deref(),
        &lab(|login| SelectionChange::SelectNone { login }),
    )
    .unwrap();
    assert_eq!(included(&picker::view(&home)), Vec::<String>::new());
    assert!(!selection_text(&home).contains("[[repo]]"));
}

// ---- one repository at a time ----

#[test]
fn a_repositorys_own_line_round_trips_and_keeps_the_owners_comments() {
    let dir = collected(
        "picker-repo",
        "# Mine: the lab's tools.\n[[owner]]\nlogin = \"example-lab\" # the lab\ninclude = \"all\"\nforks = false\narchived = false\n",
    );
    let home = home(&dir);
    assert_eq!(
        included(&picker::view(&home)),
        ["example-lab/tool-a", "example-lab/tool-b"]
    );

    // Out: a line that says so, over the owner's.
    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::Repo {
            name: "example-lab/tool-b".into(),
            include: Some(false),
        }],
    )
    .unwrap();
    let v = picker::view(&home);
    let b = repo(&v, "example-lab/tool-b");
    assert_eq!(
        (b.included, b.by_owner, b.line),
        (Some(false), Some(true), Some(false))
    );
    // In: a fork, which the owner's line leaves out (G4).
    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::Repo {
            name: "example-lab/fork-of-upstream".into(),
            include: Some(true),
        }],
    )
    .unwrap();
    assert_eq!(
        included(&picker::view(&home)),
        ["example-lab/fork-of-upstream", "example-lab/tool-a"]
    );
    // The same line again, flipped: edited in place, not a second line.
    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::Repo {
            name: "Example-Lab/Tool-B".into(),
            include: Some(true),
        }],
    )
    .unwrap();
    assert_eq!(selection_text(&home).matches("tool-b").count(), 1);
    // Back to the owner's line: the exception goes.
    save_selection(
        &home,
        base(&home).as_deref(),
        &[
            SelectionChange::Repo {
                name: "example-lab/tool-b".into(),
                include: None,
            },
            SelectionChange::Repo {
                name: "example-lab/fork-of-upstream".into(),
                include: None,
            },
        ],
    )
    .unwrap();
    let text = selection_text(&home);
    assert!(!text.contains("[[repo]]"), "{text}");
    assert!(text.starts_with("# Mine: the lab's tools.\n"), "{text}");
    assert!(text.contains("# the lab"), "{text}");
    assert_eq!(
        included(&picker::view(&home)),
        ["example-lab/tool-a", "example-lab/tool-b"]
    );
}

/// A file the page makes says where it came from, writes G4's filters out,
/// and leaves forks and archived repositories out until the owner's line
/// takes them.
#[test]
fn a_new_file_says_who_wrote_it_and_excludes_forks_and_archived_by_default() {
    let dir = collected("picker-new", LAB_LISTED);
    let home = home(&dir);
    std::fs::remove_file(selection_path(&home)).unwrap();
    let v = picker::view(&home);
    assert_eq!(v.selection.state, FileState::Absent);
    assert_eq!(v.selection.digest, None);
    let a = repo(&v, "example-lab/tool-a");
    assert_eq!(
        (a.included, a.by_owner),
        (Some(false), Some(false)),
        "no file selects nothing, which is known — and editable"
    );

    save_selection(
        &home,
        None,
        &[SelectionChange::SelectAll {
            login: "example-lab".into(),
        }],
    )
    .unwrap();
    let text = selection_text(&home);
    assert!(text.starts_with("# Which repositories"), "{text}");
    assert!(
        text.contains("forks = false") && text.contains("archived = false"),
        "{text}"
    );
    assert!(parse_selection(&text).is_ok(), "{text}");
    let v = picker::view(&home);
    assert_eq!(
        repo(&v, "example-lab/fork-of-upstream").included,
        Some(false)
    );
    assert_eq!(repo(&v, "example-lab/old-site").included, Some(false));

    save_selection(
        &home,
        base(&home).as_deref(),
        &[SelectionChange::Filters {
            login: "example-lab".into(),
            forks: true,
            archived: false,
        }],
    )
    .unwrap();
    let v = picker::view(&home);
    assert_eq!(
        repo(&v, "example-lab/fork-of-upstream").included,
        Some(true)
    );
    assert_eq!(repo(&v, "example-lab/old-site").included, Some(false));
    assert!(
        selection_text(&home).starts_with("# Which repositories"),
        "the header stays on top"
    );
}

// ---- what is refused ----

/// A hand edit between the page's read and its save is never written over:
/// the save is refused, and the file is the hand edit's.
#[test]
fn a_save_from_a_page_that_read_an_older_file_is_refused_and_writes_nothing() {
    let dir = collected("picker-stale", LAB_LISTED);
    let home = home(&dir);
    let read = base(&home);
    let hand = format!("{LAB_LISTED}\n# edited by hand\n");
    std::fs::write(selection_path(&home), &hand).unwrap();
    let all = [SelectionChange::SelectAll {
        login: "example-lab".into(),
    }];
    assert_eq!(
        save_selection(&home, read.as_deref(), &all),
        Err(Refusal::Stale)
    );
    assert_eq!(selection_text(&home), hand);
    // A page that read no file, when there is one now; and one that read a
    // file since removed.
    assert_eq!(save_selection(&home, None, &all), Err(Refusal::Stale));
    std::fs::remove_file(selection_path(&home)).unwrap();
    assert_eq!(
        save_selection(&home, Some(&picker::digest(&hand)), &all),
        Err(Refusal::Stale)
    );
    assert!(!selection_path(&home).exists());
}

/// The collector's own reader is the last gate: an edit that would make a
/// file it refuses writes nothing, whatever the checks before it let by.
#[test]
fn a_result_the_collectors_reader_would_refuse_writes_nothing() {
    let dir = collected("picker-invalid", LAB_LISTED);
    let home = home(&dir);
    let path = selection_path(&home);
    let read = base(&home);
    let refused = picker::save(
        &path,
        read.as_deref(),
        "",
        |t| parse_selection(t).map(|_| ()),
        |doc| {
            let mut t = toml_edit::Table::new();
            t.insert("login", toml_edit::value("not a login"));
            doc["owner"].as_array_of_tables_mut().unwrap().push(t);
            Ok(())
        },
    );
    assert!(
        matches!(&refused, Err(Refusal::Invalid(why)) if why.contains("collector would refuse")),
        "{refused:?}"
    );
    assert_eq!(selection_text(&home), LAB_LISTED);

    // And the checks before it, by name.
    for bad in [
        SelectionChange::Repo {
            name: "example-lab/..".into(),
            include: Some(true),
        },
        SelectionChange::Repo {
            name: "no-slash".into(),
            include: Some(true),
        },
        SelectionChange::SelectAll {
            login: "bad login".into(),
        },
    ] {
        let r = save_selection(&home, read.as_deref(), std::slice::from_ref(&bad));
        assert!(matches!(r, Err(Refusal::Invalid(_))), "{bad:?}: {r:?}");
        assert_eq!(selection_text(&home), LAB_LISTED);
    }
    assert!(matches!(
        save_selection(&home, read.as_deref(), &[]),
        Err(Refusal::BadRequest(_))
    ));
}

/// A file the collector cannot read is the owner's to fix by hand: the page
/// says so and edits nothing in it, rather than replacing it.
#[test]
fn a_file_the_collector_cannot_read_is_said_and_never_edited() {
    let dir = collected("picker-unusable", LAB_LISTED);
    let home = home(&dir);
    let broken = "[[owner]]\nlogin = \"example-lab\"\nincluded = \"all\"\n";
    std::fs::write(selection_path(&home), broken).unwrap();
    let v = picker::view(&home);
    assert_eq!(v.selection.state, FileState::Invalid);
    assert!(v.selection.error.as_deref().unwrap().contains("included"));
    assert_eq!(
        repo(&v, "example-lab/tool-a").included,
        None,
        "unknown, not unselected"
    );
    let r = save_selection(
        &home,
        v.selection.digest.as_deref(),
        &[SelectionChange::SelectAll {
            login: "example-lab".into(),
        }],
    );
    assert!(matches!(r, Err(Refusal::Unusable(_))), "{r:?}");
    assert_eq!(selection_text(&home), broken);

    // An inline list is the owner's form to keep.
    let inline = "owner = [{ login = \"example-lab\", include = \"listed\" }]\n";
    std::fs::write(selection_path(&home), inline).unwrap();
    let r = save_selection(
        &home,
        Some(&picker::digest(inline)),
        &[SelectionChange::SelectAll {
            login: "example-lab".into(),
        }],
    );
    assert!(matches!(r, Err(Refusal::Unusable(_))), "{r:?}");
    assert_eq!(selection_text(&home), inline);
}

/// No store, and a store that cannot be read, are said — never an empty
/// inventory standing for one.
#[test]
fn an_absent_or_unreadable_store_is_said_not_shown_empty() {
    let dir = scratch("picker-store");
    let home = home(&dir);
    select(&dir, LAB_LISTED);
    let v = picker::view(&home);
    assert!(matches!(v.store, StoreView::Absent { .. }), "{:?}", v.store);
    assert_eq!(
        owner(&v, "example-lab").repos.len(),
        0,
        "the file's owner, listed"
    );

    std::fs::write(db_path(&home), b"this is not a database").unwrap();
    let v = picker::view(&home);
    assert!(
        matches!(&v.store, StoreView::Unreadable { error, .. } if !error.is_empty()),
        "{:?}",
        v.store
    );
    assert_eq!(v.selection.state, FileState::Read);

    // A store the collector wrote: when it last ran and listed.
    let dir = collected("picker-store-read", LAB_LISTED);
    let v = picker::view(&home_of(&dir));
    let StoreView::Read {
        listed_at,
        last_collect,
        ..
    } = &v.store
    else {
        panic!("{:?}", v.store)
    };
    assert!(listed_at.is_some());
    assert!(last_collect
        .as_ref()
        .is_some_and(|l| l.finished >= l.started));
    assert!(v
        .schedule
        .as_deref()
        .is_some_and(|s| s.starts_with("daily at ")));
}

fn home_of(dir: &Path) -> PathBuf {
    home(dir)
}

// ---- the package mapping ----

/// A correction set, changed and cleared through the page, read back by
/// step 5's own reader each time; the owner's comment on a line survives.
#[test]
fn a_package_correction_round_trips_through_step_fives_reader() {
    let dir = collected("picker-packages", LAB_LISTED);
    let home = home(&dir);
    let v = picker::view(&home);
    assert_eq!(v.packages.state, FileState::Absent);
    let pypi = |v: &View| {
        repo(v, "example-lab/tool-a")
            .packages
            .iter()
            .find(|p| p.registry == Registry::Pypi)
            .cloned()
            .unwrap()
    };
    assert_eq!(pypi(&v).state, PackageState::NotScanned);

    let set = |base: Option<String>, to: PackageTo| {
        save_packages(
            &home,
            base.as_deref(),
            &[PackageChange {
                repo: "example-lab/tool-a".into(),
                registry: Registry::Pypi,
                to,
            }],
        )
    };
    // Clearing a line in a file that is not there creates no file.
    assert_eq!(set(None, PackageTo::Detected), Ok(None));
    assert!(!packages_path(&home).exists());

    set(None, PackageTo::Name("Tool_A.Py".into())).unwrap();
    let PackagesRead::Read(rules) = read_packages(&packages_path(&home)) else {
        panic!("step 5's reader cannot read what the page wrote");
    };
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name.as_deref(), Some("tool-a-py"));
    let v = picker::view(&home);
    assert_eq!(
        (pypi(&v).state, pypi(&v).name),
        (PackageState::Owner, Some("tool-a-py".into()))
    );

    // The owner annotates the line by hand; "publishes nothing" keeps it.
    let text = std::fs::read_to_string(packages_path(&home)).unwrap();
    let annotated = text.replace("name = ", "# renamed on PyPI in 2025\nname = ");
    std::fs::write(packages_path(&home), &annotated).unwrap();
    set(v.packages.digest.clone(), PackageTo::None).unwrap_err();
    set(Some(picker::digest(&annotated)), PackageTo::None).unwrap();
    let text = std::fs::read_to_string(packages_path(&home)).unwrap();
    assert!(
        text.contains("# renamed on PyPI in 2025\nnone = true"),
        "{text}"
    );
    assert!(!text.contains("name ="), "{text}");
    let PackagesRead::Read(rules) = read_packages(&packages_path(&home)) else {
        panic!("{text}");
    };
    assert!(rules[0].none && rules[0].name.is_none());
    assert_eq!(pypi(&picker::view(&home)).state, PackageState::OwnerNone);

    // Cleared: back to what the manifests say.
    set(picker::view(&home).packages.digest, PackageTo::Detected).unwrap();
    let PackagesRead::Read(rules) = read_packages(&packages_path(&home)) else {
        panic!();
    };
    assert!(rules.is_empty());
    assert_eq!(pypi(&picker::view(&home)).state, PackageState::NotScanned);

    // A name outside the registry's grammar is refused before the write.
    let before = std::fs::read_to_string(packages_path(&home)).unwrap();
    let r = set(
        picker::view(&home).packages.digest,
        PackageTo::Name("../../etc".into()),
    );
    assert!(matches!(r, Err(Refusal::Invalid(_))), "{r:?}");
    assert_eq!(
        std::fs::read_to_string(packages_path(&home)).unwrap(),
        before
    );
}

/// What the manifests said shows as detected, a scan that found nothing as
/// "none found", and an unreadable `packages.toml` as such.
#[test]
fn the_mapping_shows_detected_none_found_and_an_unusable_file() {
    let dir = scratch("picker-detected");
    select(&dir, LAB_ALL);
    let cases = with(vec![ok(
        "*'fragment M on Repository'*",
        &batch(&[
            manifests(&[("pyproject", "[project]\nname = \"Tool-A\"\n")]),
            manifests(&[]),
        ]),
    )]);
    collect_and_record(&dir, &fake(&dir, &cases), now());
    let home = home(&dir);
    let v = picker::view(&home);
    let state = |v: &View, full: &str, reg: Registry| {
        repo(v, full)
            .packages
            .iter()
            .find(|p| p.registry == reg)
            .map(|p| (p.state, p.name.clone()))
            .unwrap()
    };
    assert_eq!(
        state(&v, "example-lab/tool-a", Registry::Pypi),
        (PackageState::Detected, Some("tool-a".into()))
    );
    assert_eq!(
        state(&v, "example-lab/tool-b", Registry::Pypi),
        (PackageState::NoneFound, None)
    );
    assert_eq!(
        state(&v, "example-lab/fork-of-upstream", Registry::Npm),
        (PackageState::NotScanned, None),
        "a repository not selected is not scanned"
    );

    std::fs::write(packages_path(&home), "[[package]]\nrepo = 1\n").unwrap();
    let v = picker::view(&home);
    assert_eq!(v.packages.state, FileState::Invalid);
    assert!(v.packages.error.is_some());
    let r = save_packages(
        &home,
        v.packages.digest.as_deref(),
        &[PackageChange {
            repo: "example-lab/tool-a".into(),
            registry: Registry::Npm,
            to: PackageTo::None,
        }],
    );
    assert!(matches!(r, Err(Refusal::Unusable(_))), "{r:?}");
}
