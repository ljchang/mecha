//! `mecha doctor` — every store's distress, read in one pass.
//!
//! The incident this exists to end: a revoked OAuth token killed the
//! scheduling pipeline for three days, and every component recorded its
//! trouble correctly *in its own store* — an `auth_error.json` marker beside
//! the credentials, outbox items pending with a release error, frontdoor
//! requests parked in `awaiting_me`, trigger-ledger rows — while the operator
//! learned nothing, because nothing reads **across** the stores. Doctor is
//! that read.
//!
//! Two rules carry the design:
//!
//! - **Doctor is an observer, never load-bearing.** No network, no model, no
//!   tokens — and no writes: the stores are read directly rather than through
//!   the store constructors, because those create and re-chmod their
//!   directories on open, and an examination that heals the permissions it
//!   was about to report is measuring itself. Every check is individually
//!   best-effort: an unreadable or unparseable store is itself a finding
//!   ("store unreadable: <why>"), never a crash, and one check's failure
//!   never stops the others.
//! - **Fixes go through existing commands only.** A [`Remedy`] is an argv —
//!   `mecha-mail auth personal --provider google`, `mecha outbox review` —
//!   never a direct mutation of a store. In particular doctor never releases
//!   an outbox draft: the remedy for stuck drafts is opening the review
//!   surface, full stop.
//!
//! The checks are pure functions over injected store roots and an injected
//! `now`, which is what makes "a 49-hour-old pending draft" a unit test
//! instead of a two-day wait.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// How bad a finding is. Declared broken-first so the derived order is the
/// display order: what is broken outranks what merely wants attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Something is failing right now — a dead login, a release that errored.
    Broken,
    /// Nothing is failing, but something has sat unresolved long enough that
    /// silence is the more likely explanation than intent.
    Attention,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Broken => "broken",
            Severity::Attention => "attention",
        }
    }
}

/// A proposed fix: an existing command, never a store mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remedy {
    /// One line saying what running it does — and, where ordering matters,
    /// what to do first.
    pub description: String,
    /// The command as an argv, ready to spawn. Never empty.
    pub argv: Vec<String>,
    /// Whether the command needs the real terminal — an OAuth flow, an
    /// `$EDITOR` — and must therefore inherit stdin and the screen rather
    /// than being run with its output captured.
    pub needs_terminal: bool,
}

/// One observation: which component, how bad, what, and the way out if one
/// is known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub component: String,
    pub severity: Severity,
    pub summary: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remedy: Option<Remedy>,
}

impl Finding {
    /// The observer rule made a constructor: a store doctor cannot read is a
    /// finding about that store, never an error that stops the other checks.
    fn unreadable(component: &str, what: &str, why: impl std::fmt::Display) -> Finding {
        Finding {
            component: component.to_string(),
            severity: Severity::Attention,
            summary: format!("store unreadable: {what}"),
            detail: why.to_string(),
            remedy: None,
        }
    }
}

/// A pending draft older than this with no error has most likely been
/// forgotten rather than deliberately parked.
const STUCK_DRAFT_AFTER: chrono::Duration = chrono::Duration::hours(48);

/// A frontdoor request waiting on the user for longer than this is the
/// stranger-facing silence the front door exists to prevent.
const STALE_REQUEST_AFTER: chrono::Duration = chrono::Duration::hours(72);

/// Examine every store under `home` and report what is wrong.
///
/// `now` is injected for testability; nothing here consults the clock.
/// Best-effort throughout: each check appends what it found, a failed check
/// appends a finding about the failure, and no check can stop another.
pub fn examine(home: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(check_mail(&home.join("mail")));
    findings.extend(check_legacy_mail(home));
    findings.extend(check_outbox(&home.join("outbox"), now));
    findings.extend(check_frontdoor(&home.join("requests"), now));
    findings.extend(check_triggers(&home.join("triggers"), now));
    // The graph store is `~/.mecha-graph`, a hidden sibling of the mecha home
    // by that store's own convention — resolved relative to `home` so a test
    // (or a relocated home) carries its sibling with it.
    if let Some(parent) = home.parent() {
        findings.extend(check_graph_nightly(&parent.join(".mecha-graph"), now));
    }
    sort(&mut findings);
    findings
}

/// Severity first, then component, then insertion order — the shape both the
/// renderer and the JSON output present.
pub fn sort(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.component.cmp(&b.component))
    });
}

// --- dead mail auth ---------------------------------------------------------

/// `auth_error.json`, structurally. The writer is `mecha-mail`'s token
/// lifecycle; the seam is a file of JSON exactly like the frontdoor's
/// directory-of-JSON, which is why core takes no `mecha-mail` dependency to
/// read it.
#[derive(Debug, Deserialize)]
struct AuthMarker {
    at: String,
    message: String,
}

/// `accounts.toml`, structurally, for the same reason. Only the fields doctor
/// needs; unknown ones are ignored.
#[derive(Debug, Default, Deserialize)]
struct MailAccounts {
    #[serde(default, rename = "account")]
    accounts: Vec<MailAccount>,
}

#[derive(Debug, Deserialize)]
struct MailAccount {
    name: String,
    provider: String,
    /// Declared lifetime of the refresh credential, in days. See
    /// `mecha_mail::accounts::AccountEntry::grant_lifetime_days` — absent
    /// means no known expiry, and no warning.
    #[serde(default)]
    grant_lifetime_days: Option<u32>,
}

/// Scan `<mail>/*/auth_error.json`. Presence means a *permanent* refresh
/// failure — the marker is written on `invalid_grant` and cleared by the next
/// successful credential save — so a marker is Broken, not a maybe.
fn check_mail(mail: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    if !mail.is_dir() {
        return out;
    }

    // Provider per account, best-effort: an unparseable registry costs the
    // `--provider` flag on the remedy, never the finding itself.
    let declared: Vec<MailAccount> = std::fs::read_to_string(mail.join("accounts.toml"))
        .ok()
        .and_then(|text| toml::from_str::<MailAccounts>(&text).ok())
        .map(|file| file.accounts)
        .unwrap_or_default();
    let providers: BTreeMap<String, String> = declared
        .iter()
        .map(|a| (a.name.clone(), a.provider.clone()))
        .collect();
    let lifetimes: BTreeMap<String, u32> = declared
        .iter()
        .filter_map(|a| a.grant_lifetime_days.map(|d| (a.name.clone(), d)))
        .collect();

    let entries = match std::fs::read_dir(mail) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "mail",
                "the mail directory",
                format!("{}: {e}", mail.display()),
            ));
            return out;
        }
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(account) = dir.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        // A grant that predates the triage scopes refreshes cleanly forever
        // and then 403s the first time something archives. That failure has
        // no marker — nothing has gone wrong yet — so it is read off the
        // credential file directly, structurally, like everything else here.
        out.extend(check_triage_scope(&dir, &account, providers.get(&account)));
        out.extend(check_grant_age(
            &dir,
            &account,
            providers.get(&account),
            lifetimes.get(&account).copied(),
        ));

        let marker_path = dir.join("auth_error.json");
        if !marker_path.is_file() {
            continue;
        }
        let text = match std::fs::read_to_string(&marker_path) {
            Ok(text) => text,
            Err(e) => {
                out.push(Finding::unreadable(
                    "mail",
                    &format!("auth_error.json for `{account}`"),
                    format!("{}: {e}", marker_path.display()),
                ));
                continue;
            }
        };
        match serde_json::from_str::<AuthMarker>(&text) {
            Ok(marker) => {
                let provider = providers.get(&account);
                let mut argv = vec![
                    "mecha-mail".to_string(),
                    "auth".to_string(),
                    account.clone(),
                ];
                if let Some(provider) = provider {
                    argv.push("--provider".to_string());
                    argv.push(provider.clone());
                }
                out.push(Finding {
                    component: "mail".to_string(),
                    severity: Severity::Broken,
                    summary: format!("mail auth for `{account}` is dead"),
                    // The marker's message already names the exact re-auth
                    // command, so it rides in the detail — which also covers
                    // the case where accounts.toml could not say which
                    // provider the remedy needs.
                    detail: format!(
                        "permanent refresh failure since {}: {}",
                        marker.at, marker.message
                    ),
                    remedy: Some(Remedy {
                        description: format!(
                            "re-authenticate the `{account}` account (opens an OAuth flow)"
                        ),
                        argv,
                        needs_terminal: true,
                    }),
                });
            }
            Err(e) => out.push(Finding::unreadable(
                "mail",
                &format!("auth_error.json for `{account}` did not parse"),
                format!("{}: {e}", marker_path.display()),
            )),
        }
    }
    out
}

