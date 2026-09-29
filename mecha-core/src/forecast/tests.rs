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
    let f = record(&dir, &fresh, &history, Some(window))
        .unwrap()
        .unwrap();
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
    let s = summarize(&made, 0, &items, Some(window), now);
    assert_eq!((s.forecasts, s.scored, s.hits, s.surprises), (5, 2, 1, 1));
    assert_eq!(
        (s.pending, s.unknown, s.no_basis, s.unforecast),
        (1, 1, 1, 1)
    );
    assert_eq!(s.hit_rate, Some(0.5));
    // An unreadable charter scores nothing, and says so.
    let blind = summarize(&made, 0, &items, None, now);
    assert_eq!((blind.scored, blind.unknown, blind.hit_rate), (0, 4, None));
    let _ = std::fs::remove_dir_all(&dir);
}
