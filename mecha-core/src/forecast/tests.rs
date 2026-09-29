use super::*;
use crate::agent::Taint;
use crate::outbox::{OutboxStore, Provenance};
use serde_json::json;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-forecast-{tag}-{}", uuid::Uuid::new_v4()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const DAY: i64 = 24;

fn hours(h: i64) -> chrono::Duration {
    chrono::Duration::hours(h)
}

/// A model-authored message draft staged `ago` hours before `now`.
fn draft(store: &OutboxStore, tool: &str, armed: bool, ago: i64, now: DateTime<Utc>) -> OutboxItem {
    let mut item = store
        .stage(
            tool,
            OutboxKind::Message,
            json!({"to": "idris.vale@example.org", "body": "The review is Thursday."}),
            Taint {
                private: armed,
                untrusted: armed,
            },
            Provenance {
                session_id: Some(format!("s-{}", uuid::Uuid::new_v4())),
                ..Provenance::default()
            },
        )
        .unwrap();
    item.created_at = (now - hours(ago)).to_rfc3339();
    item
}

/// `item`, resolved `after` hours after staging with `status`, by `by`.
fn resolved(mut item: OutboxItem, status: &str, after: i64, by: Actor) -> OutboxItem {
    let staged = at(&item.created_at).unwrap();
    item.status = status.into();
    item.resolved_at = Some((staged + hours(after)).to_rfc3339());
    item.resolved_by = Some(by);
    item
}