/// The scope a grant was minted with, as `mecha-mail` records it.
///
/// Read structurally rather than through `mecha-mail`'s type, for the reason
/// the whole module gives: doctor takes no dependency on the crates it
/// examines, and a field it does not know about must not stop it reading the
/// one it does.
#[derive(Debug, serde::Deserialize)]
struct StoredGrant {
    #[serde(default)]
    granted_scopes: Option<String>,
    #[serde(default)]
    granted_at: Option<String>,
}

/// How many days before a grant expires to start saying so.
///
/// Two, because the remedy is a two-minute re-auth that needs a human at a
/// terminal — long enough to survive a weekend-adjacent lapse, short enough
/// that it is not background noise on a 7-day cycle. A warning that fires
/// for most of the grant's life is a warning nobody reads.
const GRANT_WARN_WITHIN_DAYS: i64 = 2;

/// Warn before a grant with a known, fixed lifetime expires.
///
/// This exists because of the 2026-08-11 outage: Google expires the refresh
/// token of an app in *Testing* publishing status exactly 7 days after
/// consent, returns `invalid_grant` when it does — indistinguishable from a
/// revocation — and scheduling went down for three days. Doctor reported it
/// correctly *after* the fact. A recurring, dated failure deserves to be
/// reported before it happens, which is the one thing a marker written on
/// failure can never do.
///
/// Silent unless the lifetime was declared: see
/// `AccountEntry::grant_lifetime_days` for why this is not inferred.
fn check_grant_age(
    dir: &Path,
    account: &str,
    provider: Option<&String>,
    lifetime_days: Option<u32>,
) -> Vec<Finding> {
    let Some(lifetime) = lifetime_days.filter(|d| *d > 0) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(dir.join("oauth.json")) else {
        return Vec::new();
    };
    let Ok(grant) = serde_json::from_str::<StoredGrant>(&text) else {
        return Vec::new(); // already reported by the scope check
    };
    // An un-stamped grant predates the field. Its age is genuinely unknown,
    // and inventing one would either cry wolf or promise safety — so say
    // nothing and let the next re-auth start the clock honestly.
    let Some(granted_at) = grant.granted_at.as_deref() else {
        return Vec::new();
    };
    let Ok(granted) = chrono::DateTime::parse_from_rfc3339(granted_at) else {
        return Vec::new();
    };
    let expires = granted.with_timezone(&chrono::Utc) + chrono::Duration::days(lifetime as i64);
    // Hours, then round *up* to whole days. `num_days()` truncates toward
    // zero, so a grant with 47 hours left reports "1 day" — which is both
    // wrong and the wrong direction, since it makes the warning look more
    // urgent than it is and then says "1 day" again tomorrow.
    let hours_left = (expires - chrono::Utc::now()).num_hours();
    let left = (hours_left as f64 / 24.0).ceil() as i64;
    if left > GRANT_WARN_WITHIN_DAYS {
        return Vec::new();
    }
    let when = if hours_left < 0 {
        "has expired".to_string()
    } else if hours_left < 24 {
        "expires within a day".to_string()
    } else {
        format!("expires in {left} days")
    };
    let mut argv = vec![
        "mecha-mail".to_string(),
        "auth".to_string(),
        account.to_string(),
    ];
    if let Some(p) = provider {
        argv.push("--provider".to_string());
        argv.push(p.clone());
    }
    vec![Finding {
        component: "mail".to_string(),
        severity: Severity::Attention,
        summary: format!("`{account}` sign-in {when}"),
        detail: format!(
            "this grant lasts {lifetime} days from consent ({granted_at}) and refreshing does \
             not extend it. Re-authenticate before it lapses — once it does, the failure looks \
             like a revoked token and every scheduled run using this account stops."
        ),
        remedy: Some(Remedy {
            description: format!("re-authenticate `{account}` now (opens an OAuth flow)"),
            argv,
            needs_terminal: true,
        }),
    }]
}

/// Which scope each provider needs before the triage verbs work. Mirrors
/// `mecha_mail::token::triage_scope_for`; duplicated rather than imported
/// because the seam here is a directory of JSON, not a crate dependency.
fn triage_scope_for(provider: &str) -> Option<&'static str> {
    match provider {
        "google" => Some("gmail.modify"),
        "outlook" | "microsoft" => Some("Mail.ReadWrite"),
        _ => None,
    }
}

/// Report an account whose OAuth grant does not cover archive/spam/read-state.
///
/// Only reported when the provider is known: guessing which scope a grant
/// should carry would turn an unrecognised provider into a permanent false
/// finding, and a doctor that cries wolf stops being read. An **absent**
/// `granted_scopes` counts as not covered, which is correct rather than
/// harsh — every grant written before the field existed predates the scopes
/// too.
///
/// `Attention`, not `Broken`: nothing is failing right now — mail reads,
/// sends and stages drafts exactly as before — but the first archive will
/// fail, and that is precisely the "silence is the likely explanation"
/// shape this severity is for. On a managed Microsoft tenant the remedy may
/// also need an administrator rather than the user, so the detail says so
/// instead of implying a re-auth alone will fix it.
fn check_triage_scope(dir: &Path, account: &str, provider: Option<&String>) -> Vec<Finding> {
    let Some(provider) = provider else {
        return Vec::new();
    };
    let Some(needed) = triage_scope_for(provider) else {
        return Vec::new();
    };
    let path = dir.join("oauth.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        // No credentials is not a scope problem; the account simply is not
        // signed in, which other checks and the first real call will say.
        return Vec::new();
    };
    let Ok(grant) = serde_json::from_str::<StoredGrant>(&text) else {
        return vec![Finding::unreadable(
            "mail",
            &format!("oauth.json for `{account}` did not parse"),
            format!("{}", path.display()),
        )];
    };
    if grant
        .granted_scopes
        .as_deref()
        .is_some_and(|g| g.contains(needed))
    {
        return Vec::new();
    }
    let admin_note = if provider == "outlook" || provider == "microsoft" {
        " Microsoft blocks `Mail.ReadWrite` from end-user consent under its \
         recommended policy, so on a managed tenant an administrator has to \
         grant it to the app registration before this can succeed."
    } else {
        ""
    };
    vec![Finding {
        component: "mail".to_string(),
        severity: Severity::Attention,
        summary: format!("`{account}` cannot archive, spam or mark mail read"),
        detail: format!(
            "the stored grant does not include `{needed}`, so mail_triage will fail on this \
             account. Reading, sending and calendar work are unaffected.{admin_note}"
        ),
        remedy: Some(Remedy {
            description: format!(
                "re-authenticate `{account}` to add the triage scope (opens an OAuth flow)"
            ),
            argv: vec![
                "mecha-mail".to_string(),
                "auth".to_string(),
                account.to_string(),
                "--provider".to_string(),
                provider.clone(),
            ],
            needs_terminal: true,
        }),
    }]
}

