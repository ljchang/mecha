//! A factory instrument's home half: the manifest under
//! `~/.mecha/factory/types/`, and a booking's hours read out of it as data
//! and written back into it in place.
//!
//! **The manifest is the one file.** The slot pipeline reads its
//! `[availability]` section (`factory-publish slots push --manifest`), and
//! the box never computes from it — but it does read one field, `timezone`,
//! to label the week grid and the confirmation's times. So an hours edit
//! reaches the page with the next slot push alone, and a timezone edit would
//! need a `type push` too. That is a publish, and not this editor's to make,
//! so [`rewrite`] refuses a changed `timezone`: the zone changes in the file
//! and goes out with `factory-publish type push`, both by hand.
//!
//! **The owner is the only author.** Nothing here is a tool: availability is
//! what strangers may book of the owner's week, and widening it is the one
//! direction the factory's trust model (`SCHEDULING-DESIGN.md` §1.3) keeps
//! off the box. A model that could write it could be asked to by the prose a
//! stranger typed into the booking form.
//!
//! **A write keeps the file.** `toml_edit`, not a re-serialise: only the keys
//! the editor owns are replaced, so the owner's comments, the `[policy]`,
//! `[mail]` and `[[fields]]` sections, and any availability key the editor
//! does not know (`increment_minutes`, `[availability.poll]`) survive byte
//! for byte. Meaning is checked by the factory, not re-decided here:
//! `validate` refuses only what would make a write unreadable or a push
//! certain to fail, and `factory-publish type check` judges the rest.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many days the slot pipeline's freebusy fetch covers — the
/// `mecha-mail freebusy --days 60` in `scripts/mecha-slots.service`, which
/// `the_pipeline_unit_fetches_the_days_the_editor_allows` holds this to.
/// A horizon past it would make every push refuse ("short of the horizon"),
/// and the page would go stale while the editor said it saved.
pub const PIPELINE_DAYS: u32 = 60;

/// `~/.mecha/factory/types`, found by the factory's own rule
/// (`Remote::dir()` is `mecha_home()/factory`) — a reader of somebody else's
/// store uses their rule for finding it.
pub fn types_dir() -> Result<PathBuf> {
    Ok(crate::work::mecha_home()?.join("factory").join("types"))
}

/// One manifest on disk, as the list shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Instrument {
    pub id: String,
    /// `booking` or `request` — the manifest's own `kind`, defaulted as the
    /// factory defaults it.
    pub kind: String,
    pub title: String,
    pub path: PathBuf,
    /// Why this file could not be read as a manifest. A finding beside the
    /// others, never a silent omission: an unreadable store is not an empty
    /// one.
    pub problem: Option<String>,
}

impl Instrument {
    pub fn is_booking(&self) -> bool {
        self.kind == "booking"
    }
}