#[test]
fn the_owners_act_is_read_inside_the_window_and_no_act_past_it() {
    let dir = root("observe");
    let store = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    let window = hours(2 * DAY);
    let fresh = draft(&store, "mail_send", false, 1, now);
    assert_eq!(observe(&fresh, window, now), Observed::Pending);
    let stale = draft(&store, "mail_send", false, 3 * DAY, now);
    assert_eq!(
        observe(&stale, window, now),
        Observed::NoAct,
        "R37 carried to items"
    );

    let base = draft(&store, "mail_send", false, 3 * DAY, now);
    let sent = resolved(base.clone(), "sent", 1, Actor::Owner);
    assert_eq!(
        observe(&sent, window, now),
        Observed::Act(ExpectedAct::ReleasedUnchanged)
    );
    let rejected = resolved(base.clone(), "rejected", 1, Actor::Owner);
    assert_eq!(
        observe(&rejected, window, now),
        Observed::Act(ExpectedAct::Rejected)
    );
    let mut edited = resolved(base.clone(), "sent", 1, Actor::Owner);
    edited.args = json!({"to": "idris.vale@example.org", "body": "The review is Friday."});
    edited.edited_by = Some(Actor::Owner);
    assert_eq!(
        observe(&edited, window, now),
        Observed::Act(ExpectedAct::Edited)
    );
    // Not the owner's by the stamps: unknown, never the owner's act.
    let shells = resolved(base.clone(), "rejected", 1, Actor::OwnerApproved);
    assert_eq!(observe(&shells, window, now), Observed::Unknown);
    // An act after the window is not the act: the window closed on none.
    let late = resolved(base.clone(), "sent", 3 * DAY - 1, Actor::Owner);
    assert_eq!(observe(&late, window, now), Observed::NoAct);
    // Released, and the delivery failed: still `pending`, never "no act".
    let mut failed = base;
    failed.error = Some("smtp refused".into());
    assert_eq!(observe(&failed, window, now), Observed::Unknown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_base_rate_is_the_owners_most_frequent_act_on_similar_drafts_only() {
    let dir = root("base");
    let store = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    let window = hours(2 * DAY);
    let old = |tool: &str, armed: bool| draft(&store, tool, armed, 5 * DAY, now);
    let history = vec![
        resolved(old("mail_send", false), "rejected", 1, Actor::Owner),
        resolved(old("mail_send", false), "rejected", 1, Actor::Owner),
        resolved(old("mail_send", false), "sent", 1, Actor::Owner),
        // Other tool, other armed state, unstamped, not yet settled, and one
        // staged after the forecast: none of them count.
        resolved(old("mail_reply", false), "sent", 1, Actor::Owner),
        resolved(old("mail_send", true), "sent", 1, Actor::Owner),
        resolved(old("mail_send", false), "sent", 1, Actor::Unknown),
        draft(&store, "mail_send", false, 1, now),
        draft(&store, "mail_send", false, -1, now),
    ];
    assert_eq!(
        base_rate("mail_send", false, &history, window, now),
        (Some(ExpectedAct::Rejected), 3)
    );
    assert_eq!(
        base_rate("mail_send", true, &history, window, now),
        (Some(ExpectedAct::ReleasedUnchanged), 1)
    );
    assert_eq!(
        base_rate("calendar_hold", false, &history, window, now),
        (None, 0)
    );
    // A tie goes to the earlier act in `DRAFT_ACTS`, deterministically.
    let tied = vec![
        resolved(old("mail_send", false), "rejected", 1, Actor::Owner),
        resolved(old("mail_send", false), "sent", 1, Actor::Owner),
    ];
    assert_eq!(
        base_rate("mail_send", false, &tied, window, now),
        (Some(ExpectedAct::ReleasedUnchanged), 2)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store forecasts only when told to; the ledger is sealed away from
/// the item walk; a harness-authored card and a publish are not forecast.
#[test]
fn staging_forecasts_only_a_model_message_and_only_when_asked() {
    let dir = root("stage");
    let quiet = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    draft(&quiet, "mail_send", false, 0, now);
    assert!(load(&dir).unwrap().0.is_empty(), "off unless asked for");

    let store = OutboxStore::open(&dir)
        .unwrap()
        .with_forecasts(Window::Fixed(hours(2 * DAY)));
    let item = draft(&store, "mail_send", false, 0, now);
    store
        .stage_by_harness("mail_send", json!({"to": "a@example.org", "body": "b"}))
        .unwrap();
    store
        .stage(
            "bundle_publish",
            OutboxKind::Publish,
            json!({"path": "x"}),
            Taint::default(),
            Provenance::default(),
        )
        .unwrap();
    // The owner's own typed draft (`mecha mail send`: no staging session)
    // is not forecast, and is not history either.
    store
        .stage(
            "mail_send",
            OutboxKind::Message,
            json!({"to": "a@example.org", "body": "My own words."}),
            Taint::default(),
            Provenance::default(),
        )
        .unwrap();
    let (made, skipped) = load(&dir).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0].item_id, item.id);
    assert_eq!(
        (made[0].expected, made[0].basis),
        (None, 0),
        "no settled history: no basis, never a guess"
    );
    assert_eq!(store.items().unwrap().len(), 5, "the ledger is not an item");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `record` forecasts from settled history, and never from the draft
/// itself: its own staging time is `now`, which `base_rate` excludes, so a
/// forecast can never read its own outcome (review of #401).
#[test]
fn a_forecast_rests_on_settled_history_and_never_on_its_own_draft() {
    let dir = root("record");
    let store = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    let window = hours(2 * DAY);
    let mut history = vec![
        resolved(
            draft(&store, "mail_send", false, 4 * DAY, now),
            "rejected",
            1,
            Actor::Owner,
        ),
        resolved(
            draft(&store, "mail_send", false, 4 * DAY, now),
            "rejected",
            1,
            Actor::Owner,
        ),
    ];
    let mut fresh = draft(&store, "mail_send", false, 0, now);
    fresh.created_at = now.to_rfc3339();
    // The draft's own record, already settled as a release, is in the
    // history `record` is handed — as it is after `write_item`.
    let mut own = resolved(fresh.clone(), "sent", 0, Actor::Owner);
    own.created_at = fresh.created_at.clone();
    history.push(own);
    // The history is what this ledger forecast as real: before any line
    // exists, nothing counts.
    let bare = record(&dir, &fresh, Some(&history), Some(window), false)
        .unwrap()
        .unwrap();
    assert_eq!((bare.expected, bare.basis), (None, 0));
    for h in &history[..2] {
        record(&dir, h, Some(&[]), Some(window), false).unwrap();
    }
    // A smoke run's draft, rejected by the owner, is on the ledger as a
    // test and never history.
    let smoke = resolved(
        draft(&store, "mail_send", false, 4 * DAY, now),
        "sent",
        1,
        Actor::Owner,
    );
    record(&dir, &smoke, None, Some(window), true).unwrap();
    history.push(smoke);
    let f = record(&dir, &fresh, Some(&history), Some(window), false)
        .unwrap()
        .unwrap();
    assert!(!f.basis_unreadable);
    // An unreadable history is no basis, and says why.
    let blind = record(&dir, &fresh, None, Some(window), false)
        .unwrap()
        .unwrap();
    assert_eq!(
        (blind.expected, blind.basis, blind.basis_unreadable),
        (None, 0, true)
    );
    assert_eq!((f.expected, f.basis), (Some(ExpectedAct::Rejected), 2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_summary_scores_what_resolved_and_names_the_rest() {
    let dir = root("summary");
    let store = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    let window = hours(2 * DAY);
    let f = |item: &OutboxItem, expected: Option<ExpectedAct>| Forecast {
        item_id: item.id.clone(),
        session_id: item.session_id.clone(),
        at: at(&item.created_at).unwrap(),
        tool: item.tool.clone(),
        armed: false,
        expected,
        basis: 3,
        basis_unreadable: false,
        patience_secs: None,
        test: false,
        source: Source::BaseRate,
    };
    let hit = resolved(
        draft(&store, "mail_send", false, 4 * DAY, now),
        "rejected",
        1,
        Actor::Owner,
    );
    let miss = resolved(
        draft(&store, "mail_send", false, 4 * DAY, now),
        "sent",
        1,
        Actor::Owner,
    );
    let waiting = draft(&store, "mail_send", false, 1, now);
    let unowned = resolved(
        draft(&store, "mail_send", false, 4 * DAY, now),
        "sent",
        1,
        Actor::Unknown,
    );
    let basisless = draft(&store, "mail_send", false, 4 * DAY, now);
    let unforecast = draft(&store, "mail_send", false, 2, now);
    let made = vec![
        f(&hit, Some(ExpectedAct::Rejected)),
        f(&miss, Some(ExpectedAct::Rejected)),
        f(&waiting, Some(ExpectedAct::Rejected)),
        f(&unowned, Some(ExpectedAct::Rejected)),
        f(&basisless, None),
    ];
    let items = vec![hit, miss, waiting, unowned, basisless, unforecast];
    let mut skewed = made.clone();
    skewed.push(Forecast {
        expected: Some(ExpectedAct::Unknown),
        ..made[0].clone()
    });
    // A newer build's act word is unknown, never "no basis".
    let k = summarize(&skewed, 0, &items, true, Some(window), now);
    assert_eq!((k.no_basis, k.unknown), (1, 2));
    let s = summarize(&made, 0, &items, true, Some(window), now);
    assert_eq!((s.forecasts, s.scored, s.hits, s.surprises), (5, 2, 1, 1));
    assert_eq!(
        (s.pending, s.unknown, s.no_basis, s.unforecast),
        (1, 1, 1, 1)
    );
    assert_eq!(s.hit_rate, Some(0.5));
    // An unreadable charter scores nothing, and says it was the window —
    // never "not the owner's by the stamps".
    let blind = summarize(&made, 0, &items, true, None, now);
    assert_eq!(
        (
            blind.scored,
            blind.window_unreadable,
            blind.unknown,
            blind.hit_rate
        ),
        (0, 4, 0, None)
    );
    // The window a forecast was made under scores it, whatever today's
    // charter says: widening the window later does not turn a settled hit
    // back into "waiting" (review of #401).
    let untouched = draft(&store, "mail_send", false, 3 * DAY, now);
    let pinned = Forecast {
        patience_secs: Some(window.num_seconds()),
        ..f(&untouched, Some(ExpectedAct::NoAct))
    };
    let wider = Some(hours(7 * DAY));
    let r = summarize(
        std::slice::from_ref(&pinned),
        0,
        std::slice::from_ref(&untouched),
        true,
        wider,
        now,
    );
    assert_eq!((r.scored, r.hits, r.pending), (1, 1, 0));
    // A line from before the field falls back to today's window.
    let legacy = Forecast {
        patience_secs: None,
        ..pinned.clone()
    };
    let r = summarize(
        std::slice::from_ref(&legacy),
        0,
        std::slice::from_ref(&untouched),
        true,
        wider,
        now,
    );
    assert_eq!((r.scored, r.pending), (0, 1));
    // A partial outbox read: the drafts it did not see are unread, never
    // "not the owner's", and coverage is not claimed.
    let partial = summarize(&made, 0, &items[..2], false, Some(window), now);
    assert!(partial.outbox_unreadable);
    assert_eq!(
        (
            partial.scored,
            partial.drafts_unread,
            partial.unknown,
            partial.unforecast
        ),
        (2, 2, 0, 0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A surface's own store, opened like the shared one, forecasts as it does
/// — by construction, never by a copy each caller must remember; and the
/// staging's history read is bounded to the newest items (review of #401).
#[test]
fn a_surface_store_inherits_forecasting_and_history_is_bounded() {
    let dir = root("like");
    let shared = OutboxStore::open(&dir)
        .unwrap()
        .with_forecasts(Window::Fixed(hours(2 * DAY)));
    let mine = OutboxStore::open_like(&shared, &dir).unwrap();
    assert_eq!(mine.forecasting(), Some(Window::Fixed(hours(2 * DAY))));
    let plain = OutboxStore::open_like(&OutboxStore::open(&dir).unwrap(), &dir).unwrap();
    assert_eq!(plain.forecasting(), None);

    let now = Utc::now();
    for _ in 0..5 {
        draft(&mine, "mail_send", false, 0, now);
    }
    let (newest, skipped) = mine.items_newest(3).unwrap();
    assert_eq!((newest.len(), skipped), (3, 0));
    let all = mine.items().unwrap();
    let mut ids: Vec<_> = all.iter().map(|i| i.id.clone()).collect();
    ids.sort();
    let mut got: Vec<_> = newest.iter().map(|i| i.id.clone()).collect();
    got.sort();
    assert_eq!(got, ids[ids.len() - 3..].to_vec(), "the newest by id");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A smoke run's forecast line is set aside by the readout: not scored,
/// not a basis, and not an unforecast draft either.
#[test]
fn a_smoke_runs_forecast_is_set_aside() {
    let dir = root("smoke");
    let store = OutboxStore::open(&dir).unwrap();
    let now = Utc::now();
    let window = hours(2 * DAY);
    let item = resolved(
        draft(&store, "mail_send", false, 4 * DAY, now),
        "sent",
        1,
        Actor::Owner,
    );
    let f = record(&dir, &item, None, Some(window), true)
        .unwrap()
        .unwrap();
    assert!(f.test && f.expected.is_none());
    let s = summarize(
        &[f],
        0,
        std::slice::from_ref(&item),
        true,
        Some(window),
        now,
    );
    assert_eq!(
        (
            s.forecasts,
            s.tests_set_aside,
            s.scored,
            s.no_basis,
            s.unforecast
        ),
        (0, 1, 0, 0, 0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