#[cfg(test)]
mod grant_age_tests {
    use super::*;

    fn store(dir: &Path, granted_at: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        let stamp = granted_at
            .map(|g| format!(r#","granted_at":"{g}""#))
            .unwrap_or_default();
        std::fs::write(
            dir.join("oauth.json"),
            format!(r#"{{"client_id":"i","access_token":"a","refresh_token":"r","expires_at":1{stamp}}}"#),
        )
        .unwrap();
    }

    fn days_ago(n: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(n)).to_rfc3339()
    }

    /// The 7-day Testing clock, reported before it fires rather than after.
    #[test]
    fn a_grant_nearing_its_declared_lifetime_is_reported_early() {
        let tmp = std::env::temp_dir().join(format!("mecha-grant-{}", std::process::id()));
        let g = "google".to_string();

        // Fresh: silent. A warning that fires all week is not a warning.
        store(&tmp, Some(&days_ago(1)));
        assert!(check_grant_age(&tmp, "personal", Some(&g), Some(7)).is_empty());

        // Day 5 of 7 — two days left, inside the window.
        store(&tmp, Some(&days_ago(5)));
        let f = check_grant_age(&tmp, "personal", Some(&g), Some(7));
        assert_eq!(f.len(), 1, "should warn with 2 days left");
        assert!(
            f[0].summary.contains("expires in 2 days"),
            "{}",
            f[0].summary
        );
        assert!(f[0].remedy.as_ref().unwrap().needs_terminal);

        // Under 24h: worded without a misleading whole-day count.
        store(&tmp, Some(&days_ago(7)));
        let f = check_grant_age(&tmp, "personal", Some(&g), Some(7));
        assert!(f[0].summary.contains("within a day"), "{}", f[0].summary);

        // Past it: still a finding, worded as past.
        store(&tmp, Some(&days_ago(9)));
        let f = check_grant_age(&tmp, "personal", Some(&g), Some(7));
        assert!(f[0].summary.contains("has expired"), "{}", f[0].summary);

        // No declared lifetime: silent however old. Not inferred, ever.
        assert!(check_grant_age(&tmp, "personal", Some(&g), None).is_empty());

        // Un-stamped grant: age unknown, so no claim either way.
        store(&tmp, None);
        assert!(check_grant_age(&tmp, "personal", Some(&g), Some(7)).is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }
}

/// The legacy per-provider stores — `<home>/google/oauth.json` and
/// `<home>/outlook/oauth.json`, still served by the shipped `mecha-google`
/// and `mecha-outlook` binaries and what `mecha-mail import` exists to
/// migrate — get the same marker written beside their credentials by the
/// same token lifecycle. A doctor that reads only the registry layout
/// reports "all clear" over a dead legacy login.
fn check_legacy_mail(home: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for provider in ["google", "outlook"] {
        let marker_path = home.join(provider).join("auth_error.json");
        if !marker_path.is_file() {
            continue;
        }
        let text = match std::fs::read_to_string(&marker_path) {
            Ok(text) => text,
            Err(e) => {
                out.push(Finding::unreadable(
                    "mail",
                    &format!("auth_error.json for the legacy {provider} store"),
                    format!("{}: {e}", marker_path.display()),
                ));
                continue;
            }
        };
        match serde_json::from_str::<AuthMarker>(&text) {
            Ok(marker) => out.push(Finding {
                component: "mail".to_string(),
                severity: Severity::Broken,
                summary: format!("legacy {provider} mail auth is dead"),
                // The marker's message names the exact re-auth command (the
                // writer derives it from the store's directory), so it rides
                // in the detail.
                detail: format!(
                    "permanent refresh failure since {}: {}",
                    marker.at, marker.message
                ),
                remedy: Some(Remedy {
                    description: format!(
                        "bring the legacy {provider} login into the unified registry — \
                         and re-authenticate it per the detail, which no import fixes"
                    ),
                    argv: vec![
                        "mecha-mail".to_string(),
                        "import".to_string(),
                        provider.to_string(),
                        "--provider".to_string(),
                        provider.to_string(),
                    ],
                    needs_terminal: false,
                }),
            }),
            Err(e) => out.push(Finding::unreadable(
                "mail",
                &format!("auth_error.json for the legacy {provider} store did not parse"),
                format!("{}: {e}", marker_path.display()),
            )),
        }
    }
    out
}

// --- stuck outbox items -----------------------------------------------------

/// Read the outbox items directly — one JSON file per item, the store's own
/// on-disk contract — so that examining the store never creates or re-chmods
/// it the way [`crate::outbox::OutboxStore::open`] deliberately does.
fn check_outbox(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "outbox",
                "the outbox directory",
                format!("{}: {e}", root.display()),
            ));
            return out;
        }
    };

    let review = Remedy {
        description: "open the outbox review surface — doctor never releases a draft".to_string(),
        argv: vec!["mecha".into(), "outbox".into(), "review".into()],
        needs_terminal: true,
    };

    let mut stale: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let item: crate::outbox::OutboxItem =
            match std::fs::read_to_string(&path).map(|t| serde_json::from_str(&t)) {
                Ok(Ok(item)) => item,
                Ok(Err(e)) => {
                    out.push(Finding::unreadable(
                        "outbox",
                        &format!(
                            "item {} did not parse",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
                Err(e) => {
                    out.push(Finding::unreadable(
                        "outbox",
                        &format!(
                            "item {} could not be read",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
            };
        if item.status != "pending" {
            continue;
        }
        if let Some(error) = &item.error {
            out.push(Finding {
                component: "outbox".to_string(),
                severity: Severity::Broken,
                summary: format!("release failed: {error}"),
                detail: format!(
                    "{} · {} — still pending; the draft is good, the delivery was not",
                    item.id, item.summary
                ),
                remedy: Some(review.clone()),
            });
        } else if age_of(&item.created_at, now).is_some_and(|age| age > STUCK_DRAFT_AFTER) {
            stale.push(format!(
                "{} · {} — staged {}",
                item.id,
                item.summary,
                render_age(now, &item.created_at)
            ));
        }
    }

    if !stale.is_empty() {
        // read_dir order is arbitrary; ids sort by staging time.
        stale.sort();
        out.push(Finding {
            component: "outbox".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} draft{} pending for more than 48h",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" }
            ),
            detail: stale.join("\n"),
            remedy: Some(review),
        });
    }
    out
}

// --- frontdoor --------------------------------------------------------------

/// The states that mean a request is waiting on the user rather than on the
/// requester: `extracted` awaits triage, `awaiting_me` awaits a draft review,
/// and `triaged` is triage's "I drafted nothing — this needs a person":
/// nothing ever re-triages it, so left alone it waits forever, invisibly.
/// (`needs_info` waits on the stranger, and `drained` on the extraction pass.)
const WAITING_ON_ME: [&str; 3] = [
    crate::frontdoor::EXTRACTED,
    crate::frontdoor::AWAITING_ME,
    crate::frontdoor::TRIAGED,
];

/// Read the request records directly, for the same no-side-effects reason as
/// the outbox — [`crate::frontdoor::Frontdoor::open`] creates the directory.
fn check_frontdoor(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "frontdoor",
                "the request store",
                format!("{}: {e}", root.display()),
            ));
            return out;
        }
    };

    let list = Remedy {
        description: "list the frontdoor queue".to_string(),
        argv: vec!["mecha".into(), "frontdoor".into(), "list".into()],
        needs_terminal: false,
    };

    let mut stale: Vec<(i64, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(Ok(record)) = std::fs::read_to_string(&path)
            .map(|t| serde_json::from_str::<crate::frontdoor::Record>(&t))
        else {
            // The frontdoor store itself skips unreadable records; doctor
            // says so instead, because silent skipping is the disease here.
            out.push(Finding::unreadable(
                "frontdoor",
                &format!(
                    "request {} did not parse",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                path.display().to_string(),
            ));
            continue;
        };
        if record.state == crate::frontdoor::EXTRACTION_FAILED {
            out.push(Finding {
                component: "frontdoor".to_string(),
                severity: Severity::Broken,
                summary: format!(
                    "request {} failed extraction and waits for a human",
                    record.seq
                ),
                detail: format!(
                    "{} ({}) — {}",
                    record.seq,
                    record.type_id,
                    record
                        .extraction_error
                        .as_deref()
                        .unwrap_or("no error recorded")
                ),
                remedy: Some(list.clone()),
            });
        } else if WAITING_ON_ME.contains(&record.state.as_str())
            && request_age(&record, now).is_some_and(|age| age > STALE_REQUEST_AFTER)
        {
            stale.push((
                record.seq,
                format!(
                    "{} ({}) — {}, received {}",
                    record.seq,
                    record.type_id,
                    record.state,
                    render_age(now, &record.created_at)
                ),
            ));
        }
    }

    if !stale.is_empty() {
        // read_dir order is arbitrary; the queue reads oldest-first by seq.
        stale.sort_by_key(|(seq, _)| *seq);
        out.push(Finding {
            component: "frontdoor".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} request{} waiting on you for more than 72h",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" }
            ),
            detail: stale
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n"),
            remedy: Some(list),
        });
    }
    out
}