/// Every `*.toml` in `dir`, sorted by id. A missing directory is no
/// instruments (the factory is simply not set up); an unreadable one is an
/// error.
pub fn list_in(dir: &Path) -> Result<Vec<Instrument>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let parsed = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| head(&text));
        out.push(match parsed {
            Ok((id, kind, title)) => Instrument {
                id,
                kind,
                title,
                path,
                problem: None,
            },
            Err(e) => Instrument {
                id: stem,
                kind: "unknown".into(),
                title: String::new(),
                path,
                problem: Some(format!("{e:#}")),
            },
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// The instrument called `id`, by its manifest's `id` — never by a path a
/// caller supplies.
pub fn find_in(dir: &Path, id: &str) -> Result<Instrument> {
    let Some(found) = list_in(dir)?.into_iter().find(|i| i.id == id) else {
        bail!("no manifest with id `{id}` in {}", dir.display());
    };
    if let Some(problem) = &found.problem {
        bail!("{}: {problem}", found.path.display());
    }
    Ok(found)
}

fn head(text: &str) -> Result<(String, String, String)> {
    let doc: toml::Table = toml::from_str(text)?;
    let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let Some(id) = s("id") else {
        bail!("no `id`");
    };
    Ok((
        id,
        s("kind").unwrap_or_else(|| "request".into()),
        s("title").unwrap_or_default(),
    ))
}

/// A booking's hours: the `[availability]` keys the editor owns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hours {
    pub timezone: String,
    pub durations: Vec<u32>,
    pub buffer_minutes: u32,
    pub min_notice_hours: u32,
    pub horizon_days: u32,
    pub per_day_cap: Option<u32>,
    pub windows: Vec<Window>,
    /// Dates that replace the weekly pattern wholesale; no windows closes
    /// the day.
    #[serde(default)]
    pub overrides: Vec<Override>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub day: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub date: String,
    #[serde(default)]
    pub windows: Vec<[String; 2]>,
}

/// The factory's defaults for keys a manifest may leave out
/// (`mecha_manifest::availability`: `buffer_minutes` 0, `min_notice_hours`
/// 24, `horizon_days` 60). Read only for a file that omits them; every write
/// states all of them, so after one save the file says what it means.
const DEFAULT_NOTICE: u32 = 24;
const DEFAULT_HORIZON: u32 = 60;

pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Read a booking manifest's hours.
pub fn read_hours(text: &str) -> Result<Hours> {
    let doc: toml::Table = toml::from_str(text)?;
    let Some(a) = doc.get("availability").and_then(|v| v.as_table()) else {
        bail!("not a booking manifest: no [availability] section");
    };
    let int = |k: &str| -> Result<Option<u32>> {
        match a.get(k) {
            None => Ok(None),
            Some(v) => Ok(Some(
                v.as_integer()
                    .and_then(|i| u32::try_from(i).ok())
                    .with_context(|| format!("[availability] {k} is not a whole number"))?,
            )),
        }
    };
    // The windows and overrides go through serde, so a shape the editor
    // cannot represent is refused here rather than half-shown.
    #[derive(Deserialize)]
    struct Shapes {
        #[serde(default)]
        windows: Vec<Window>,
        #[serde(default)]
        overrides: Vec<Override>,
        #[serde(default)]
        durations: Vec<u32>,
        #[serde(default)]
        timezone: String,
    }
    let shapes: Shapes = toml::Value::Table(a.clone())
        .try_into()
        .context("[availability]")?;
    Ok(Hours {
        timezone: shapes.timezone,
        durations: shapes.durations,
        buffer_minutes: int("buffer_minutes")?.unwrap_or(0),
        min_notice_hours: int("min_notice_hours")?.unwrap_or(DEFAULT_NOTICE),
        horizon_days: int("horizon_days")?.unwrap_or(DEFAULT_HORIZON),
        per_day_cap: int("per_day_cap")?,
        windows: shapes.windows,
        overrides: shapes.overrides,
    })
}

/// What would make a write unreadable or a push certain to fail. The
/// factory's `type check` is the judge of meaning; this is the part that
/// has to be said in the editor's own terms, before a child is spawned.
pub fn validate(h: &Hours) -> Result<()> {
    let hhmm = |t: &str| {
        let ok = t.len() == 5
            && t.as_bytes()[2] == b':'
            && t[..2].parse::<u32>().is_ok_and(|h| h < 24)
            && t[3..].parse::<u32>().is_ok_and(|m| m < 60);
        if ok {
            Ok(())
        } else {
            Err(anyhow::anyhow!("`{t}` is not an HH:MM time"))
        }
    };
    if h.durations.is_empty() {
        bail!("offer at least one meeting length");
    }
    if h.windows.is_empty() {
        bail!("open at least one weekly window — with none, nothing is ever offered");
    }
    if h.horizon_days == 0 || h.horizon_days > PIPELINE_DAYS {
        bail!(
            "how far ahead must be 1–{PIPELINE_DAYS} days: the slot timer fetches \
             {PIPELINE_DAYS} days of your calendar, and a longer horizon would make \
             every push refuse"
        );
    }
    for w in &h.windows {
        if !DAYS.contains(&w.day.as_str()) {
            bail!("`{}` is not a weekday (mon–sun)", w.day);
        }
        hhmm(&w.start)?;
        hhmm(&w.end)?;
        if w.start >= w.end {
            bail!("{} {}–{} ends before it starts", w.day, w.start, w.end);
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for o in &h.overrides {
        let ok =
            o.date.len() == 10 && chrono::NaiveDate::parse_from_str(&o.date, "%Y-%m-%d").is_ok();
        if !ok {
            bail!("`{}` is not a YYYY-MM-DD date", o.date);
        }
        if !seen.insert(o.date.as_str()) {
            bail!("{} is overridden twice", o.date);
        }
        for [s, e] in &o.windows {
            hhmm(s)?;
            hhmm(e)?;
            if s >= e {
                bail!("{} {s}–{e} ends before it starts", o.date);
            }
        }
    }
    Ok(())
}

/// `text` with its hours replaced by `h`, and everything else kept.
pub fn write_hours(text: &str, h: &Hours) -> Result<String> {
    use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};
    validate(h)?;
    let mut doc: DocumentMut = text.parse().context("parsing the manifest")?;
    let Some(a) = doc.get_mut("availability").and_then(Item::as_table_mut) else {
        bail!("not a booking manifest: no [availability] section");
    };
    a["timezone"] = value(h.timezone.as_str());
    a["durations"] = value(h.durations.iter().map(|d| i64::from(*d)).collect::<Array>());
    a["buffer_minutes"] = value(i64::from(h.buffer_minutes));
    a["min_notice_hours"] = value(i64::from(h.min_notice_hours));
    a["horizon_days"] = value(i64::from(h.horizon_days));
    match h.per_day_cap {
        Some(cap) => a["per_day_cap"] = value(i64::from(cap)),
        None => {
            a.remove("per_day_cap");
        }
    }
    // New tables have no document position and would print at the end of
    // the file, after `[confirmation]`; they take the place of the ones they
    // replace, or follow the `[availability]` header when there were none.
    let anchor = |name: &str, a: &Table| -> Option<isize> {
        a.get(name)
            .and_then(Item::as_array_of_tables)
            .and_then(|t| t.iter().next())
            .and_then(Table::position)
            .or(a.position())
    };
    let windows_at = anchor("windows", a);
    let mut windows = ArrayOfTables::new();
    for w in &h.windows {
        let mut t = Table::new();
        t["day"] = value(w.day.as_str());
        t["start"] = value(w.start.as_str());
        t["end"] = value(w.end.as_str());
        t.set_position(windows_at);
        windows.push(t);
    }
    a["windows"] = Item::ArrayOfTables(windows);
    // Exceptions read after the weekly pattern they amend: where they were,
    // or with the windows when this is the first.
    let overrides_at = a
        .get("overrides")
        .and_then(Item::as_array_of_tables)
        .and_then(|t| t.iter().next())
        .and_then(Table::position)
        .or(windows_at);
    if h.overrides.is_empty() {
        a.remove("overrides");
    } else {
        let mut overrides = ArrayOfTables::new();
        for o in &h.overrides {
            let mut t = Table::new();
            t["date"] = value(o.date.as_str());
            if !o.windows.is_empty() {
                let pairs: Array = o
                    .windows
                    .iter()
                    .map(|[s, e]| {
                        let mut pair = Array::new();
                        pair.push(s.as_str());
                        pair.push(e.as_str());
                        toml_edit::Value::Array(pair)
                    })
                    .collect();
                t["windows"] = value(pairs);
            }
            t.set_position(overrides_at);
            overrides.push(t);
        }
        a["overrides"] = Item::ArrayOfTables(overrides);
    }
    Ok(doc.to_string())
}

/// The manifest as a page read it, for a save to prove it is editing the
/// file that is on disk now — the charter's `digest`, one store over.
pub fn digest(text: &str) -> String {
    crate::charter::digest(text)
}

/// A manifest is a few kilobytes; a save body orders of magnitude past that
/// is not an edit of one.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// `text` with its hours replaced, refused if the file is no longer the one
/// the page read (`base`, its [`digest`]).
pub fn rewrite(disk: &str, base: &str, h: &Hours) -> Result<String> {
    if digest(disk) != base {
        bail!(Stale);
    }
    let current = read_hours(disk)?;
    if h.timezone != current.timezone {
        bail!(
            "the time zone is read by the booking page itself, so changing it is a \
             `factory-publish type push`, not an hours save: edit `timezone` in the \
             manifest and push it (the page would otherwise label {} slots in {})",
            h.timezone,
            current.timezone
        );
    }
    write_hours(disk, h)
}

/// The manifest changed between the page's read and its save.
#[derive(Debug)]
pub struct Stale;

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "the manifest changed since this page read it — reload to see the change, then edit again",
        )
    }
}