/// How long a request has waited: from when it arrived here (`drained_at`),
/// falling back to when the stranger sent it. Unparseable stamps mean the age
/// is unknown, and unknown never counts as stale — a doctor that guesses is
/// worse than one that says nothing.
fn request_age(record: &crate::frontdoor::Record, now: DateTime<Utc>) -> Option<chrono::Duration> {
    age_of(&record.drained_at, now).or_else(|| age_of(&record.created_at, now))
}

// --- trigger health ---------------------------------------------------------

/// How many recent runs the reliability check averages over.
///
/// Five, because one bad morning is not a trend and a long window would hide a
/// trigger that broke this week behind a month of health.
const HEALTH_WINDOW: usize = 5;

/// Below this many calls in the window, no rate is reported.
///
/// A rate over three calls is noise, and a doctor that cries wolf stops being
/// read — the same reasoning as the scope check declining to guess.
const HEALTH_MIN_CALLS: u32 = 10;

/// The share of failed calls that is worth a human's attention.
///
/// A third. Deliberately not near-zero: a model that tries a path, is told it
/// does not exist, and tries the right one has done nothing wrong, and errors
/// are how a run learns about its environment. What this is looking for is a
/// trigger whose environment has moved out from under it.
const HEALTH_ERROR_RATE: f64 = 1.0 / 3.0;

/// Read the trigger files and the ledger directly — same reason as above:
/// [`crate::trigger::TriggerStore::open`] creates and re-chmods the root.
fn check_triggers(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "triggers",
                "the trigger store",
                format!("{}: {e}", root.display()),
            ));
            return out;
        }
    };

    let mut triggers: Vec<crate::trigger::Trigger> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        match std::fs::read_to_string(&path).map(|t| toml::from_str::<crate::trigger::Trigger>(&t))
        {
            Ok(Ok(mut trigger)) => {
                trigger.name = name;
                triggers.push(trigger);
            }
            _ => out.push(Finding::unreadable(
                "triggers",
                &format!("trigger file `{name}.toml` did not parse"),
                path.display().to_string(),
            )),
        }
    }

    // One ledger scan for both questions: the newest row that actually *ran*
    // per trigger, and the newest *accounted slot* per trigger (manual runs
    // carry no slot, so they are invisible to the schedule on purpose).
    let mut recent: BTreeMap<String, Vec<crate::trigger::RunRecord>> = BTreeMap::new();
    let mut last_slot: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
    let ledger = root.join("runs.jsonl");
    if ledger.is_file() {
        match std::fs::read_to_string(&ledger) {
            Ok(text) => {
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    // A torn line is the store's problem, not this row's
                    // neighbours': skip it the way the ledger reader does.
                    let Ok(row) = serde_json::from_str::<crate::trigger::RunRecord>(line) else {
                        continue;
                    };
                    if let Some(slot) = row.slot {
                        let newest = last_slot.entry(row.trigger.clone()).or_insert(slot);
                        if slot > *newest {
                            *newest = slot;
                        }
                    }
                    // A skip is a row, not a run: a skipped-stale or
                    // skipped-overlap appended after an error is bookkeeping,
                    // not a recovery, and keying on the literal last row let
                    // it hide the failure the operator needed to see.
                    if matches!(
                        row.status,
                        crate::trigger::RunStatus::Ok | crate::trigger::RunStatus::Error
                    ) {
                        let window = recent.entry(row.trigger.clone()).or_default();
                        window.push(row);
                        if window.len() > HEALTH_WINDOW {
                            window.remove(0);
                        }
                    }
                }
            }
            Err(e) => out.push(Finding::unreadable(
                "triggers",
                "the run ledger",
                format!("{}: {e}", ledger.display()),
            )),
        }
    }

    for trigger in &triggers {
        if !trigger.enabled {
            continue;
        }

        // The most recent run failed: a manual run is the safe probe, because
        // it records a row with no slot and so never advances the schedule.
        let window = recent.get(&trigger.name);
        if let Some(row) = window.and_then(|w| w.last()) {
            if row.status == crate::trigger::RunStatus::Error {
                out.push(Finding {
                    component: "triggers".to_string(),
                    severity: Severity::Attention,
                    summary: format!("trigger `{}`'s most recent run failed", trigger.name),
                    detail: format!(
                        "started {}: {}",
                        row.started_at.to_rfc3339(),
                        row.error.as_deref().unwrap_or("no error recorded")
                    ),
                    remedy: Some(Remedy {
                        description: format!(
                            "run `{}` by hand — a manual run is evidence, not a fire; it never advances the schedule",
                            trigger.name
                        ),
                        argv: vec![
                            "mecha".into(),
                            "trigger".into(),
                            "run".into(),
                            trigger.name.clone(),
                        ],
                        needs_terminal: false,
                    }),
                });
            }
        }

        // Reliability across the window. An unattended run has nobody
        // watching it fail: the briefing still arrives, the ledger still says
        // `ok`, and a trigger failing a third of its calls looks exactly like
        // one that works. Silent below a floor of calls, because a rate over
        // three of them is noise, and unknown is never a finding.
        let (calls, errors) = window
            .map(|w| {
                w.iter().fold((0u32, 0u32), |(c, e), r| {
                    (c + r.tool_calls, e + r.tool_errors)
                })
            })
            .unwrap_or((0, 0));
        if calls >= HEALTH_MIN_CALLS && f64::from(errors) / f64::from(calls) >= HEALTH_ERROR_RATE {
            let runs = window.map(Vec::len).unwrap_or(0);
            out.push(Finding {
                component: "triggers".to_string(),
                severity: Severity::Attention,
                summary: format!(
                    "trigger `{}` failed {errors} of {calls} tool calls",
                    trigger.name
                ),
                detail: format!(
                    "across its last {runs} run(s){}. A run's answer arrives either way, so                      this is invisible in the ledger's status — and per-step reliability is                      what decides how long a task the run can finish, so a third of the                      calls failing is not a third of the work lost.",
                    if window.is_some_and(|w| w.last().is_some_and(|r| r.ended_on_failed_call)) {
                        ", and the most recent run answered with its last call failed"
                    } else {
                        ""
                    }
                ),
                // Reading is the remedy: what to change is in the transcript,
                // and doctor never decides that.
                remedy: Some(Remedy {
                    description: format!("read `{}`'s recent runs", trigger.name),
                    argv: vec![
                        "mecha".into(),
                        "trigger".into(),
                        "show".into(),
                        trigger.name.clone(),
                    ],
                    needs_terminal: false,
                }),
            });
        }

        // A catch-up-always trigger whose accounted slots stopped advancing:
        // a healthy daemon fires the most recent slot every tick, so more
        // than two slots newer than the last accounted one means nothing is
        // ticking at all. Cheap by construction — three `prev_at_or_before`
        // calls, no schedule re-derivation.
        if trigger.catch_up != crate::trigger::CatchUp::Always {
            continue;
        }
        let Some(anchor) = last_slot.get(&trigger.name).copied().or(trigger.created_at) else {
            // No ledger row and no creation stamp: there is no baseline to
            // measure staleness against, and unknown is not stale.
            continue;
        };
        let tz = trigger.tz(None);
        let step = chrono::Duration::seconds(1);
        let missed_more_than_two = trigger
            .schedule
            .prev_at_or_before(now, tz)
            .and_then(|s0| trigger.schedule.prev_at_or_before(s0 - step, tz))
            .and_then(|s1| trigger.schedule.prev_at_or_before(s1 - step, tz))
            .is_some_and(|s2| s2 > anchor);
        if missed_more_than_two {
            out.push(Finding {
                component: "triggers".to_string(),
                severity: Severity::Attention,
                summary: format!("trigger `{}` has missed more than two slots", trigger.name),
                detail: format!(
                    "last accounted slot {}; with catch_up=always a healthy scheduler fires \
                     the most recent slot every tick, so the daemon or its timer may be down \
                     (systemctl --user status mecha-triggers)",
                    anchor.to_rfc3339()
                ),
                // No argv on purpose: running the trigger by hand would not
                // restart whatever stopped ticking.
                remedy: None,
            });
        }
    }
    out
}

// --- graph nightly silence --------------------------------------------------

/// The two daily jobs that keep the knowledge graph current, each of which
/// writes `<prefix>YYYYMMDD.log` on *every* run — a deferred night says so in
/// the log — so a day with no file means the script never started. That is
/// exactly the failure cron cannot report: no MTA, and the script's own
/// logging begins after the point where an exec failure kills it (measured
/// 2026-08-17, when a missing execute bit cost a night of vet and gossip and
/// nothing anywhere said so).
const GRAPH_NIGHTLIES: &[(&str, &str)] = &[
    ("nightly-", "the graph's own sweep (ingest, extract, decay)"),
    ("mecha-nightly-", "the mecha half (vet, precheck, gossip)"),
];

/// Scan `<graph store>/logs` for each nightly family's newest dated log.
///
/// Quiet when the store, the logs directory, or a family has never existed —
/// absence is "not installed", which is not a finding. The bar is "newer than
/// the day before yesterday": today's file legitimately does not exist before
/// that job's cron slot, so yesterday's is the newest a healthy quiet morning
/// can show.
fn check_graph_nightly(store: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    let logs = store.join("logs");
    if !logs.is_dir() {
        return out;
    }
    let names: Vec<String> = match std::fs::read_dir(&logs) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .collect(),
        Err(e) => {
            out.push(Finding::unreadable(
                "graph",
                "the graph nightly logs",
                format!("{}: {e}", logs.display()),
            ));
            return out;
        }
    };

    for (prefix, what) in GRAPH_NIGHTLIES {
        let newest = names
            .iter()
            .filter_map(|n| {
                n.strip_prefix(prefix)?
                    .strip_suffix(".log")
                    .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y%m%d").ok())
            })
            .max();
        // Never ran at all: indistinguishable from "this half is not set up",
        // and a doctor that guesses teaches people to ignore it.
        let Some(newest) = newest else { continue };
        let days_quiet = (now.date_naive() - newest).num_days();
        if days_quiet > 1 {
            out.push(Finding {
                component: "graph".to_string(),
                severity: Severity::Attention,
                summary: format!(
                    "the graph nightly ({}) has not run for {days_quiet} days",
                    prefix.trim_end_matches('-'),
                ),
                detail: format!(
                    "{what} last wrote {}{}.log under {}; it logs every \
                     run including deferred ones, so a missing day means the \
                     script never started — cron reports that nowhere",
                    prefix,
                    newest.format("%Y%m%d"),
                    logs.display(),
                ),
                remedy: Some(Remedy {
                    description: "list the cron entries that fire the graph nightlies, \
                                  then run the silent one by hand and read its error"
                        .to_string(),
                    argv: vec!["crontab".into(), "-l".into()],
                    needs_terminal: false,
                }),
            });
        }
    }
    out
}

// --- shared helpers ---------------------------------------------------------

/// The age of an RFC 3339 stamp, or `None` when it does not parse — unknown
/// must never masquerade as old (or as fresh).
fn age_of(stamp: &str, now: DateTime<Utc>) -> Option<chrono::Duration> {
    DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|at| now - at.with_timezone(&Utc))
}