impl std::error::Error for Stale {}

/// Replace `path` with `text`, atomically, keeping its permissions: a
/// sibling temp file renamed over it, so the timer never reads half a file.
/// Keyed per call, not per process — one server answers many requests, and
/// two saves sharing a temp path can each report the other's outcome.
/// Hidden and not `.toml`, so a list taken mid-write never sees it.
pub fn replace_file(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("a manifest path has a directory")?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manifest"),
        uuid::Uuid::new_v4()
    ));
    let written = std::fs::write(&tmp, text)
        .with_context(|| format!("writing {}", tmp.display()))
        .and_then(|()| {
            if let Ok(meta) = std::fs::metadata(path) {
                std::fs::set_permissions(&tmp, meta.permissions())?;
            }
            std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    // A made-up manifest in the shipped starter's shape — comments, the
    // box's sections, and an availability key the editor does not own.
    const MANIFEST: &str = r#"# The owner's own note, which a save must keep.

id = "book"
version = 1
kind = "booking"
title = "Book a meeting"

[availability]
timezone = "America/New_York"
durations = [30, 60]
buffer_minutes = 10
increment_minutes = 15  # not the editor's: kept as written

[[availability.windows]]
day = "tue"
start = "13:00"
end = "17:00"

[policy]
hold_minutes = 30

[[fields]]
name = "requester_email"
label = "Your email"
kind = "email"
required = true

[confirmation]
heading = "You're booked"
"#;

    fn hours() -> Hours {
        read_hours(MANIFEST).unwrap()
    }

    #[test]
    fn hours_read_with_the_factorys_defaults_for_what_is_left_out() {
        let h = hours();
        assert_eq!(h.timezone, "America/New_York");
        assert_eq!(h.durations, vec![30, 60]);
        assert_eq!(h.buffer_minutes, 10);
        assert_eq!(h.min_notice_hours, 24);
        assert_eq!(h.horizon_days, 60);
        assert_eq!(h.per_day_cap, None);
        assert_eq!(h.windows.len(), 1);
        assert!(h.overrides.is_empty());
    }

    #[test]
    fn a_write_replaces_the_hours_and_keeps_everything_else() {
        let mut h = hours();
        h.windows = vec![
            Window {
                day: "mon".into(),
                start: "09:00".into(),
                end: "11:30".into(),
            },
            Window {
                day: "fri".into(),
                start: "14:00".into(),
                end: "16:00".into(),
            },
        ];
        h.per_day_cap = Some(2);
        h.overrides = vec![
            Override {
                date: "2026-12-24".into(),
                windows: vec![],
            },
            Override {
                date: "2026-12-30".into(),
                windows: vec![["10:00".into(), "12:00".into()]],
            },
        ];
        let out = write_hours(MANIFEST, &h).unwrap();
        // Round trip: what was written is what reads back.
        assert_eq!(read_hours(&out).unwrap(), h);
        // The owner's comments and the box's sections survive.
        assert!(out.starts_with("# The owner's own note, which a save must keep."));
        assert!(out.contains("increment_minutes = 15  # not the editor's: kept as written"));
        assert!(out.contains("[policy]\nhold_minutes = 30"));
        assert!(out.contains("[confirmation]"));
        // The new tables sit where the old ones were — inside the
        // availability block, before [policy] — not at the end of the file.
        let windows_at = out.find("[[availability.windows]]").unwrap();
        let overrides_at = out.find("[[availability.overrides]]").unwrap();
        assert!(
            windows_at < overrides_at,
            "exceptions follow the week: {out}"
        );
        let policy_at = out.find("[policy]").unwrap();
        assert!(windows_at < policy_at, "{out}");
        assert!(overrides_at < policy_at, "{out}");
        assert!(!out.contains("tue"), "the old window is gone: {out}");
        // A closed day is a date with no windows key at all.
        let closed = &out[out.find("2026-12-24").unwrap()..];
        let closed = &closed[..closed.find("[[").unwrap_or(closed.len())];
        assert!(!closed.contains("windows"), "{closed}");
    }

    #[test]
    fn removing_the_cap_and_the_overrides_removes_their_keys() {
        let mut h = hours();
        h.per_day_cap = Some(3);
        h.overrides = vec![Override {
            date: "2026-12-24".into(),
            windows: vec![],
        }];
        let with = write_hours(MANIFEST, &h).unwrap();
        h.per_day_cap = None;
        h.overrides.clear();
        let without = write_hours(&with, &h).unwrap();
        assert!(!without.contains("per_day_cap"), "{without}");
        assert!(!without.contains("overrides"), "{without}");
    }

    #[test]
    fn what_would_break_a_push_is_refused_before_it_is_written() {
        let refuse = |edit: fn(&mut Hours), says: &str| {
            let mut h = hours();
            edit(&mut h);
            let err = write_hours(MANIFEST, &h).unwrap_err().to_string();
            assert!(err.contains(says), "`{err}` should say `{says}`");
        };
        // The pipeline fetches PIPELINE_DAYS; a longer horizon fails every push.
        refuse(|h| h.horizon_days = PIPELINE_DAYS + 1, "slot timer fetches");
        refuse(|h| h.horizon_days = 0, "1–60");
        refuse(|h| h.windows.clear(), "at least one weekly window");
        refuse(|h| h.durations.clear(), "meeting length");
        refuse(|h| h.windows[0].day = "tuesday".into(), "not a weekday");
        refuse(
            |h| h.windows[0].end = "12:00".into(),
            "ends before it starts",
        );
        refuse(|h| h.windows[0].start = "1pm".into(), "HH:MM");
        refuse(
            |h| {
                h.overrides = vec![
                    Override {
                        date: "2026-12-24".into(),
                        windows: vec![],
                    },
                    Override {
                        date: "2026-12-24".into(),
                        windows: vec![],
                    },
                ]
            },
            "overridden twice",
        );
        refuse(
            |h| {
                h.overrides = vec![Override {
                    date: "12/24/2026".into(),
                    windows: vec![],
                }]
            },
            "YYYY-MM-DD",
        );
    }

    #[test]
    fn the_time_zone_is_not_an_hours_edit() {
        // The box labels the page in this zone; a save that changed it
        // without a type push would sell new-zone slots under old-zone labels.
        let mut h = hours();
        h.timezone = "Europe/Helsinki".into();
        let err = rewrite(MANIFEST, &digest(MANIFEST), &h).unwrap_err();
        assert!(err.to_string().contains("type push"), "{err:#}");
    }

    #[test]
    fn a_save_over_a_file_that_changed_since_the_read_is_refused() {
        let base = digest(MANIFEST);
        assert!(rewrite(MANIFEST, &base, &hours()).is_ok());
        let edited_elsewhere = MANIFEST.replace("Book a meeting", "Book time");
        let err = rewrite(&edited_elsewhere, &base, &hours()).unwrap_err();
        assert!(err.downcast_ref::<Stale>().is_some(), "{err:#}");
    }

    #[test]
    fn a_request_type_has_no_hours_to_edit() {
        let plain = "id = \"letter\"\ntitle = \"A letter\"\n";
        assert!(read_hours(plain).is_err());
        assert!(write_hours(plain, &hours()).is_err());
    }

    #[test]
    fn the_list_reads_every_manifest_and_names_the_one_it_cannot() {
        let root = std::env::temp_dir().join(format!("mecha-instrument-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = Gone(root);
        let dir = &dir.0;
        std::fs::write(dir.join("book.toml"), MANIFEST).unwrap();
        std::fs::write(
            dir.join("letter.toml"),
            "id = \"letter\"\ntitle = \"A letter\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "id = [").unwrap();
        std::fs::write(dir.join("notes.md"), "not a manifest").unwrap();
        let all = list_in(dir).unwrap();
        let ids: Vec<_> = all.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["book", "broken", "letter"]);
        assert!(all[0].is_booking());
        assert_eq!(all[2].kind, "request");
        assert!(
            all[1].problem.is_some(),
            "an unreadable manifest is a finding"
        );
        // Found by id, and the broken one refuses rather than half-loads.
        assert_eq!(find_in(dir, "book").unwrap().title, "Book a meeting");
        assert!(find_in(dir, "broken").is_err());
        assert!(find_in(dir, "nope").is_err());
        // A missing directory is a factory not set up, not an error.
        assert!(list_in(&dir.join("absent")).unwrap().is_empty());
    }

    #[test]
    fn the_pipelines_fetch_the_days_the_editor_allows() {
        // PIPELINE_DAYS is one number in every script that runs the push:
        // the timer, and the drain follower's push after a booking lands.
        // If one fetches fewer days than the editor lets a horizon reach, a
        // saved horizon fails that push; the follower's failure is
        // swallowed by its `|| echo`, so nothing else would say so.
        let scripts = [
            (
                "mecha-slots.service",
                include_str!("../../scripts/mecha-slots.service"),
            ),
            (
                "drain-follow.sh",
                include_str!("../../scripts/drain-follow.sh"),
            ),
        ];
        for (name, text) in scripts {
            let fetches: Vec<&str> = text
                .lines()
                .filter(|l| !l.trim_start().starts_with('#') && l.contains("freebusy --days"))
                .collect();
            assert!(!fetches.is_empty(), "{name} runs a freebusy fetch");
            for fetch in fetches {
                let days: u32 = fetch
                    .split("--days ")
                    .nth(1)
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("{name}: the fetch names --days: {fetch}"));
                assert_eq!(days, PIPELINE_DAYS, "{name}: {fetch}");
            }
            assert!(
                text.contains("slots push book --manifest"),
                "{name} computes slots from the manifest the editor writes"
            );
        }
    }
}