/// "49h ago", "3d ago", or the raw stamp when it does not parse.
fn render_age(now: DateTime<Utc>, stamp: &str) -> String {
    match age_of(stamp, now) {
        Some(age) if age >= chrono::Duration::days(2) => format!("{}d ago", age.num_days()),
        Some(age) if age >= chrono::Duration::hours(1) => format!("{}h ago", age.num_hours()),
        Some(age) => format!("{}m ago", age.num_minutes().max(0)),
        None => stamp.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use crate::outbox::{OutboxItem, OutboxKind};
    use serde_json::json;
    use std::path::PathBuf;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const NOW: &str = "2026-08-14T12:00:00Z";

    /// A scratch mecha home, unique per test and thread.
    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-doctor-test-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_marker(home: &Path, account: &str, body: &str) {
        let dir = home.join("mail").join(account);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("auth_error.json"), body).unwrap();
    }

    fn valid_marker() -> String {
        json!({
            "at": "2026-08-11T09:00:00Z",
            "message": "the refresh token was revoked — run `mecha-mail auth personal --provider google` to sign in again",
        })
        .to_string()
    }

    fn pending_item(home: &Path, id: &str, created_at: &str, error: Option<&str>) {
        let item = OutboxItem {
            id: id.to_string(),
            status: "pending".into(),
            tool: "mail__send".into(),
            kind: OutboxKind::Message,
            args_before: json!({"to": "a@x.org"}),
            args: json!({"to": "a@x.org"}),
            summary: "mail__send to a@x.org".into(),
            session_id: None,
            workspace: None,
            taint: Taint::default(),
            created_at: created_at.to_string(),
            resolved_at: None,
            reason: None,
            error: error.map(String::from),
        };
        let dir = home.join("outbox");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(&item).unwrap(),
        )
        .unwrap();
    }

    fn request(home: &Path, seq: i64, state: &str, drained_at: &str) {
        let dir = home.join("requests");
        std::fs::create_dir_all(&dir).unwrap();
        let record = json!({
            "seq": seq,
            "type_id": "meeting",
            "state": state,
            "created_at": drained_at,
            "drained_at": drained_at,
            "valid": true,
            "values": {},
            "free_text": [],
        });
        std::fs::write(
            dir.join(format!("{seq:010}-meeting.json")),
            record.to_string(),
        )
        .unwrap();
    }

    fn trigger_file(home: &Path, name: &str, extra: &str) {
        let dir = home.join("triggers");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}.toml")),
            format!(
                "schedule = \"0 7 * * *\"\nprompt = \"brief me\"\ntimezone = \"UTC\"\n\
                 created_at = \"2026-08-01T00:00:00Z\"\n{extra}"
            ),
        )
        .unwrap();
    }

    fn ledger_row(home: &Path, row: &serde_json::Value) {
        use std::io::Write;
        let dir = home.join("triggers");
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("runs.jsonl"))
            .unwrap();
        writeln!(file, "{row}").unwrap();
    }

    fn of<'a>(findings: &'a [Finding], component: &str) -> Vec<&'a Finding> {
        findings
            .iter()
            .filter(|f| f.component == component)
            .collect()
    }

    #[test]
    fn a_dead_auth_marker_is_found_and_an_absent_one_is_not() {
        let home = home("dead-auth");
        write_marker(&home, "personal", &valid_marker());
        // A healthy account: a directory with credentials and no marker.
        std::fs::create_dir_all(home.join("mail").join("dartmouth")).unwrap();
        std::fs::write(
            home.join("mail").join("accounts.toml"),
            "[[account]]\nname = \"personal\"\nprovider = \"google\"\n\
             [[account]]\nname = \"dartmouth\"\nprovider = \"outlook\"\n",
        )
        .unwrap();

        let findings = examine(&home, utc(NOW));
        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1, "{findings:#?}");
        assert_eq!(mail[0].severity, Severity::Broken);
        assert!(mail[0].summary.contains("personal"), "{}", mail[0].summary);
        let remedy = mail[0].remedy.as_ref().expect("a dead login has a way out");
        assert_eq!(
            remedy.argv,
            vec!["mecha-mail", "auth", "personal", "--provider", "google"]
        );
        assert!(
            remedy.needs_terminal,
            "an OAuth flow needs the real terminal"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_provider_the_registry_cannot_name_is_omitted_from_the_remedy_not_guessed() {
        let home = home("no-registry");
        // No accounts.toml at all.
        write_marker(&home, "personal", &valid_marker());

        let findings = examine(&home, utc(NOW));
        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1);
        let remedy = mail[0].remedy.as_ref().unwrap();
        assert_eq!(remedy.argv, vec!["mecha-mail", "auth", "personal"]);
        // The marker's message names the full command, and it rides in the
        // detail so the operator still sees the provider.
        assert!(
            mail[0].detail.contains("--provider google"),
            "{}",
            mail[0].detail
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Legacy per-provider stores (`<home>/google/oauth.json`, still served
    /// by the shipped `mecha-google` binary) get the same marker beside
    /// their credentials — and the old scan, which read only
    /// `<home>/mail/*/`, walked straight past it.
    #[test]
    fn a_marker_in_a_legacy_per_provider_store_is_found_and_proposes_import() {
        let home = home("legacy-auth");
        let dir = home.join("google");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth_error.json"),
            json!({
                "at": "2026-08-11T09:00:00Z",
                "message": "account `google`: refresh token expired or revoked — run `mecha-mail auth google --provider google` (invalid_grant)",
            })
            .to_string(),
        )
        .unwrap();

        let findings = examine(&home, utc(NOW));
        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1, "{findings:#?}");
        assert_eq!(mail[0].severity, Severity::Broken);
        assert!(
            mail[0].summary.contains("legacy google"),
            "{}",
            mail[0].summary
        );
        // The marker's message names the exact re-auth command; it must ride
        // in the detail.
        assert!(
            mail[0]
                .detail
                .contains("run `mecha-mail auth google --provider google`"),
            "{}",
            mail[0].detail
        );
        let remedy = mail[0].remedy.as_ref().expect("a way out");
        assert_eq!(
            remedy.argv,
            vec!["mecha-mail", "import", "google", "--provider", "google"]
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unparseable_marker_is_a_store_unreadable_finding_not_a_crash() {
        let home = home("bad-marker");
        write_marker(&home, "personal", "{ this is not json");

        let findings = examine(&home, utc(NOW));
        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1, "{findings:#?}");
        assert!(
            mail[0].summary.starts_with("store unreadable:"),
            "{}",
            mail[0].summary
        );
        assert!(mail[0].summary.contains("personal"), "{}", mail[0].summary);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_pending_item_with_an_error_is_broken_and_a_resolved_one_is_not() {
        let home = home("outbox-error");
        pending_item(
            &home,
            "20260814-000001-aaa",
            NOW,
            Some("server unreachable"),
        );
        // A sent item with an old date and even an error field: never flagged.
        let mut sent = json!({
            "id": "20260810-000001-bbb",
            "status": "sent",
            "tool": "mail__send",
            "args_before": {},
            "args": {},
            "summary": "mail__send",
            "created_at": "2026-08-01T00:00:00Z",
        });
        sent["error"] = json!(null);
        std::fs::write(
            home.join("outbox").join("20260810-000001-bbb.json"),
            sent.to_string(),
        )
        .unwrap();

        let findings = examine(&home, utc(NOW));
        let outbox = of(&findings, "outbox");
        assert_eq!(outbox.len(), 1, "{findings:#?}");
        assert_eq!(outbox[0].severity, Severity::Broken);
        assert!(
            outbox[0]
                .summary
                .contains("release failed: server unreachable"),
            "{}",
            outbox[0].summary
        );
        let remedy = outbox[0].remedy.as_ref().unwrap();
        assert_eq!(remedy.argv, vec!["mecha", "outbox", "review"]);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_pending_draft_is_stale_at_49_hours_and_not_at_47() {
        let home = home("outbox-stale");
        // 49h before NOW.
        pending_item(&home, "20260812-110000-old", "2026-08-12T11:00:00Z", None);
        let findings = examine(&home, utc(NOW));
        let outbox = of(&findings, "outbox");
        assert_eq!(outbox.len(), 1, "{findings:#?}");
        assert_eq!(outbox[0].severity, Severity::Attention);
        assert!(outbox[0].summary.contains("pending for more than 48h"));
        assert_eq!(
            outbox[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "outbox", "review"],
            "the remedy is the review surface, never send"
        );

        // 47h old: a person may simply not have reviewed yet.
        let fresh = home;
        let _ = std::fs::remove_dir_all(fresh.join("outbox"));
        pending_item(&fresh, "20260812-130000-new", "2026-08-12T13:00:00Z", None);
        let findings = examine(&fresh, utc(NOW));
        assert!(of(&findings, "outbox").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&fresh);
    }

    #[test]
    fn a_failed_extraction_is_broken_at_any_age() {
        let home = home("frontdoor-failed");
        request(&home, 12, crate::frontdoor::EXTRACTION_FAILED, NOW);

        let findings = examine(&home, utc(NOW));
        let front = of(&findings, "frontdoor");
        assert_eq!(front.len(), 1, "{findings:#?}");
        assert_eq!(front[0].severity, Severity::Broken);
        assert!(front[0].summary.contains("12"), "{}", front[0].summary);
        assert_eq!(
            front[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "frontdoor", "list"]
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_request_waiting_on_me_is_stale_at_73_hours_and_not_at_71() {
        let home = home("frontdoor-stale");
        // 73h before NOW.
        request(
            &home,
            1,
            crate::frontdoor::AWAITING_ME,
            "2026-08-11T11:00:00Z",
        );
        let findings = examine(&home, utc(NOW));
        let front = of(&findings, "frontdoor");
        assert_eq!(front.len(), 1, "{findings:#?}");
        assert_eq!(front[0].severity, Severity::Attention);
        assert!(front[0].summary.contains("waiting on you"));

        // 71h: not yet.
        let _ = std::fs::remove_dir_all(home.join("requests"));
        request(
            &home,
            2,
            crate::frontdoor::AWAITING_ME,
            "2026-08-11T13:00:00Z",
        );
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "frontdoor").is_empty(), "{findings:#?}");

        // And a state waiting on the *requester* is never the user's fault.
        let _ = std::fs::remove_dir_all(home.join("requests"));
        request(
            &home,
            3,
            crate::frontdoor::NEEDS_INFO,
            "2026-08-01T00:00:00Z",
        );
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "frontdoor").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// `triaged` means "triage considered it and drafted nothing — a person
    /// has to decide", and nothing ever re-triages it: left off the
    /// waiting-on-me list it waits forever, invisibly.
    #[test]
    fn a_triaged_request_nothing_will_revisit_goes_stale() {
        let home = home("frontdoor-triaged");
        // 73h before NOW.
        request(&home, 4, crate::frontdoor::TRIAGED, "2026-08-11T11:00:00Z");
        // Older still, but waiting on the *stranger*: never the user's fault.
        request(
            &home,
            5,
            crate::frontdoor::NEEDS_INFO,
            "2026-08-01T00:00:00Z",
        );

        let findings = examine(&home, utc(NOW));
        let front = of(&findings, "frontdoor");
        assert_eq!(front.len(), 1, "{findings:#?}");
        assert_eq!(front[0].severity, Severity::Attention);
        assert!(front[0].detail.contains("triaged"), "{}", front[0].detail);
        assert!(
            !front[0].detail.contains("needs_info"),
            "needs_info waits on the requester: {}",
            front[0].detail
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_trigger_whose_last_run_failed_is_flagged_with_the_manual_probe() {
        let home = home("trigger-failed");
        trigger_file(&home, "morning", "");
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-13T07:00:00Z",
                "started_at": "2026-08-13T07:00:01Z",
                "status": "ok",
                "summary": "fine",
            }),
        );
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "error",
                "error": "provider unreachable",
            }),
        );

        let findings = examine(&home, utc(NOW));
        let triggers = of(&findings, "triggers");
        assert_eq!(triggers.len(), 1, "{findings:#?}");
        assert_eq!(triggers[0].severity, Severity::Attention);
        assert!(triggers[0].summary.contains("morning"));
        assert!(triggers[0].detail.contains("provider unreachable"));
        assert_eq!(
            triggers[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "trigger", "run", "morning"],
            "a manual run is the safe probe: it never advances the schedule"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A skip is a row, not a run: the overlap/staleness bookkeeping the
    /// scheduler appends after a failure must not read as a recovery. The
    /// old check keyed on the literal last ledger row and reported nothing.
    #[test]
    fn a_skip_row_after_a_failed_run_does_not_hide_the_failure() {
        let home = home("trigger-skip-hides-error");
        trigger_file(&home, "morning", "");
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-13T07:00:00Z",
                "started_at": "2026-08-13T07:00:01Z",
                "status": "error",
                "error": "provider unreachable",
            }),
        );
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "skipped-stale",
            }),
        );

        let findings = examine(&home, utc(NOW));
        let triggers = of(&findings, "triggers");
        assert_eq!(triggers.len(), 1, "{findings:#?}");
        assert!(
            triggers[0].summary.contains("most recent run failed"),
            "{}",
            triggers[0].summary
        );
        assert!(triggers[0].detail.contains("provider unreachable"));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_ok_run_followed_by_a_skip_is_healthy() {
        let home = home("trigger-ok-then-skip");
        trigger_file(&home, "morning", "");
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-13T07:00:00Z",
                "started_at": "2026-08-13T07:00:01Z",
                "status": "ok",
            }),
        );
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "skipped-overlap",
            }),
        );

        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "triggers").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_trigger_quietly_failing_a_third_of_its_calls_is_reported() {
        // Every run says `ok` and every briefing arrived. The only place the
        // degradation exists is the call counts, which nothing read before.
        let home = home("trigger-tool-errors");
        trigger_file(&home, "morning", "");
        for day in 10..15 {
            ledger_row(
                &home,
                &json!({
                    "trigger": "morning",
                    "slot": format!("2026-08-{day}T07:00:00Z"),
                    "started_at": format!("2026-08-{day}T07:00:01Z"),
                    "status": "ok",
                    "summary": "briefed",
                    "tool_calls": 6,
                    "tool_errors": 3,
                }),
            );
        }

        let findings = examine(&home, utc(NOW));
        let triggers = of(&findings, "triggers");
        assert_eq!(triggers.len(), 1, "{findings:#?}");
        assert_eq!(triggers[0].severity, Severity::Attention);
        assert!(
            triggers[0].summary.contains("15 of 30"),
            "{}",
            triggers[0].summary
        );
        assert_eq!(
            triggers[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "trigger", "show", "morning"],
            "reading is the remedy — what to change is in the transcript"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_handful_of_failed_calls_is_not_a_trend() {
        // Two rules at once, and both are about not crying wolf. A rate over
        // three calls is noise, so the floor holds; and errors are how a run
        // learns about its environment, so a rate under the bar is silence
        // rather than a quieter finding.
        let home = home("trigger-tool-errors-quiet");
        trigger_file(&home, "morning", "");
        // Under the call floor, though every call failed.
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "ok",
                "tool_calls": 3,
                "tool_errors": 3,
            }),
        );
        assert!(of(&examine(&home, utc(NOW)), "triggers").is_empty());

        // Over the floor, under the rate.
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-15T07:00:00Z",
                "started_at": "2026-08-15T07:00:01Z",
                "status": "ok",
                "tool_calls": 40,
                "tool_errors": 4,
            }),
        );
        assert!(of(&examine(&home, utc(NOW)), "triggers").is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_ledger_written_before_the_counts_existed_reports_nothing() {
        // Not a perfect score, and not a division by zero: no data is not a
        // finding, which is the rule the whole module runs on.
        let home = home("trigger-tool-errors-bare");
        trigger_file(&home, "morning", "");
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "ok",
            }),
        );
        assert!(of(&examine(&home, utc(NOW)), "triggers").is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_disabled_trigger_is_nobody_s_emergency() {
        let home = home("trigger-disabled");
        trigger_file(&home, "morning", "enabled = false\n");
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "error",
                "error": "boom",
            }),
        );
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "triggers").is_empty(), "{findings:#?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_catch_up_trigger_whose_slots_stopped_advancing_names_the_daemon() {
        let home = home("trigger-stale");
        trigger_file(&home, "morning", "");
        // Last accounted slot five days ago; daily at 07:00 UTC, so slots on
        // the 10th..14th are all unaccounted — far more than two.
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-09T07:00:00Z",
                "started_at": "2026-08-09T07:00:01Z",
                "status": "ok",
            }),
        );

        let findings = examine(&home, utc(NOW));
        let triggers = of(&findings, "triggers");
        assert_eq!(triggers.len(), 1, "{findings:#?}");
        assert_eq!(triggers[0].severity, Severity::Attention);
        assert!(triggers[0].summary.contains("missed more than two slots"));
        assert!(
            triggers[0].detail.contains("daemon"),
            "{}",
            triggers[0].detail
        );
        assert!(
            triggers[0].remedy.is_none(),
            "running the trigger would not restart the scheduler"
        );

        // A current ledger is healthy: this morning's 07:00 accounted for.
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "ok",
            }),
        );
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "triggers").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The observer rule, which matters most: one poisoned store must not
    /// suppress what the other checks found — and must itself be reported.
    #[cfg(unix)]
    #[test]
    fn one_poisoned_store_does_not_suppress_the_others() {
        use std::os::unix::fs::PermissionsExt;
        // Root reads through 0o000 like it is not there, and the test would
        // be vacuous.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let home = home("poisoned");
        write_marker(&home, "personal", &valid_marker());
        let outbox = home.join("outbox");
        std::fs::create_dir_all(&outbox).unwrap();
        std::fs::set_permissions(&outbox, std::fs::Permissions::from_mode(0o000)).unwrap();

        let findings = examine(&home, utc(NOW));

        // Restore before asserting, so a failure can still clean up.
        std::fs::set_permissions(&outbox, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1, "the mail finding survived: {findings:#?}");
        assert_eq!(mail[0].severity, Severity::Broken);
        let broken_store = of(&findings, "outbox");
        assert_eq!(broken_store.len(), 1, "{findings:#?}");
        assert!(
            broken_store[0].summary.starts_with("store unreadable:"),
            "{}",
            broken_store[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Finding-6 drift pin, reader half. Twin test (same golden bytes):
    /// `mecha_mail::token::tests::record_auth_error_serialises_the_golden_marker_byte_for_byte`
    /// in mecha-mail/src/token.rs — the crates share no types on purpose
    /// (the seam is a file of JSON), so a field rename on either side would
    /// pass both suites separately and silently kill this finding at
    /// runtime. If this literal changes, change the twin's too.
    #[test]
    fn the_golden_marker_literal_parses_into_the_dead_auth_finding() {
        const GOLDEN: &str = r#"{
  "at": "2026-08-11T09:00:00Z",
  "message": "account `personal`: refresh token expired or revoked — run `mecha-mail auth personal --provider google` (invalid_grant: Token has been revoked.)"
}"#;
        let home = home("golden-marker");
        write_marker(&home, "personal", GOLDEN);

        let findings = examine(&home, utc(NOW));
        let mail = of(&findings, "mail");
        assert_eq!(mail.len(), 1, "{findings:#?}");
        assert_eq!(mail[0].severity, Severity::Broken);
        assert!(
            mail[0].detail.contains("since 2026-08-11T09:00:00Z"),
            "the marker's `at` must reach the detail: {}",
            mail[0].detail
        );
        assert!(
            mail[0]
                .detail
                .contains("run `mecha-mail auth personal --provider google`"),
            "the marker's `message` must reach the detail: {}",
            mail[0].detail
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn findings_sort_broken_first() {
        let mut findings = vec![
            Finding {
                component: "outbox".into(),
                severity: Severity::Attention,
                summary: "stale".into(),
                detail: String::new(),
                remedy: None,
            },
            Finding {
                component: "mail".into(),
                severity: Severity::Broken,
                summary: "dead".into(),
                detail: String::new(),
                remedy: None,
            },
        ];
        sort(&mut findings);
        assert_eq!(findings[0].severity, Severity::Broken);
    }

    #[test]
    fn an_empty_home_is_healthy() {
        let home = home("empty");
        assert!(examine(&home, utc(NOW)).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    // --- graph nightly silence ---

    /// A graph store nested inside a unique scratch dir, so no test plants a
    /// `.mecha-graph` beside another test's home in the shared temp dir.
    fn graph_store(name: &str) -> PathBuf {
        let store = home(name).join(".mecha-graph");
        std::fs::create_dir_all(store.join("logs")).unwrap();
        store
    }

    fn nightly_log(store: &Path, file: &str) {
        std::fs::write(store.join("logs").join(file), "ran\n").unwrap();
    }

    // NOW is 2026-08-14: a 08-12 log is two days quiet (stale), 08-13 is
    // yesterday (the newest a healthy quiet morning can show).

    #[test]
    fn a_graph_nightly_that_stopped_writing_logs_is_a_finding() {
        let store = graph_store("graph-stale");
        nightly_log(&store, "nightly-20260812.log");
        let findings = check_graph_nightly(&store, utc(NOW));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].component, "graph");
        assert_eq!(findings[0].severity, Severity::Attention);
        assert!(
            findings[0].summary.contains("2 days"),
            "{}",
            findings[0].summary
        );
        assert!(
            findings[0].detail.contains("nightly-20260812.log"),
            "{}",
            findings[0].detail
        );
    }

    #[test]
    fn yesterdays_log_is_healthy_because_todays_slot_may_not_have_fired() {
        let store = graph_store("graph-yesterday");
        nightly_log(&store, "nightly-20260813.log");
        nightly_log(&store, "mecha-nightly-20260813.log");
        assert!(check_graph_nightly(&store, utc(NOW)).is_empty());
    }

    /// The two families age independently: the sweep running every night must
    /// not vouch for the vet/gossip half — that is exactly how 2026-08-17
    /// stayed invisible.
    #[test]
    fn each_nightly_family_is_judged_alone() {
        let store = graph_store("graph-split");
        nightly_log(&store, "nightly-20260814.log");
        nightly_log(&store, "mecha-nightly-20260811.log");
        let findings = check_graph_nightly(&store, utc(NOW));
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].summary.contains("mecha-nightly"),
            "{}",
            findings[0].summary
        );
    }

    /// The `nightly-` scan must not claim `mecha-nightly-` files as its own:
    /// a fresh mecha-nightly log would otherwise hide a dead sweep.
    #[test]
    fn the_shorter_prefix_does_not_claim_the_longer_familys_logs() {
        let store = graph_store("graph-prefix");
        nightly_log(&store, "mecha-nightly-20260814.log");
        nightly_log(&store, "nightly-20260810.log");
        let findings = check_graph_nightly(&store, utc(NOW));
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].detail.contains("nightly-20260810.log"),
            "{}",
            findings[0].detail
        );
    }

    /// Absence is "not installed", never a finding — a missing store, an
    /// empty log directory, and names that parse to no date all stay quiet.
    #[test]
    fn a_graph_that_never_ran_is_not_a_finding() {
        let missing = home("graph-missing").join(".mecha-graph");
        assert!(check_graph_nightly(&missing, utc(NOW)).is_empty());

        let empty = graph_store("graph-empty");
        assert!(check_graph_nightly(&empty, utc(NOW)).is_empty());

        let odd = graph_store("graph-odd-names");
        nightly_log(&odd, "nightly-garbage.log");
        nightly_log(&odd, "gossip-20260812.jsonl");
        assert!(check_graph_nightly(&odd, utc(NOW)).is_empty());
    }

    /// The examine wiring: the store is found as the home's hidden sibling.
    #[test]
    fn examine_reads_the_graph_store_beside_the_home() {
        let scratch = home("graph-sibling");
        let mecha_home = scratch.join(".mecha");
        std::fs::create_dir_all(&mecha_home).unwrap();
        let store = scratch.join(".mecha-graph");
        std::fs::create_dir_all(store.join("logs")).unwrap();
        nightly_log(&store, "nightly-20260810.log");
        let findings = examine(&mecha_home, utc(NOW));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].component, "graph");
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
