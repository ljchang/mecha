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
/// forgotten rather than deliberately parked — unless the owner's charter
/// says otherwise; see [`Patience`].
const STUCK_DRAFT_AFTER: chrono::Duration = chrono::Duration::hours(48);

/// A frontdoor request waiting on the user for longer than this is the
/// stranger-facing silence the front door exists to prevent.
const STALE_REQUEST_AFTER: chrono::Duration = chrono::Duration::hours(72);

/// How long a store check waits before it calls an item stuck — the
/// harness's constant, or **the owner's own number** where a charter line
/// carries a sensor on that store (`docs/GOAL-SYSTEM-DESIGN.md` §11.1: "where
/// a line names a setpoint, the doctor reports against the owner's number").
/// The finding then names the line, so the owner reads *which priority* the
/// store is failing rather than a threshold nobody chose.
#[derive(Debug, Clone, PartialEq)]
struct Patience {
    after: chrono::Duration,
    /// As printed — `48h`, or the setpoint in the owner's own spelling.
    text: String,
    /// The charter line the number came from, when it is the owner's.
    line: Option<String>,
}

impl Patience {
    /// The owner's setpoint for `kind`, or the harness's `fallback`.
    ///
    /// A setpoint the charter typed as anything but a duration cannot reach
    /// here — `SensorKind::unit` fixes it — so the fallback arm on a
    /// non-duration is unreachable rather than a silent default.
    fn for_kind(
        charter: Option<&crate::charter::Charter>,
        kind: crate::charter::SensorKind,
        fallback: chrono::Duration,
        fallback_text: &str,
    ) -> Patience {
        if let Some(line) = charter.and_then(|c| c.line_for_sensor(&[kind])) {
            if let Some(sensor) = &line.sensor {
                if let crate::charter::Setpoint::Duration(d) = sensor.setpoint {
                    if let Ok(after) = chrono::Duration::from_std(d) {
                        return Patience {
                            after,
                            text: sensor.setpoint_text.clone(),
                            line: Some(line.id.clone()),
                        };
                    }
                }
            }
        }
        Patience {
            after: fallback,
            text: fallback_text.to_string(),
            line: None,
        }
    }

    /// `48h`, or ``24h (charter line `answer-what-waits-on-me`)``.
    fn describe(&self) -> String {
        match &self.line {
            Some(id) => format!("{} (charter line `{id}`)", self.text),
            None => self.text.clone(),
        }
    }
}

/// The owner's setpoint for `kind`, with the line that carries it — for the
/// kinds whose reading is a count or a rate rather than an age, which the
/// walkers compare against directly.
fn owner_setpoint(
    charter: Option<&crate::charter::Charter>,
    kind: crate::charter::SensorKind,
) -> Option<(crate::charter::Setpoint, String, String)> {
    let line = charter?.line_for_sensor(&[kind])?;
    let sensor = line.sensor.as_ref()?;
    Some((
        sensor.setpoint,
        sensor.setpoint_text.clone(),
        line.id.clone(),
    ))
}

/// Examine every store under `home` and report what is wrong.
///
/// `now` is injected for testability; nothing here consults the clock.
/// Best-effort throughout: each check appends what it found, a failed check
/// appends a finding about the failure, and no check can stop another.
pub fn examine(home: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();
    // The owner's setpoints, read once for the store checks below. A charter
    // that does not load is `check_charter`'s finding; the store checks then
    // read against the harness's constants, as they did before there was a
    // charter — never against a number nobody could parse.
    let charter = crate::charter::Charter::load(&home.join("charter.toml")).ok();
    let charter = charter.as_ref();
    findings.extend(check_mail(&home.join("mail")));
    findings.extend(check_legacy_mail(home));
    findings.extend(check_outbox(&home.join("outbox"), now, charter));
    findings.extend(check_questions(&home.join("questions"), now, charter));
    findings.extend(check_frontdoor(&home.join("requests"), now, charter));
    findings.extend(check_triggers(&home.join("triggers"), now));
    findings.extend(check_charter(&home.join("charter.toml")));
    findings.extend(check_runs(&home.join("sessions"), charter));
    findings.extend(check_harness(&home.join("learning").join("harness"), now));
    findings.extend(check_learning(&home.join("learning"), now));
    findings.extend(check_proposal_review(&home.join("learning"), now));
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
fn check_outbox(
    root: &Path,
    now: DateTime<Utc>,
    charter: Option<&crate::charter::Charter>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let patience = Patience::for_kind(
        charter,
        crate::charter::SensorKind::OutboxAge,
        STUCK_DRAFT_AFTER,
        "48h",
    );
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
    let mut pending: u64 = 0;
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
        pending += 1;
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
        } else if age_of(&item.created_at, now).is_some_and(|age| age > patience.after) {
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
                "{} draft{} pending for more than {}",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" },
                patience.describe()
            ),
            detail: stale.join("\n"),
            remedy: Some(review.clone()),
        });
    }
    // The count kind has no harness constant to fall back to: how many
    // drafts may wait is the owner's number or nobody's, so this fires only
    // where a charter line names one.
    if let Some((crate::charter::Setpoint::Count(max), text, line)) =
        owner_setpoint(charter, crate::charter::SensorKind::OutboxWaiting)
    {
        if pending > max {
            out.push(Finding {
                component: "outbox".to_string(),
                severity: Severity::Attention,
                summary: format!(
                    "{pending} drafts pending, past the {text} setpoint on charter line `{line}`"
                ),
                detail: format!(
                    "the line says at most {text} should wait on you; {pending} do. Every one is \
                     yours to release or reject — doctor never does either"
                ),
                remedy: Some(review),
            });
        }
    }
    out
}

// --- questions nobody answered ----------------------------------------------

/// A question older than this has most likely been missed rather than
/// deliberately left.
///
/// **Shorter than a stale draft's 48h, and deliberately so.** A pending draft
/// is work already done, sitting safely until someone looks. An unanswered
/// question is a *delegation frozen mid-flight*: the run stopped, the task is
/// parked in `waiting`, and nothing moves until a person types one sentence.
/// The cost of the wait is higher, so the patience is shorter.
const UNANSWERED_QUESTION_AFTER: chrono::Duration = chrono::Duration::hours(24);

/// Read the question records directly, for the reason [`check_outbox`] does:
/// an examination that creates or re-chmods the store is measuring itself.
fn check_questions(
    root: &Path,
    now: DateTime<Utc>,
    charter: Option<&crate::charter::Charter>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        // Never asked is not a problem, and must not read as one. A machine
        // that has delegated no tasks looks exactly like this.
        return out;
    }
    let patience = Patience::for_kind(
        charter,
        crate::charter::SensorKind::QuestionLatency,
        UNANSWERED_QUESTION_AFTER,
        "24h",
    );
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "questions",
                "the question store",
                format!("{}: {e}", root.display()),
            ));
            return out;
        }
    };

    let mut stale: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let q: crate::questions::Question =
            match std::fs::read_to_string(&path).map(|t| serde_json::from_str(&t)) {
                Ok(Ok(q)) => q,
                Ok(Err(e)) => {
                    out.push(Finding::unreadable(
                        "questions",
                        &format!(
                            "question {} did not parse",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
                Err(e) => {
                    out.push(Finding::unreadable(
                        "questions",
                        &format!(
                            "question {} could not be read",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
            };
        if !q.is_open() {
            continue;
        }
        if age_of(&q.asked_at, now).is_some_and(|age| age > patience.after) {
            stale.push(format!(
                "{} · {} — asked {}",
                crate::questions::QuestionStore::short(&q.id),
                q.summary(),
                render_age(now, &q.asked_at)
            ));
        }
    }

    if !stale.is_empty() {
        stale.sort();
        out.push(Finding {
            component: "questions".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} question{} unanswered for more than {} — {} run{} cannot continue",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" },
                patience.describe(),
                stale.len(),
                if stale.len() == 1 { "" } else { "s" }
            ),
            detail: stale.join("\n"),
            // Lists rather than answers, on doctor's rule: findings propose
            // and a human disposes. An answer is the owner's words, and a
            // remedy that supplied them would be inventing the thing the
            // question exists to obtain.
            remedy: Some(Remedy {
                description: "see what the agent is stuck on — doctor never answers for you"
                    .to_string(),
                argv: vec!["mecha".into(), "questions".into(), "list".into()],
                needs_terminal: true,
            }),
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
fn check_frontdoor(
    root: &Path,
    now: DateTime<Utc>,
    charter: Option<&crate::charter::Charter>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let patience = Patience::for_kind(
        charter,
        crate::charter::SensorKind::RequestClosure,
        STALE_REQUEST_AFTER,
        "72h",
    );
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
            && request_age(&record, now).is_some_and(|age| age > patience.after)
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
                "{} request{} waiting on you for more than {}",
                stale.len(),
                if stale.len() == 1 { "" } else { "s" },
                patience.describe()
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
                    "across its last {runs} run(s){}. A run's answer arrives either way, so this is invisible in the ledger's status — and per-step reliability is what decides how long a task the run can finish, so a third of the calls failing is not a third of the work lost.",
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

        // The null run: it fired, it succeeded, and it did nothing. The rate
        // check above cannot see this one — a rate over zero calls is
        // undefined rather than bad — so a trigger that made thirty calls a
        // morning and now makes none is silent in every signal the ledger
        // carries. Found by a sibling arc hitting the same shape one layer
        // down, where `mecha mail classify` returned success having classified
        // 0 of 16.
        //
        // Measured against the trigger's *own* history, never an absolute
        // floor: a prompt that legitimately needs no tools makes zero calls
        // every morning, and a check that called that broken would be wrong
        // about the healthiest trigger on the machine. So the earlier runs in
        // the window have to show the work that stopped.
        if let Some(window) = window {
            let newest = window.last();
            let before: u32 = window[..window.len().saturating_sub(1)]
                .iter()
                .map(|r| r.tool_calls)
                .sum();
            // Only an `ok` run: an errored one already has a finding above,
            // and two findings for one fact leave neither meaning anything.
            let stopped = newest
                .is_some_and(|r| r.tool_calls == 0 && r.status == crate::trigger::RunStatus::Ok)
                && before >= HEALTH_MIN_CALLS;
            if stopped {
                out.push(Finding {
                    component: "triggers".to_string(),
                    severity: Severity::Attention,
                    summary: format!(
                        "trigger `{}`'s most recent run did no work",
                        trigger.name
                    ),
                    detail: format!(
                        "it succeeded having made no tool calls, where its previous {} run(s) made {before}. A run that does nothing and reports success is indistinguishable from a healthy one in every other signal — the status is `ok`, the schedule advanced, and the answer arrived.",
                        window.len() - 1
                    ),
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

// --- run quality ------------------------------------------------------------

/// How many sessions back a run-quality check reads.
///
/// Doctor runs in one pass with no network and no model, and each session is
/// a file read — so this is a budget, not a claim about relevance. Two hundred
/// covers weeks of ordinary use and stays well inside "fast enough to run
/// whenever you wonder".
pub const RUNS_WINDOW: usize = 200;

/// Below this many runs *for one model*, no rate is reported.
///
/// Twenty rather than the trigger check's ten, because these rates are
/// population statistics across mixed work rather than one job doing the same
/// thing every morning, and the noise is correspondingly higher.
const RUNS_MIN: usize = 20;

/// The share of runs finishing over a failed call that is worth saying out
/// loud. Deliberately high: rule-based evaluators are measured to *under*
/// report success — they mark good trajectories as failures more often than
/// humans do (AgentRewardBench) — so a low bar here would fire constantly on
/// runs that were fine, and a doctor that cries wolf stops being read.
const ENDED_ON_FAILURE_RATE: f64 = 0.20;

/// The share of attempted tool calls the environment refuses.
const TOOL_ERROR_RATE: f64 = 0.25;

/// And below this many *calls* across the window, no rate at all. Runs and
/// calls are different denominators: twenty runs can hold four calls.
const RUNS_MIN_CALLS: u64 = 20;

/// The share of runs the *harness* cut short. The cancellations — `Stopped`,
/// `Parked`, `Shutdown`, and the older unknown-which `Interrupted` — are
/// excluded from the numerator by [`cut_short`]: a person pressing Ctrl-C is
/// the system working, and counting it would make an attentive user look
/// like a problem.
const CUT_SHORT_RATE: f64 = 0.25;

/// Did the harness end this run? One definition, on [`crate::agent::StopCause`],
/// shared with the candidate gate's metric — see its doc for why there were two.
fn cut_short(stats: &crate::session::RunStats) -> bool {
    stats.stop_cause.is_some_and(|c| c.cut_short())
}

/// A charter that fails to load degrades every run to un-chartered with
/// nothing but a stderr line the TUI's alternate screen covers for the whole
/// session (`setup.rs::prepare_tools`) — the same discovery gap `mecha
/// skills` exists to close for a bad `SKILL.md`, with no `/charter` modal yet
/// to close it here. `Charter::load` is read-only and creates nothing, so
/// calling it directly is safe under doctor's own rule against healing what
/// it is about to report.
fn check_charter(path: &Path) -> Vec<Finding> {
    // `exists()`, not `is_file()`: the latter also reads false for a
    // directory sitting at this path or a broken symlink, which would
    // silently report a broken charter as "nothing written yet" instead of
    // falling through to `Charter::load` below and getting a real `Err` —
    // `read_to_string` on a directory fails with its own I/O error rather
    // than `NotFound`, so `Charter::load` already tells the two apart
    // correctly once it's actually called.
    if !path.exists() {
        return Vec::new();
    }
    let remedy = |description: &str| {
        Some(Remedy {
            description: description.to_string(),
            argv: vec!["mecha".to_string(), "charter".to_string()],
            needs_terminal: false,
        })
    };
    match crate::charter::Charter::load(path) {
        Err(e) => vec![Finding {
            component: "charter".to_string(),
            severity: Severity::Broken,
            summary: "charter did not load".to_string(),
            detail: format!(
                "{}: {e:#} — every run is proceeding un-chartered",
                path.display()
            ),
            remedy: remedy("see the parse error and fix charter.toml"),
        }],
        // Loads and is usable, but costs more of the cached prefix than
        // argued — a warning, not a failure: it still rides in every prompt
        // exactly as authored, the same "warns and still loads" shape
        // `over_budget_domains` gives the learned-rules cap.
        Ok(charter) if charter.over_budget() => vec![Finding {
            component: "charter".to_string(),
            severity: Severity::Attention,
            summary: "charter is over its character budget".to_string(),
            detail: format!(
                "{} is {} characters, over the {}-character budget",
                path.display(),
                charter.char_count(),
                crate::charter::CHARTER_CHAR_BUDGET,
            ),
            remedy: remedy("review the charter and trim it"),
        }],
        // A file that exists and parses *cleanly* to zero lines is an
        // authoring mistake by construction — nobody writes an empty charter
        // on purpose — and otherwise indistinguishable from never having
        // written one at all: `load` returns `Ok`, `prompt_block` returns
        // `None`, and `prepare_tools` prints nothing. This is not the
        // typo'd-table-name case: `RawCharter` denies unknown fields, so
        // `[[lines]]` instead of `[[line]]` is a load error and reaches the
        // `Err` arm above, not this one. What lands here is a file that is
        // empty, or holds only comments.
        Ok(charter) if charter.is_empty() => vec![Finding {
            component: "charter".to_string(),
            severity: Severity::Attention,
            summary: "charter file exists but has no lines".to_string(),
            detail: format!(
                "{} parsed cleanly with zero `[[line]]` entries — nothing from it \
                 rides in any prompt",
                path.display()
            ),
            remedy: remedy("see what's actually in the charter file"),
        }],
        Ok(_) => Vec::new(),
    }
}

/// Report population-level run quality: the signals that are invisible in any
/// single run and obvious across a few hundred.
///
/// Split by model, because a corpus spanning two has no single rate worth
/// quoting — the blend is true and useless, and a threshold on it fires for
/// the wrong model. Silent until there is enough of one model to say
/// anything, which is the same rule as everywhere else here: unknown is not a
/// finding.
fn check_runs(sessions: &Path, charter: Option<&crate::charter::Charter>) -> Vec<Finding> {
    use crate::runlog::{Corpus, Scan};

    let mut out = Vec::new();
    if !sessions.is_dir() {
        return out;
    }
    let corpus = match Corpus::scan(
        sessions,
        &Scan {
            max_sessions: Some(RUNS_WINDOW),
            since: None,
            // Every workspace: doctor reports the machine's health, and
            // health is not scoped to one job.
            workspace: None,
            kind: None,
            include_tests: false,
            // As the other readers: admitted in a trial home, hidden elsewhere.
            include_experiments: crate::experiment::in_experiment_home(),
        },
    ) {
        Ok(c) => c,
        Err(e) => {
            out.push(Finding::unreadable(
                "runs",
                "the session store",
                format!("{}: {e}", sessions.display()),
            ));
            return out;
        }
    };

    // Per-file rot, not the store-level failure above: every reader over
    // this store is best-effort by design (`Session::list` skips a
    // headerless file, `Corpus::scan` a torn body), which is right for the
    // readers and wrong as a *diagnosis* — a store losing one transcript at
    // a time was invisible from every surface at once. Doctor is the one
    // reader whose job is the store itself.
    if corpus.unreadable > 0 {
        out.push(Finding::unreadable(
            "runs",
            &format!("{} transcript(s) in the session store", corpus.unreadable),
            // Precise about what the counter can actually see: both
            // increments are a file that could not be read or carries no
            // session header — `Session::read` and `outcomes_attributed`
            // skip malformed *lines* without erroring, so line-level rot
            // inside a readable transcript is deliberately not claimed here.
            format!(
                "{}: files with a .jsonl extension that could not be read \
                 or carry no session header; every reader silently skips them",
                sessions.display()
            ),
        ));
    }

    // The run check reads the store with smoke tests excluded, like every
    // corpus reader now does — and it is the one reader whose job is the
    // store itself, so a window dominated by smoke tests must not read as
    // "runs are healthy" on a denominator nobody was told shrank (found on
    // review). Reported only when the tests outnumber what was read: a
    // handful beside real use is the mark working, not a finding.
    if corpus.hidden_tests > corpus.sessions_read {
        out.push(Finding {
            component: "runs".into(),
            severity: Severity::Attention,
            summary: format!(
                "the run check is reading {} real session(s) beside {} smoke-test session(s) it hid",
                corpus.sessions_read, corpus.hidden_tests
            ),
            detail: format!(
                "{}: sessions recorded with MECHA_SESSION_KIND=test are excluded from every \
                 corpus readout by default, so the rates below describe the few real runs, \
                 not the store; `--include-tests` shows the rest",
                sessions.display()
            ),
            remedy: Some(Remedy {
                description: "read the run-quality summary with the smoke tests shown".into(),
                argv: vec![
                    "mecha".into(),
                    "sessions".into(),
                    "health".into(),
                    "--include-tests".into(),
                ],
                needs_terminal: false,
            }),
        });
    }

    if let Some(charter) = charter {
        out.extend(check_sensor_saturation(&corpus, charter));
        out.extend(check_intervention_rate(&corpus, charter));
    }

    let remedy = |what: &str| {
        Some(Remedy {
            description: format!("read the run-quality summary ({what})"),
            argv: vec![
                "mecha".into(),
                "sessions".into(),
                "health".into(),
                "--days".into(),
                "30".into(),
            ],
            needs_terminal: false,
        })
    };

    for (model, runs) in corpus.by_model() {
        if runs.len() < RUNS_MIN {
            continue;
        }
        let n = runs.len();

        if let Some(rate) = runs.rate_of(|r| r.stats.ended_on_failed_call) {
            if rate >= ENDED_ON_FAILURE_RATE {
                out.push(Finding {
                    component: "runs".to_string(),
                    severity: Severity::Attention,
                    summary: format!(
                        "{:.0}% of `{model}` runs finished on a failed tool call",
                        rate * 100.0
                    ),
                    detail: format!(
                        "{} of {n} recent run(s). The model stopped of its own accord with its last call failed, and the answer it wrote may report success over it — which nothing in the text or the stop reason can show.",
                        runs.ended_on_failed_call()
                    ),
                    remedy: remedy("which runs, and what failed"),
                });
            }
        }

        if let Some(rate) = runs.tool_error_rate() {
            // The sibling trigger check states the rule this one omitted: a
            // rate over three calls is noise. Twenty conversational runs that
            // made four calls between them must not raise a finding because
            // one of them errored.
            if rate >= TOOL_ERROR_RATE && runs.tool_calls() >= RUNS_MIN_CALLS {
                out.push(Finding {
                    component: "runs".to_string(),
                    severity: Severity::Attention,
                    summary: format!(
                        "`{model}` runs fail {:.0}% of their tool calls",
                        rate * 100.0
                    ),
                    detail: format!(
                        "{} of {} call(s) across {n} run(s) were refused by the environment. Errors are how a run learns where it is, so some are healthy — a quarter of them says something moved: a renamed path, a revoked grant, a tool whose schema the model keeps mis-filling.",
                        runs.tool_errors(),
                        runs.tool_calls()
                    ),
                    remedy: remedy("which tool, and how it failed"),
                });
            }
        }

        if let Some(rate) = runs.rate_of(|r| cut_short(&r.stats)) {
            if rate >= CUT_SHORT_RATE {
                let cut = runs.rows.iter().filter(|r| cut_short(&r.stats)).count();
                out.push(Finding {
                    component: "runs".to_string(),
                    severity: Severity::Attention,
                    summary: format!(
                        "the harness cut {:.0}% of `{model}` runs short",
                        rate * 100.0
                    ),
                    detail: format!(
                        "{cut} of {n} recent run(s) hit a turn, token or cost ceiling, or tripped the loop guard. A budget that stops a quarter of runs is measuring the budget rather than the work — the answers are truncated and say so only in `stop_cause`. Cancellations are not counted here.",
                    ),
                    remedy: remedy("which ceiling, and how often"),
                });
            }
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

/// A consolidation this recent means the empty clean pool is a pass having
/// just consumed it, not evidence never arriving.
///
/// Two days: `learn` now runs per session, so a gap this long means nothing
/// has consolidated across many sessions — which is the starvation the check
/// next door exists to report.
const RECENT_CONSOLIDATION: chrono::Duration = chrono::Duration::hours(48);

/// Did any domain consolidate within `window`? Read from the store's own pass
/// log rather than inferred from rule timestamps: a pass that produced *no*
/// rules still consumed its reflections, and that is exactly the case an
/// inference from rule mtimes would miss.
///
/// "Consolidate" means **consumed reflections**. A retirement pass appends a
/// `LeapRun` too, with `reflexions_processed: 0` — and it runs nightly, so
/// without the filter a retirement was enough to suppress the starved-learner
/// finding for 48h while the pool sat exactly as unconsumed as before.
fn learned_within(root: &Path, now: DateTime<Utc>, window: chrono::Duration) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join("runs.jsonl")) else {
        return false;
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["reflexions_processed"].as_u64().unwrap_or(0) > 0)
        .filter_map(|v| {
            v["created_at"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        })
        .any(|t| now.signed_duration_since(t.with_timezone(&Utc)) < window)
}

/// Read a domain's learned rules **without constructing a store**.
///
/// `LearningStore::open` creates directories, runs `git init` and writes a
/// `.gitignore`. Doctor reports on stores; it must not bring one into being,
/// or running the health check on a machine that has never learned anything
/// leaves a store behind that says it has.
///
/// **An absent file is an empty rule set, not an unknown one** — a domain that
/// has never consolidated has no learned rules, and that is a fact `accept`
/// acts on. `None` is reserved for a file that exists and cannot be read or
/// parsed, where the honest answer is that we do not know and must not claim
/// a proposal is unappliable.
fn read_learned_rules(root: &Path, domain: &str) -> Option<Vec<crate::learning::Rule>> {
    let path = root.join("rules").join(format!("{domain}.learned.toml"));
    if !path.exists() {
        return Some(Vec::new());
    }
    let text = std::fs::read_to_string(&path).ok()?;
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        rules: Vec<crate::learning::Rule>,
    }
    toml::from_str::<File>(&text).ok().map(|f| f.rules)
}

/// `proposals::accept`'s staleness test, restated: positional over every
/// rule, comparing `text` and `enabled`.
///
/// Duplicated rather than shared because `accept` lives in the CLI crate and
/// this is core — but it is a duplication with a test
/// (`the_stale_predicate_matches_accepts`) pinning the two together, since a
/// doctor that disagrees with the verb it recommends is worse than silence.
fn same_rules_as_accept(a: &[crate::learning::Rule], b: &[crate::learning::Rule]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.text == y.text && x.enabled == y.enabled)
}

/// A pending rule proposal older than this is a review nobody knows is due.
///
/// Two nights, deliberately: the nightly is daily, so one unreviewed pass is
/// ordinary and two is a pattern. Shorter would fire on every proposal staged
/// after the owner went to bed.
const STALE_PROPOSAL_AFTER: chrono::Duration = chrono::Duration::hours(48);

/// **The proposal queue stalling itself**, which is invisible from every
/// other angle.
///
/// Every proposal is a full rewrite measured against `rules_before`, and
/// `proposals accept` refuses one whose baseline has moved — so a second
/// pending proposal is not a second decision: accepting either makes the rest
/// unappliable. They also *claim* their reflections, which `learn` then skips,
/// so an unreviewed queue starves the very pass that would replace it.
///
/// On 2026-08-29 four had accumulated over six days holding 27 of 43
/// reflections, `learn` skipped every night for want of three free ones, and
/// `doctor` said nothing — the starved-learner check next door measures
/// *origin exclusion*, so review latency read exactly like a healthy loop.
/// "Nothing went wrong" and "nothing happened" are opposite findings and this
/// is the second, which is why it is a separate check rather than another
/// clause in that one.
fn check_proposal_review(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    let dir = root.join("proposals");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No proposals directory is a store that has never staged one, not a
        // fault. An unreadable one *is* — but read_dir cannot tell us which
        // without a second syscall, and a missing directory is overwhelmingly
        // the common case on a young install.
        return out;
    };

    let mut pending: Vec<(String, DateTime<Utc>, usize, bool)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            out.push(Finding::unreadable(
                "learning",
                "a rule proposal",
                entry.path().display().to_string(),
            ));
            continue;
        };
        let Ok(p) = serde_json::from_str::<crate::learning::Proposal>(&text) else {
            continue;
        };
        if p.status != "pending" {
            continue;
        }
        let Ok(at) = DateTime::parse_from_rfc3339(&p.created_at) else {
            continue;
        };
        // Whether `accept` would still take it — and it must be the *same*
        // predicate, or doctor tells the owner to supersede something that
        // would have applied fine.
        //
        // Two things were wrong here. `LearningStore::open` is a **writing**
        // constructor (it creates `root` and `root/rules`, runs `git init`,
        // writes `.gitignore`), and a check that reports on a store must not
        // create one — the rule this module states two checks up about
        // `Charter::load`. And the comparison was an order-insensitive set of
        // *active* rule texts, where `accept`'s `same_rules` compares
        // positionally over every rule, `text` **and** `enabled` — so a
        // rewrite that reordered the same texts, or one that only flipped a
        // rule's `enabled`, read as unchanged here and as changed there.
        let unappliable = read_learned_rules(root, &p.domain)
            .map(|live| !same_rules_as_accept(&live, &p.rules_before))
            .unwrap_or(false);
        pending.push((
            p.id.clone(),
            at.with_timezone(&Utc),
            p.reflexion_ids.len(),
            unappliable,
        ));
    }

    if pending.is_empty() {
        return out;
    }
    pending.sort_by_key(|(_, at, _, _)| *at);

    let held: usize = pending.iter().map(|(_, _, n, _)| n).sum();
    let oldest = pending[0].1;
    let age = now.signed_duration_since(oldest);
    let unappliable: Vec<&str> = pending
        .iter()
        .filter(|(_, _, _, stale)| *stale)
        .map(|(id, _, _, _)| id.as_str())
        .collect();

    // Dead paper first: it needs no judgement, and its remedy is exact.
    if !unappliable.is_empty() {
        out.push(Finding {
            component: "learning".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} rule proposal(s) can no longer be applied — the live rules moved \
                 after they were measured",
                unappliable.len()
            ),
            detail: format!(
                "`proposals accept` refuses a proposal whose baseline has changed, so these \
                 are not decisions waiting on you — they are paper, and they still hold \
                 their reflections out of `learn`. Superseding releases that evidence \
                 *unconsumed*; rejecting would mark it processed and lose corrections you \
                 never ruled on. Affected: {}.",
                unappliable.join(", ")
            ),
            remedy: Some(Remedy {
                description: "release their reflections back to the pool".to_string(),
                argv: vec![
                    "mecha".into(),
                    "proposals".into(),
                    "supersede".into(),
                    "--stale".into(),
                ],
                needs_terminal: false,
            }),
        });
    }

    // Then latency, for whatever is genuinely still appliable.
    if age > STALE_PROPOSAL_AFTER && unappliable.len() < pending.len() {
        out.push(Finding {
            component: "learning".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} rule proposal(s) awaiting review, oldest {} day(s) — holding {held} \
                 reflection(s) out of `learn`",
                pending.len(),
                age.num_days().max(1),
            ),
            detail: "A pending proposal claims its reflections, and `learn` skips claimed \
                     ones — so an unreviewed queue starves the pass that would replace it, \
                     and every night reports success with nothing to show. Only one of \
                     several can ever be applied: each is a full rewrite measured against \
                     the rules that were live when it was staged."
                .to_string(),
            remedy: Some(Remedy {
                description: "read what is waiting".to_string(),
                argv: vec!["mecha".into(), "proposals".into()],
                needs_terminal: false,
            }),
        });
    }

    out
}

/// A staged harness candidate older than this is the nightly loop waiting on
/// a review nobody knows is due — the same shape as a stuck draft, one store
/// over. Not blocking anything, hence the longer leash.
const STALE_CANDIDATE_AFTER: chrono::Duration = chrono::Duration::hours(72);

/// Below this many origin-excluded reflections, a learner that has not run is
/// a young install, not a starved one — the silence carries no information.
/// High on the doctor rule: a finding that fires on every fresh setup trains
/// the reader to skip the component it names.
const STARVED_LEARNER_MIN_EXCLUDED: usize = 10;

/// The starved learner: reflections keep arriving, the origin gate keeps
/// excluding them, and no domain ever reaches `learn`'s floor — so the rule
/// learner reports success every night and has produced nothing for weeks.
///
/// This is the null-run bug one layer up from the trigger version: every
/// stage exits 0, the ledger says `ok`, and the only evidence is a
/// distribution across a file nothing was reading. The check counts and
/// never judges the gate itself — the exclusions are the provenance design
/// working as specified — so the finding proposes a *decision*, not a
/// command: accept the rate, or change what evidence the loop can use. That
/// is why its remedy is the dry-run that shows the classifications, never
/// anything that loosens the gate.
fn check_learning(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    let path = root.join("reflections.jsonl");
    if !path.is_file() {
        return out;
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            out.push(Finding::unreadable(
                "learning",
                "the reflections archive",
                format!("{}: {e}", path.display()),
            ));
            return out;
        }
    };

    let mut total = 0usize;
    let mut excluded = 0usize;
    let mut newest_excluded: Option<DateTime<Utc>> = None;
    // domain → clean unprocessed reflections. Kept as records rather than
    // a count because the floor `learn` applies is per *situation batch*
    // within a domain, not per domain: three reflections on three focus
    // tools read as a healthy pool by count while nothing is ever learned,
    // which is the incident `LEARN_MIN_REFLECTIONS` exists for, one level
    // down (found on review).
    let mut waiting: std::collections::BTreeMap<String, Vec<crate::learning::Reflexion>> =
        Default::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(r) = serde_json::from_str::<crate::learning::Reflexion>(line) else {
            // One torn line is not the archive; the counts below are still
            // a lower bound, which is the fail-quiet direction for a check
            // whose finding asks a person to make a decision.
            continue;
        };
        total += 1;
        // `r.origin` is the miner's decision at write time, and the store is
        // append-only — `r.learnable()` is what `learn.rs` actually admits
        // today, re-derived rather than stored, on the same split
        // `Session::taint_timeline` makes about checkpoints. Reading
        // `r.origin` directly here disagreed with the gate on exactly the
        // records a harness-voice fix reclassifies: they would sit in
        // `waiting` forever (never marked processed, since `learn` skips
        // them), silently suppressing this very finding with the records
        // that caused the starvation.
        if r.learnable() {
            if !r.is_processed {
                waiting.entry(r.domain.clone()).or_default().push(r);
            }
        } else if r.dropped_at.is_none() {
            // `learnable()` checks the drop before it checks provenance, so
            // `!r.learnable()` alone cannot tell "the gate excluded this" from
            // "the owner refused this" — and `/learning`'s whole point is
            // letting the owner do the latter. Counting a drop as a provenance
            // exclusion would make dropping ten lessons the owner disagrees
            // with (the intended use of that key) both trip this finding and
            // report the refusal as the gate's doing, with a remedy
            // (`mecha reflect --dry-run`) that answers a question nobody
            // asked. Only what provenance itself blocked counts here.
            excluded += 1;
            if let Ok(t) = DateTime::parse_from_rfc3339(&r.created_at) {
                let t = t.with_timezone(&Utc);
                if newest_excluded.is_none_or(|n| t > n) {
                    newest_excluded = Some(t);
                }
            }
        }
    }

    let floor = crate::learning::LEARN_MIN_REFLECTIONS;
    // Any domain at the floor means learn will consolidate on its next pass:
    // not starved, whatever the exclusion count says.
    // The same split `learn` makes: a batch of one situation reaches the
    // floor, or nothing does.
    let batches: Vec<(String, crate::situation::Situation, usize)> = waiting
        .iter()
        .flat_map(|(domain, rs)| {
            crate::learning::batches_by_region(rs.clone())
                .into_iter()
                .map(move |(region, batch)| (domain.clone(), region, batch.len()))
        })
        .collect();
    if batches.iter().any(|(_, _, n)| *n >= floor) {
        return out;
    }
    // **A loop that just ran is the opposite of starved**, and without this
    // it reports starvation loudest immediately after succeeding: a
    // consolidation marks its reflections processed, so the clean pool it
    // leaves behind is *empty by construction* and every remaining exclusion
    // is suddenly the whole picture. Latent while `learn` staged proposals
    // and consumed nothing; live consolidation (2026-08-29) made it the
    // normal state. An empty pool after a pass is the pass working.
    if learned_within(root, now, RECENT_CONSOLIDATION) {
        return out;
    }
    if excluded < STARVED_LEARNER_MIN_EXCLUDED {
        return out;
    }
    // A loop nothing has fed for a month is dormant, not starved — the
    // distinction matters because the remedy for dormant is elsewhere
    // (triggers, reflect itself), and this finding must not shadow it.
    let alive = newest_excluded.is_some_and(|t| now.signed_duration_since(t).num_days() <= 30);
    if !alive {
        return out;
    }

    let pool = if batches.is_empty() {
        "none clean and unprocessed".to_string()
    } else {
        batches
            .iter()
            .map(|(d, region, n)| format!("{d} [{}] {n}/{floor}", region.describe()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    out.push(Finding {
        component: "learning".to_string(),
        severity: Severity::Attention,
        summary: format!(
            "the rule learner is starved: {excluded} of {total} reflections excluded by \
             origin, and no situation batch reaches the learn floor of {floor}"
        ),
        detail: format!(
            "reflect keeps mining and the provenance gate keeps excluding — the gate working \
             as designed, every night, with nothing downstream to show for it. Clean pool: \
             {pool}. The excluded records stay readable in {} — some are third-party evidence \
             the gate held back, some may be mecha's own words correctly kept out of a \
             feedback loop; the decision this proposes is yours, not a command's: read what \
             got excluded, and change what evidence the loop may consolidate if the mix \
             looks wrong.",
            path.display()
        ),
        remedy: Some(Remedy {
            description: "see how new interventions classify — doctor never loosens the gate"
                .to_string(),
            argv: vec!["mecha".into(), "reflect".into(), "--dry-run".into()],
            needs_terminal: false,
        }),
    });
    out
}

/// Scan `<learning>/harness/candidates` for staged candidates waiting on the
/// user. Quiet when the store has never existed — the loop not being wired is
/// not a finding. Reads the files directly, on the rule that an examination
/// must not heal (or create) what it reports on.
fn check_harness(root: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    let dir = root.join("candidates");
    if !dir.is_dir() {
        return out;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            out.push(Finding::unreadable(
                "harness",
                "the harness candidate directory",
                format!("{}: {e}", dir.display()),
            ));
            return out;
        }
    };
    let mut stale: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let cand: crate::harness::HarnessCandidate =
            match std::fs::read_to_string(&path).map(|t| serde_json::from_str(&t)) {
                Ok(Ok(cand)) => cand,
                Ok(Err(e)) => {
                    out.push(Finding::unreadable(
                        "harness",
                        &format!(
                            "candidate {} did not parse",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
                Err(e) => {
                    out.push(Finding::unreadable(
                        "harness",
                        &format!(
                            "candidate {} could not be read",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
            };
        if !cand.pending() {
            continue;
        }
        let old_enough = chrono::DateTime::parse_from_rfc3339(&cand.created_at)
            .map(|t| {
                now.signed_duration_since(t.with_timezone(&chrono::Utc)) > STALE_CANDIDATE_AFTER
            })
            // An unparseable stamp cannot prove the candidate is fresh.
            .unwrap_or(true);
        if old_enough {
            stale.push(format!("{} · {:?} {}", cand.id, cand.class, cand.change));
        }
    }
    if !stale.is_empty() {
        stale.sort();
        out.push(Finding {
            component: "harness".to_string(),
            severity: Severity::Attention,
            summary: format!(
                "{} harness candidate(s) staged for more than {}h",
                stale.len(),
                STALE_CANDIDATE_AFTER.num_hours()
            ),
            detail: stale.join("\n"),
            remedy: Some(Remedy {
                description: "review the staged candidates — doctor never accepts one".to_string(),
                argv: vec!["mecha".into(), "harness".into(), "list".into()],
                needs_terminal: false,
            }),
        });
    }
    out
}

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

// --- charter sensors, read back off the recorded runs -----------------------

/// A sensored line that has read past its setpoint on each of the last
/// [`crate::reading::SATURATED_AFTER_RUNS`] recorded runs
/// (`docs/GOAL-SYSTEM-DESIGN.md` §11.1, containment 5's second guard).
///
/// Either the debt is real — in which case the store's own finding above
/// names the item — or the setpoint is tighter than the line means, and a
/// reading that is always past its setpoint is the constant the sensor
/// exists to replace. Doctor cannot tell which; it says both. The streak
/// counts only rows that read the *same* sensor — same line, same kind,
/// same setpoint spelling — so an edited setpoint starts a fresh streak,
/// and skips rows whose reading says nothing either way (`Unread`,
/// `Deferred`, or a row from before the field).
fn check_sensor_saturation(
    corpus: &crate::runlog::Corpus,
    charter: &crate::charter::Charter,
) -> Vec<Finding> {
    use crate::reading::SATURATED_AFTER_RUNS;
    let mut rows: Vec<&crate::runlog::RunRow> = corpus
        .rows
        .iter()
        .filter(|r| {
            r.stats
                .homeostat
                .as_ref()
                .is_some_and(|h| h.charter.is_some())
        })
        .collect();
    // Newest first. The corpus is session-newest-first with runs in order
    // inside a session, which is not quite the same thing.
    rows.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.run.cmp(&a.run)));
    let mut out = Vec::new();
    for line in charter.lines() {
        let Some(sensor) = &line.sensor else {
            continue;
        };
        let streak: Vec<bool> = rows
            .iter()
            .filter_map(|r| {
                r.stats
                    .homeostat
                    .as_ref()?
                    .charter
                    .as_ref()?
                    .iter()
                    .find(|lr| {
                        lr.line == line.id
                            && lr.kind == sensor.kind
                            && lr.setpoint == sensor.setpoint_text
                    })?
                    .reading
                    .over()
            })
            .take(SATURATED_AFTER_RUNS)
            .collect();
        if streak.len() == SATURATED_AFTER_RUNS && streak.iter().all(|over| *over) {
            out.push(Finding {
                component: "charter".to_string(),
                severity: Severity::Attention,
                summary: format!(
                    "charter line `{}` has read past its {} setpoint on each of the last {} runs",
                    line.id, sensor.setpoint_text, SATURATED_AFTER_RUNS
                ),
                detail: format!(
                    "sensor `{}`: either what it watches has genuinely waited past {} that whole \
                     time — the store's own finding names the item — or the setpoint is tighter \
                     than the line means (an hour where you meant a day). A reading that is \
                     always past its setpoint is the constant the sensor exists to replace",
                    sensor.kind.wire(),
                    sensor.setpoint_text
                ),
                remedy: Some(Remedy {
                    description: "see each sensor's current reading beside its line".to_string(),
                    argv: vec!["mecha".to_string(), "charter".to_string()],
                    needs_terminal: false,
                }),
            });
        }
    }
    out
}

/// The `intervention_rate` kind, read off the corpus the run check already
/// scanned and compared with the owner's setpoint — the one sensor kind
/// whose store is the session store, so its finding lives with the run
/// findings rather than with a store walker. Silent under [`RUNS_MIN`]
/// rows, like every rate here: a share of three runs is noise.
fn check_intervention_rate(
    corpus: &crate::runlog::Corpus,
    charter: &crate::charter::Charter,
) -> Vec<Finding> {
    let Some((crate::charter::Setpoint::Rate(max), text, line)) =
        owner_setpoint(Some(charter), crate::charter::SensorKind::InterventionRate)
    else {
        return Vec::new();
    };
    if corpus.len() < RUNS_MIN {
        return Vec::new();
    }
    let Some(rate) = corpus.intervention_rate() else {
        return Vec::new();
    };
    if rate <= max {
        return Vec::new();
    }
    vec![Finding {
        component: "runs".to_string(),
        severity: Severity::Attention,
        summary: format!(
            "you stepped into {:.0}% of the last {} runs, past the {text} setpoint on charter line `{line}`",
            rate * 100.0,
            corpus.len()
        ),
        detail: "counted as a denied tool call or a stop by request on the run record — steers, \
                 corrected follow-ups and edited drafts are not on it, so this under-counts \
                 rather than guesses"
            .to_string(),
        remedy: Some(Remedy {
            description: "read what the owner has been stepping in on".to_string(),
            argv: vec![
                "mecha".to_string(),
                "reflect".to_string(),
                "--dry-run".to_string(),
            ],
            needs_terminal: false,
        }),
    }]
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
            filled_defaults: Vec::new(),
            call_id: None,
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

    fn question(home: &Path, id: &str, asked_at: &str, status: &str) {
        let dir = home.join("questions");
        std::fs::create_dir_all(&dir).unwrap();
        let q = crate::questions::Question {
            id: id.into(),
            status: status.into(),
            question: "Which address should the letter go to?".into(),
            options: vec![],
            session_id: "sess-1".into(),
            task_id: Some("task-9".into()),
            workspace: None,
            taint: Default::default(),
            asked_at: asked_at.into(),
            answered_at: None,
            answer: None,
        };
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(&q).unwrap(),
        )
        .unwrap();
    }

    /// Shorter patience than a stale draft's 48h, because the cost differs: a
    /// pending draft is finished work sitting safely, while an unanswered
    /// question is a run that stopped and a task parked in `waiting`.
    #[test]
    fn an_unanswered_question_is_stale_at_25_hours_and_not_at_23() {
        let home = home("questions-stale");
        question(
            &home,
            "20260813-100000-aaaaaaaa",
            "2026-08-13T10:00:00Z",
            "open",
        );
        let findings = examine(&home, utc(NOW));
        let qs = of(&findings, "questions");
        assert_eq!(qs.len(), 1, "{findings:#?}");
        assert_eq!(qs[0].severity, Severity::Attention);
        assert!(
            qs[0].summary.contains("cannot continue"),
            "{:?}",
            qs[0].summary
        );
        assert_eq!(
            qs[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "questions", "list"],
            "doctor lists what is stuck; it never answers for the owner"
        );

        let fresh = home;
        let _ = std::fs::remove_dir_all(fresh.join("questions"));
        question(
            &fresh,
            "20260813-130000-bbbbbbbb",
            "2026-08-13T13:00:00Z",
            "open",
        );
        assert!(of(&examine(&fresh, utc(NOW)), "questions").is_empty());
        let _ = std::fs::remove_dir_all(&fresh);
    }

    /// An answered question is history, however old. Ageing the record rather
    /// than the *waiting* would turn the permanent archive into a permanent
    /// finding — the store never deletes, so this would only ever grow.
    #[test]
    fn an_answered_question_never_ages_into_a_finding() {
        let home = home("questions-answered");
        question(
            &home,
            "20260701-100000-cccccccc",
            "2026-07-01T10:00:00Z",
            "answered",
        );
        question(
            &home,
            "20260701-100000-dddddddd",
            "2026-07-01T10:00:00Z",
            "abandoned",
        );
        assert!(of(&examine(&home, utc(NOW)), "questions").is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Never having asked is not a problem and must not read as one — a
    /// machine that has delegated no tasks has no question store at all.
    #[test]
    fn a_store_that_was_never_created_is_not_a_finding() {
        let home = home("questions-absent");
        assert!(of(&examine(&home, utc(NOW)), "questions").is_empty());
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

    fn harness_candidate(home: &Path, id: &str, created_at: &str, status: &str) {
        let dir = home.join("learning").join("harness").join("candidates");
        std::fs::create_dir_all(&dir).unwrap();
        let cand = crate::harness::HarnessCandidate {
            id: id.into(),
            created_at: created_at.into(),
            class: crate::candidate::ChangeClass::Config,
            change: "compact_at_tokens=24000".into(),
            metric: crate::candidate::Metric::CutShort,
            rationale: "test".into(),
            evidence: String::new(),
            model: None,
            status: status.into(),
            measurement: None,
            resolved_at: None,
            reason: None,
        };
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(&cand).unwrap(),
        )
        .unwrap();
    }

    fn reflection_line(id: &str, origin: &str, processed: bool, created_at: &str) -> String {
        reflection_line_with_intervention(id, origin, processed, created_at, "")
    }

    /// Same record, with the `intervention` text a caller wants to control —
    /// for a reflection stored `clean` before `is_harness_voice` existed,
    /// whose *effective* provenance (`Reflexion::provenance`) is `Derived`
    /// once the text is mecha's own.
    fn reflection_line_with_intervention(
        id: &str,
        origin: &str,
        processed: bool,
        created_at: &str,
        intervention: &str,
    ) -> String {
        serde_json::json!({
            "id": id,
            "domain": "behavior",
            "session_id": "s",
            "trigger": "steer",
            "context": "",
            "intervention": intervention,
            "reflexion_text": "test",
            "is_processed": processed,
            "created_at": created_at,
            "origin": origin,
        })
        .to_string()
    }

    /// A reflection the owner refused with `/learning`'s `d` key — the
    /// counterpart provenance exclusion is blind to: `learnable()` checks
    /// the drop before it checks origin, so a naive `!learnable()` count
    /// cannot tell "the gate excluded this" from "the owner refused this".
    fn reflection_line_dropped(id: &str, origin: &str, created_at: &str) -> String {
        serde_json::json!({
            "id": id,
            "domain": "behavior",
            "session_id": "s",
            "trigger": "steer",
            "context": "",
            "intervention": "",
            "reflexion_text": "test",
            "is_processed": false,
            "created_at": created_at,
            "origin": origin,
            "dropped_at": created_at,
        })
        .to_string()
    }

    fn write_reflections(home: &Path, lines: &[String]) {
        let dir = home.join("learning");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reflections.jsonl"), lines.join("\n")).unwrap();
    }

    /// **A loop that just consolidated is not starved**, and this is the
    /// case that reports starvation loudest without the guard: `learn` marks
    /// its reflections processed, so the clean pool it leaves is empty by
    /// construction and every remaining exclusion becomes the whole picture.
    ///
    /// Latent while learning staged proposals and consumed nothing; live
    /// consolidation made it the normal state on 2026-08-29, when `doctor`
    /// called the learner starved minutes after it turned 28 reflections
    /// into 12 rules.
    #[test]
    fn a_learner_that_just_ran_is_not_starved() {
        let home = home("learning-just-ran");
        let root = home.join("learning");
        std::fs::create_dir_all(&root).unwrap();

        // Twelve exclusions and an empty clean pool: the starved shape.
        let mut lines = String::new();
        for i in 0..12 {
            lines.push_str(&format!(
                r#"{{"id":"x{i}","domain":"behavior","session_id":"s","trigger":"steer","context":"c","intervention":"i","reflexion_text":"t","error_type":null,"confidence":null,"is_processed":false,"leap_run_id":null,"created_at":"2026-08-28T00:00:00Z","origin":"untrusted","evidence":"full"}}
"#
            ));
        }
        std::fs::write(root.join("reflections.jsonl"), &lines).unwrap();

        let now = DateTime::parse_from_rfc3339("2026-08-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // With no pass on record, that is genuine starvation.
        assert!(
            check_learning(&root, now)
                .iter()
                .any(|f| f.summary.contains("starved")),
            "an unfed learner with no recent pass is starved"
        );

        // A pass hours ago explains the empty pool, and the finding goes.
        std::fs::write(
            root.join("runs.jsonl"),
            "{\"id\":\"r1\",\"domain\":\"behavior\",\"reflexions_processed\":28,\"rules_before\":0,\"rules_after\":12,\"created_at\":\"2026-08-29T09:00:00Z\"}\n",
        )
        .unwrap();
        assert!(
            check_learning(&root, now).is_empty(),
            "a consolidation nine hours ago is the pool being consumed, not starvation"
        );

        // A pass from last month does not explain today's empty pool.
        std::fs::write(
            root.join("runs.jsonl"),
            "{\"id\":\"r1\",\"domain\":\"behavior\",\"reflexions_processed\":28,\"rules_before\":0,\"rules_after\":12,\"created_at\":\"2026-07-20T09:00:00Z\"}\n",
        )
        .unwrap();
        assert!(
            check_learning(&root, now)
                .iter()
                .any(|f| f.summary.contains("starved")),
            "a pass five weeks old explains nothing about today"
        );

        // A *retirement* pass hours ago consumed nothing — `rules
        // propose-retirements --apply` appends a LeapRun with
        // `reflexions_processed: 0`, nightly — so it must not read as the
        // pool having been consumed.
        std::fs::write(
            root.join("runs.jsonl"),
            "{\"id\":\"r2\",\"domain\":\"behavior\",\"reflexions_processed\":0,\"rules_before\":12,\"rules_after\":11,\"created_at\":\"2026-08-29T09:00:00Z\"}\n",
        )
        .unwrap();
        assert!(
            check_learning(&root, now)
                .iter()
                .any(|f| f.summary.contains("starved")),
            "a retirement pass consumed no reflections and must not silence starvation"
        );

        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_learner_fed_only_excluded_evidence_is_starved_and_a_met_floor_is_not() {
        let home = home("learning-starved");
        // 12 recent exclusions, one clean reflection stuck below the floor.
        let mut lines: Vec<String> = (0..12)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z"))
            .collect();
        lines.push(reflection_line(
            "c1",
            "clean",
            false,
            "2026-08-05T00:00:00Z",
        ));
        write_reflections(&home, &lines);

        let findings = examine(&home, utc(NOW));
        let learning = of(&findings, "learning");
        assert_eq!(learning.len(), 1, "{findings:#?}");
        assert_eq!(learning[0].severity, Severity::Attention);
        assert!(
            learning[0].summary.contains("starved"),
            "{}",
            learning[0].summary
        );
        assert!(
            learning[0].summary.contains("12 of 13"),
            "{}",
            learning[0].summary
        );
        assert_eq!(
            learning[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "reflect", "--dry-run"],
            "the remedy shows classifications; nothing may loosen the gate"
        );

        // A domain at the floor means learn runs tonight: not starved.
        lines.push(reflection_line(
            "c2",
            "clean",
            false,
            "2026-08-06T00:00:00Z",
        ));
        lines.push(reflection_line(
            "c3",
            "clean",
            false,
            "2026-08-07T00:00:00Z",
        ));
        write_reflections(&home, &lines);
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "learning").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The floor is per situation batch, as `learn` applies it: three
    /// clean reflections on three different focus tools do not meet it,
    /// and the finding names each batch. Fails on the per-domain count.
    #[test]
    fn a_pool_split_across_situations_below_the_floor_is_still_starved() {
        let home = home("learning-starved-regions");
        let mut lines: Vec<String> = (0..12)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z"))
            .collect();
        for (i, tool) in ["shell", "fs_write", "http_fetch"].iter().enumerate() {
            let mut v: serde_json::Value = serde_json::from_str(&reflection_line(
                &format!("c{i}"),
                "clean",
                false,
                "2026-08-05T00:00:00Z",
            ))
            .unwrap();
            v["trigger"] = serde_json::json!("denial");
            v["situation"] = serde_json::json!({ "tools": [tool], "trigger": "denial" });
            lines.push(v.to_string());
        }
        write_reflections(&home, &lines);
        let findings = examine(&home, utc(NOW));
        let learning = of(&findings, "learning");
        assert_eq!(learning.len(), 1, "{findings:#?}");
        assert!(learning[0].summary.contains("no situation batch reaches"));
        assert!(
            learning[0].detail.contains("[shell] 1/3"),
            "{}",
            learning[0].detail
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Dropping ten lessons is `/learning`'s intended use, and must not read
    /// as the provenance gate failing: "refused by the owner" and "held back
    /// by the gate" are opposite findings with opposite remedies, and
    /// conflating them would report a person's own decisions back to them as
    /// a starved learner.
    #[test]
    fn an_owners_drop_is_not_a_provenance_exclusion() {
        let home = home("learning-dropped");
        // Twelve reflections the owner refused by hand — enough to trip the
        // old, unsplit count, and recent enough to read as alive if it did.
        let lines: Vec<String> = (0..12)
            .map(|i| reflection_line_dropped(&format!("d{i}"), "untrusted", "2026-08-13T12:00:00Z"))
            .collect();
        write_reflections(&home, &lines);
        assert!(
            of(&examine(&home, utc(NOW)), "learning").is_empty(),
            "a dozen owner refusals must not read as a starved learner"
        );

        // Mixed with genuine provenance exclusions below the finding's own
        // floor: still quiet, because the drops must not pad the count that
        // decides whether the gate — not the owner — is the story.
        let mut lines = lines;
        lines.extend((0..5).map(|i| {
            reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z")
        }));
        write_reflections(&home, &lines);
        assert!(
            of(&examine(&home, utc(NOW)), "learning").is_empty(),
            "5 genuine exclusions is below the floor even with 12 drops beside them"
        );

        // Past the floor on genuine exclusions alone, the finding fires and
        // its own count excludes every drop.
        lines.extend((5..10).map(|i| {
            reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z")
        }));
        write_reflections(&home, &lines);
        let findings = examine(&home, utc(NOW));
        let learning = of(&findings, "learning");
        assert_eq!(learning.len(), 1, "{findings:#?}");
        assert!(
            learning[0].summary.contains("10 of"),
            "the 12 drops must not be counted as excluded: {}",
            learning[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Two reflections stored `clean` before `is_harness_voice` existed —
    /// mecha's own nudge, mined as though a person had typed it — must not
    /// count toward the waiting pool just because the *stored* field says
    /// clean. `learn` skips them via `learnable()` and never marks them
    /// processed, so counting them here would let them sit in `waiting`
    /// forever and permanently suppress the very starvation they caused.
    #[test]
    fn a_reflection_stored_clean_before_harness_voice_existed_does_not_count_as_waiting() {
        let home = home("learning-harness-voice");
        let mut lines: Vec<String> = (0..10)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z"))
            .collect();
        // Two self-authored nudges, recorded `clean` at the time.
        lines.push(reflection_line_with_intervention(
            "h1",
            "clean",
            false,
            "2026-08-05T00:00:00Z",
            crate::agent::FINAL_ANSWER_NUDGE,
        ));
        lines.push(reflection_line_with_intervention(
            "h2",
            "clean",
            false,
            "2026-08-06T00:00:00Z",
            crate::agent::FINAL_ANSWER_NUDGE,
        ));
        // One genuine clean reflection: the floor is 3, so under the old
        // origin-only count this domain would read 3/3 (not starved) —
        // under `learnable()` it reads 1/3, and the finding still fires.
        lines.push(reflection_line(
            "c1",
            "clean",
            false,
            "2026-08-07T00:00:00Z",
        ));
        write_reflections(&home, &lines);

        let findings = examine(&home, utc(NOW));
        let learning = of(&findings, "learning");
        assert_eq!(learning.len(), 1, "{findings:#?}");
        assert!(
            learning[0].summary.contains("starved"),
            "the two harness-voice records must not read as met-floor evidence: {}",
            learning[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn thin_or_dormant_exclusion_is_not_starvation() {
        let home = home("learning-thin");
        // Nine exclusions: below the evidence floor, silence means nothing yet.
        let lines: Vec<String> = (0..9)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z"))
            .collect();
        write_reflections(&home, &lines);
        assert!(of(&examine(&home, utc(NOW)), "learning").is_empty());

        // Twelve exclusions, all long stale: a dormant loop, not a starved one
        // — the newest excluded reflection is months before NOW.
        let lines: Vec<String> = (0..12)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-05-01T12:00:00Z"))
            .collect();
        write_reflections(&home, &lines);
        assert!(of(&examine(&home, utc(NOW)), "learning").is_empty());

        // Processed clean reflections do not count toward the waiting pool —
        // consumed evidence is not a pool the floor can be met from.
        let mut lines: Vec<String> = (0..12)
            .map(|i| reflection_line(&format!("u{i}"), "untrusted", false, "2026-08-13T12:00:00Z"))
            .collect();
        for i in 0..3 {
            lines.push(reflection_line(
                &format!("p{i}"),
                "clean",
                true,
                "2026-08-05T00:00:00Z",
            ));
        }
        write_reflections(&home, &lines);
        let findings = examine(&home, utc(NOW));
        assert_eq!(of(&findings, "learning").len(), 1, "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_staged_harness_candidate_is_stale_at_73_hours_and_not_at_71() {
        let home = home("harness-stale");
        // 73h before NOW.
        harness_candidate(&home, "hc-old", "2026-08-11T11:00:00Z", "staged");
        // Resolved candidates never nag, however old.
        harness_candidate(&home, "hc-done", "2026-08-01T00:00:00Z", "rejected");
        let findings = examine(&home, utc(NOW));
        let harness = of(&findings, "harness");
        assert_eq!(harness.len(), 1, "{findings:#?}");
        assert_eq!(harness[0].severity, Severity::Attention);
        assert!(harness[0].summary.contains("staged for more than 72h"));
        assert!(
            harness[0].detail.contains("hc-old"),
            "{}",
            harness[0].detail
        );
        assert_eq!(
            harness[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "harness", "list"],
            "the remedy is the review surface, never accept"
        );

        // 71h old: the person may simply not have looked yet.
        let _ = std::fs::remove_dir_all(home.join("learning"));
        harness_candidate(&home, "hc-new", "2026-08-11T13:00:00Z", "staged");
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "harness").is_empty(), "{findings:#?}");

        let _ = std::fs::remove_dir_all(&home);
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
    fn a_trigger_that_stopped_doing_anything_is_reported() {
        // The null run. Status `ok`, schedule advanced, answer delivered, and
        // no work done — invisible in every signal the ledger carried before.
        let home = home("trigger-stopped-working");
        trigger_file(&home, "morning", "");
        for day in 10..14 {
            ledger_row(
                &home,
                &json!({
                    "trigger": "morning",
                    "slot": format!("2026-08-{day}T07:00:00Z"),
                    "started_at": format!("2026-08-{day}T07:00:01Z"),
                    "status": "ok",
                    "tool_calls": 8,
                    "tool_errors": 0,
                }),
            );
        }
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "ok",
                "summary": "nothing to report",
                "tool_calls": 0,
                "tool_errors": 0,
            }),
        );

        let findings = examine(&home, utc(NOW));
        let triggers = of(&findings, "triggers");
        assert_eq!(triggers.len(), 1, "{findings:#?}");
        assert!(
            triggers[0].summary.contains("did no work"),
            "{}",
            triggers[0].summary
        );
        assert!(triggers[0].detail.contains("made 32"));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_trigger_that_never_needed_tools_is_not_broken_for_not_using_them() {
        // The reason this is measured against the trigger's own history and
        // never an absolute floor: a prompt that needs no tools makes zero
        // calls every morning, and calling that broken would be wrong about
        // the healthiest trigger on the machine.
        let home = home("trigger-never-used-tools");
        trigger_file(&home, "haiku", "");
        for day in 10..15 {
            ledger_row(
                &home,
                &json!({
                    "trigger": "haiku",
                    "slot": format!("2026-08-{day}T07:00:00Z"),
                    "started_at": format!("2026-08-{day}T07:00:01Z"),
                    "status": "ok",
                    "tool_calls": 0,
                    "tool_errors": 0,
                }),
            );
        }
        assert!(of(&examine(&home, utc(NOW)), "triggers").is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_failed_run_that_did_no_work_is_reported_once_not_twice() {
        // An errored run already has a finding naming the error. Reporting the
        // absence of work on top of it would be two findings for one fact,
        // and a reader who has to decide which of two rows is the real one is
        // reading a worse report than one row.
        let home = home("trigger-failed-no-work");
        trigger_file(&home, "morning", "");
        for day in 10..14 {
            ledger_row(
                &home,
                &json!({
                    "trigger": "morning",
                    "slot": format!("2026-08-{day}T07:00:00Z"),
                    "started_at": format!("2026-08-{day}T07:00:01Z"),
                    "status": "ok",
                    "tool_calls": 8,
                    "tool_errors": 0,
                }),
            );
        }
        ledger_row(
            &home,
            &json!({
                "trigger": "morning",
                "slot": "2026-08-14T07:00:00Z",
                "started_at": "2026-08-14T07:00:01Z",
                "status": "error",
                "error": "provider unreachable",
                "tool_calls": 0,
                "tool_errors": 0,
            }),
        );

        let triggers = of(&examine(&home, utc(NOW)), "triggers")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(triggers.len(), 1, "{triggers:#?}");
        assert!(triggers[0].detail.contains("provider unreachable"));

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The regression this pins: a corrupt transcript was invisible from
    /// every surface at once — `Session::list` skips it "quietly",
    /// `sessions appraise` counts it nowhere, and doctor said "nothing
    /// wrong". Every reader stays best-effort; doctor is the one whose job
    /// is the store itself, so the skip count surfaces here.
    #[test]
    fn an_unreadable_transcript_is_a_finding_not_an_empty_queue() {
        let home = home("runs-unreadable");
        let dir = home.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260828T000000-torn.jsonl"), "not json\n").unwrap();

        let all = examine(&home, utc(NOW));
        let findings = of(&all, "runs");
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0].summary.contains("unreadable") && findings[0].summary.contains('1'),
            "{}",
            findings[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// Write `n` runs of one model into the session store, so the
    /// population checks have something to be a population of.
    fn runs_in(
        home: &Path,
        model: &str,
        n: usize,
        stats: impl Fn(usize) -> crate::session::RunStats,
    ) {
        let dir = home.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..n {
            let session = crate::session::Session::create(
                &dir,
                crate::session::SessionMeta {
                    // The model rides in the id: two calls to this helper
                    // in one test must not collide, or the second silently
                    // rewrites the first's transcripts and the fixture stops
                    // describing what the test says it does.
                    id: format!("2026080{}T00000{i:03}-{model}", 1 + i % 9),
                    created_at: utc(NOW),
                    provider: "local".into(),
                    model: model.to_string(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            session
                .append(&crate::session::Record::Outcome(stats(i)))
                .unwrap();
        }
    }

    fn run_stats(
        calls: u32,
        errors: u32,
        ended_failed: bool,
        cause: crate::agent::StopCause,
    ) -> crate::session::RunStats {
        crate::session::RunStats {
            tool_calls: calls,
            tool_errors: errors,
            ended_on_failed_call: ended_failed,
            stop_cause: Some(cause),
            ..Default::default()
        }
    }

    #[test]
    fn a_model_that_keeps_finishing_over_failures_is_reported() {
        use crate::agent::StopCause;
        let home = home("runs-ended-on-failure");
        // A third of runs end over a failure; everything else is healthy.
        runs_in(&home, "tiny-local", 30, |i| {
            run_stats(6, 0, i % 3 == 0, StopCause::Completed)
        });

        let all = examine(&home, utc(NOW));
        let findings = of(&all, "runs");
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0].summary.contains("tiny-local"),
            "{}",
            findings[0].summary
        );
        assert!(
            findings[0].summary.contains("33%"),
            "{}",
            findings[0].summary
        );
        assert_eq!(
            findings[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "sessions", "health", "--days", "30"],
            "reading is the remedy; doctor never decides what to change"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_cancelled_run_is_not_the_harness_cutting_it_short() {
        use crate::agent::StopCause;
        // A person pressing Ctrl-C is the system working, and counting it
        // would make an attentive user look like a problem.
        let home = home("runs-interrupted");
        runs_in(&home, "tiny-local", 30, |_| {
            run_stats(6, 0, false, StopCause::Interrupted)
        });
        let findings = examine(&home, utc(NOW));
        assert!(of(&findings, "runs").is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_turn_ceiling_stopping_a_quarter_of_runs_is_a_finding() {
        use crate::agent::StopCause;
        let home = home("runs-max-turns");
        runs_in(&home, "tiny-local", 30, |_| {
            run_stats(6, 0, false, StopCause::MaxTurns)
        });
        let all = examine(&home, utc(NOW));
        let findings = of(&all, "runs");
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0].summary.contains("cut"),
            "{}",
            findings[0].summary
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_thin_sample_of_one_model_says_nothing_about_it() {
        use crate::agent::StopCause;
        // Every run terrible, and still silent: nineteen runs is not a
        // population, and unknown is not a finding.
        let home = home("runs-thin");
        runs_in(&home, "tiny-local", 19, |_| {
            run_stats(6, 6, true, StopCause::MaxTurns)
        });
        let all = examine(&home, utc(NOW));
        assert!(of(&all, "runs").is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_bad_model_does_not_drag_a_good_one_into_a_finding() {
        use crate::agent::StopCause;
        // The reason rates split: blended, these two average to a rate that
        // describes neither, and a threshold on it names the wrong model.
        let home = home("runs-two-models");
        runs_in(&home, "steady", 25, |_| {
            run_stats(10, 0, false, StopCause::Completed)
        });
        runs_in(&home, "flaky", 25, |_| {
            run_stats(10, 9, false, StopCause::Completed)
        });

        let all = examine(&home, utc(NOW));
        let findings = of(&all, "runs");
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(
            findings[0].summary.contains("flaky"),
            "{}",
            findings[0].summary
        );
        assert!(
            !findings[0].summary.contains("steady"),
            "the healthy model was named in a finding about the other one"
        );
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

    #[test]
    fn a_malformed_charter_is_broken_and_names_the_remedy() {
        let home = home("charter-broken");
        std::fs::write(
            home.join("charter.toml"),
            "[[line]]\nid = \"a\"\ntext = \"one\"\n[[line]]\nid = \"a\"\ntext = \"two\"\n",
        )
        .unwrap();

        let findings = check_charter(&home.join("charter.toml"));
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Broken);
        assert!(
            findings[0].detail.contains("used more than once"),
            "{}",
            findings[0].detail
        );
        assert_eq!(
            findings[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "charter"]
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_charter_over_budget_is_attention_not_broken_and_still_named_loaded() {
        let home = home("charter-over-budget");
        let long = "x".repeat(3000);
        std::fs::write(
            home.join("charter.toml"),
            format!("[[line]]\nid = \"only\"\ntext = \"{long}\"\n"),
        )
        .unwrap();

        let findings = check_charter(&home.join("charter.toml"));
        assert_eq!(findings.len(), 1, "{findings:#?}");
        // Attention, not Broken: the document is valid and still loads in
        // full — it only costs more of the prefix than argued.
        assert_eq!(findings[0].severity, Severity::Attention);
        assert!(
            findings[0].summary.contains("budget"),
            "{}",
            findings[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_healthy_charter_and_a_missing_one_are_both_silent() {
        let home = home("charter-healthy");
        assert!(
            check_charter(&home.join("charter.toml")).is_empty(),
            "no file at all"
        );

        std::fs::write(
            home.join("charter.toml"),
            "[[line]]\nid = \"a\"\ntext = \"protect the owner\"\n",
        )
        .unwrap();
        assert!(check_charter(&home.join("charter.toml")).is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_genuinely_empty_charter_file_is_flagged_not_silent() {
        // A file that exists and parses cleanly (a comment, or nothing at
        // all) to zero `[[line]]` entries — as opposed to a typo'd table
        // name, which `RawCharter`'s `deny_unknown_fields` now turns into a
        // load error instead, covered by the next test.
        let home = home("charter-empty-comment");
        std::fs::write(home.join("charter.toml"), "# no priorities written yet\n").unwrap();

        let findings = check_charter(&home.join("charter.toml"));
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Attention);
        assert!(
            findings[0].summary.contains("no lines"),
            "{}",
            findings[0].summary
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_directory_at_the_charter_path_is_broken_not_silently_absent() {
        // `is_file()` would read this as "nothing written yet" and stay
        // silent; `exists()` lets it reach `Charter::load`, whose
        // `read_to_string` fails on a directory with a real I/O error rather
        // than `NotFound`.
        let home = home("charter-is-a-directory");
        std::fs::create_dir_all(home.join("charter.toml")).unwrap();

        let findings = check_charter(&home.join("charter.toml"));
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Broken);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_typo_d_table_name_beside_a_real_line_is_broken_not_silently_short() {
        let home = home("charter-typo-table");
        std::fs::write(
            home.join("charter.toml"),
            "[[line]]\nid = \"a\"\ntext = \"one\"\n\n[[lines]]\nid = \"b\"\ntext = \"two\"\n",
        )
        .unwrap();

        let findings = check_charter(&home.join("charter.toml"));
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, Severity::Broken);

        let _ = std::fs::remove_dir_all(&home);
    }
    // --- the owner's setpoints (§11.1's readings phase) ---------------------

    fn charter_with(dir: &Path, sensor_toml: &str) {
        std::fs::write(
            dir.join("charter.toml"),
            format!(
                "[[line]]\nid = \"waits\"\ntext = \"Keep what waits on me short.\"\n{sensor_toml}"
            ),
        )
        .unwrap();
    }

    /// Where a charter line names a setpoint the doctor reads against the
    /// owner's number and says which line; without one, the harness's 48h
    /// stands — a 20-hour-old draft is stuck under the first and fine under
    /// the second, which is what makes this fail on the old behaviour.
    #[test]
    fn a_stuck_draft_is_judged_against_the_owners_setpoint_where_a_line_names_one() {
        let dir = home("outbox-owner-setpoint");
        pending_item(&dir, "20260813-160000-aaa", "2026-08-13T16:00:00Z", None);

        let none = examine(&dir, utc(NOW));
        assert!(of(&none, "outbox").is_empty(), "{none:#?}");

        charter_with(
            &dir,
            "[line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"12h\"\n",
        );
        let findings = examine(&dir, utc(NOW));
        let outbox = of(&findings, "outbox");
        assert_eq!(outbox.len(), 1, "{findings:#?}");
        assert_eq!(
            outbox[0].summary,
            "1 draft pending for more than 12h (charter line `waits`)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same rule on the other two stores: the question and the request
    /// walkers read the owner's `question_latency` and `request_closure`.
    #[test]
    fn questions_and_requests_read_the_owners_setpoints_too() {
        let dir = home("owner-setpoints-q-r");
        // Six hours old: within the harness's 24h and 72h, past the owner's 1h.
        question(&dir, "q-1", "2026-08-14T06:00:00Z", crate::questions::OPEN);
        request(&dir, 7, "extracted", "2026-08-14T06:00:00Z");
        let none = examine(&dir, utc(NOW));
        assert!(of(&none, "questions").is_empty(), "{none:#?}");
        assert!(of(&none, "frontdoor").is_empty(), "{none:#?}");

        std::fs::write(
            dir.join("charter.toml"),
            "[[line]]\nid = \"answer\"\ntext = \"Answer fast.\"\n[line.sensor]\nkind = \"question_latency\"\nsetpoint = \"1h\"\n\n\
             [[line]]\nid = \"close\"\ntext = \"Close requests.\"\n[line.sensor]\nkind = \"request_closure\"\nsetpoint = \"1h\"\n",
        )
        .unwrap();
        let findings = examine(&dir, utc(NOW));
        let q = of(&findings, "questions");
        assert_eq!(q.len(), 1, "{findings:#?}");
        assert!(
            q[0].summary
                .contains("more than 1h (charter line `answer`)"),
            "{}",
            q[0].summary
        );
        let r = of(&findings, "frontdoor");
        assert_eq!(r.len(), 1, "{findings:#?}");
        assert!(
            r[0].summary.contains("more than 1h (charter line `close`)"),
            "{}",
            r[0].summary
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// How many drafts may wait has no harness constant behind it, so the
    /// count finding exists only where the owner wrote a number.
    #[test]
    fn a_count_setpoint_fires_only_where_the_charter_names_one() {
        let dir = home("outbox-count-setpoint");
        pending_item(&dir, "20260814-110000-aaa", "2026-08-14T11:00:00Z", None);
        pending_item(&dir, "20260814-110000-bbb", "2026-08-14T11:00:00Z", None);
        assert!(of(&examine(&dir, utc(NOW)), "outbox").is_empty());

        charter_with(
            &dir,
            "[line.sensor]\nkind = \"outbox_waiting\"\nsetpoint = 1\n",
        );
        let findings = examine(&dir, utc(NOW));
        let outbox = of(&findings, "outbox");
        assert_eq!(outbox.len(), 1, "{findings:#?}");
        assert_eq!(
            outbox[0].summary,
            "2 drafts pending, past the 1 setpoint on charter line `waits`"
        );
        assert_eq!(outbox[0].severity, Severity::Attention);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One session per run, newest last, each recording the given readings.
    fn runs_reading(dir: &Path, readings: Vec<Option<crate::reading::Reading>>) {
        use crate::session::{Record, RunStats, Session, SessionMeta};
        for (i, reading) in readings.into_iter().enumerate() {
            let stamp = format!("2026-08-01T00:{:02}:00Z", i);
            let s = Session::create(
                dir,
                SessionMeta {
                    id: format!("20260801T00{i:02}00-r"),
                    created_at: utc(&stamp),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            let charter = reading.map(|reading| {
                vec![crate::reading::LineReading {
                    line: "waits".into(),
                    kind: crate::charter::SensorKind::OutboxAge,
                    setpoint: "24h".into(),
                    reading,
                }]
            });
            s.append(&Record::Outcome(RunStats {
                homeostat: Some(crate::homeostat::Homeostat {
                    charter,
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .unwrap();
        }
    }

    fn over() -> crate::reading::Reading {
        crate::reading::Reading::Observed {
            value: crate::reading::Observed::Seconds(200_000),
            over: true,
            excess: 0.5,
        }
    }

    /// Containment 5's second guard: a sensor past its setpoint on each of
    /// the last ten recorded runs is a finding. Nine is not; a row that
    /// read nothing is skipped rather than counted on either side; and a
    /// reading against a different setpoint spelling starts a fresh streak,
    /// because the record kept the setpoint for exactly this.
    #[test]
    fn a_sensor_past_its_setpoint_on_ten_consecutive_runs_is_saturated() {
        use crate::reading::{Reading, SATURATED_AFTER_RUNS};
        let sensor = "[line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n";

        let dir = home("saturated-ten");
        charter_with(&dir, sensor);
        let mut readings: Vec<Option<Reading>> = vec![Some(over()); SATURATED_AFTER_RUNS];
        // Unread rows in the middle say nothing either way.
        readings.insert(3, Some(Reading::Unread));
        readings.insert(5, None);
        runs_reading(&dir.join("sessions"), readings);
        let findings = examine(&dir, utc(NOW));
        let charter = of(&findings, "charter");
        assert_eq!(charter.len(), 1, "{findings:#?}");
        assert_eq!(
            charter[0].summary,
            format!(
                "charter line `waits` has read past its 24h setpoint on each of the last {} runs",
                SATURATED_AFTER_RUNS
            )
        );
        assert_eq!(
            charter[0].remedy.as_ref().unwrap().argv,
            vec!["mecha", "charter"]
        );
        let _ = std::fs::remove_dir_all(&dir);

        let dir = home("saturated-nine");
        charter_with(&dir, sensor);
        runs_reading(
            &dir.join("sessions"),
            vec![Some(over()); SATURATED_AFTER_RUNS - 1],
        );
        assert!(of(&examine(&dir, utc(NOW)), "charter").is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // The newest run read nothing waiting: the streak is broken there.
        let dir = home("saturated-met");
        charter_with(&dir, sensor);
        let mut readings: Vec<Option<Reading>> = vec![Some(over()); SATURATED_AFTER_RUNS];
        readings.push(Some(Reading::Nothing));
        runs_reading(&dir.join("sessions"), readings);
        assert!(of(&examine(&dir, utc(NOW)), "charter").is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // The owner has since changed the setpoint: the recorded streak was
        // against `24h`, the charter now says `48h`, nothing matches.
        let dir = home("saturated-resp");
        charter_with(
            &dir,
            "[line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"48h\"\n",
        );
        runs_reading(
            &dir.join("sessions"),
            vec![Some(over()); SATURATED_AFTER_RUNS],
        );
        assert!(of(&examine(&dir, utc(NOW)), "charter").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The corpus kind, compared against the owner's rate over the run
    /// check's own scan — and silent under the run floor, like every rate.
    #[test]
    fn the_intervention_rate_is_read_off_the_corpus_against_the_owners_share() {
        use crate::agent::StopCause;
        use crate::session::{Record, RunStats, Session, SessionMeta};
        let write = |dir: &Path, n: usize| {
            for i in 0..n {
                let s = Session::create(
                    &dir.join("sessions"),
                    SessionMeta {
                        id: format!("20260801T00{i:02}00-r"),
                        created_at: utc(&format!("2026-08-01T00:{i:02}:00Z")),
                        provider: "local".into(),
                        model: "m".into(),
                        workspace: std::path::PathBuf::from("/tmp"),
                        title: None,
                        kind: None,
                    },
                )
                .unwrap();
                // Every other run was stopped by request: a 50% share.
                let cause = if i % 2 == 0 {
                    StopCause::Stopped
                } else {
                    StopCause::Completed
                };
                s.append(&Record::Outcome(RunStats {
                    stop_cause: Some(cause),
                    ..Default::default()
                }))
                .unwrap();
            }
        };
        let sensor = "[line.sensor]\nkind = \"intervention_rate\"\nsetpoint = \"20%\"\n";

        let dir = home("intervention-rate");
        charter_with(&dir, sensor);
        write(&dir, RUNS_MIN);
        let findings = examine(&dir, utc(NOW));
        let runs: Vec<_> = of(&findings, "runs")
            .into_iter()
            .filter(|f| f.summary.contains("stepped into"))
            .collect();
        assert_eq!(runs.len(), 1, "{findings:#?}");
        assert_eq!(
            runs[0].summary,
            format!(
                "you stepped into 50% of the last {RUNS_MIN} runs, past the 20% setpoint on charter line `waits`"
            )
        );
        let _ = std::fs::remove_dir_all(&dir);

        // Under the floor: a share of a handful is noise.
        let dir = home("intervention-rate-few");
        charter_with(&dir, sensor);
        write(&dir, RUNS_MIN - 1);
        assert!(!of(&examine(&dir, utc(NOW)), "runs")
            .iter()
            .any(|f| f.summary.contains("stepped into")));
        let _ = std::fs::remove_dir_all(&dir);

        // No charter line: the rate is nobody's number, and nothing fires.
        let dir = home("intervention-rate-no-line");
        write(&dir, RUNS_MIN);
        assert!(!of(&examine(&dir, utc(NOW)), "runs")
            .iter()
            .any(|f| f.summary.contains("stepped into")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod proposal_review_tests {
    use super::*;

    fn store_at(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("mecha-doctor-proposals")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proposals")).unwrap();
        dir
    }

    fn write_proposal(root: &Path, id: &str, created: &str, before: &[&str], reflexions: usize) {
        let p = serde_json::json!({
            "id": id,
            "domain": "behavior",
            "status": "pending",
            "reflexion_ids": (0..reflexions).map(|i| format!("r-{i}")).collect::<Vec<_>>(),
            "rules_before": before.iter().map(|t| serde_json::json!({"text": t})).collect::<Vec<_>>(),
            "rules": [{"text": "a new rule"}],
            "evidence": "e",
            "created_at": created,
            "resolved_at": null,
            "reason": null,
        });
        std::fs::write(
            root.join("proposals").join(format!("{id}.json")),
            serde_json::to_string(&p).unwrap(),
        )
        .unwrap();
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// **Doctor's staleness test must be `proposals accept`'s**, or it tells
    /// the owner to supersede a proposal that would have applied fine.
    ///
    /// Fails on the old behaviour: that compared an order-insensitive set of
    /// *active* rule texts, so a rewrite that reordered the same rules read as
    /// unchanged here and as changed by `accept`, and a rule merely disabled
    /// read as unchanged here and as changed there.
    #[test]
    fn the_stale_predicate_matches_accepts() {
        let r = |text: &str, enabled: bool| crate::learning::Rule {
            text: text.into(),
            enabled,
            ..Default::default()
        };
        let base = vec![r("alpha", true), r("beta", true)];

        assert!(same_rules_as_accept(&base, &base.clone()));
        // Order is part of it — a set comparison would call this unchanged.
        assert!(!same_rules_as_accept(
            &base,
            &[r("beta", true), r("alpha", true)]
        ));
        // So is `enabled` — an active-only filter would drop the disabled one
        // from both sides and call these equal.
        assert!(!same_rules_as_accept(
            &base,
            &[r("alpha", true), r("beta", false)]
        ));
        assert!(!same_rules_as_accept(&base, &[r("alpha", true)]));
    }

    /// **Doctor must not bring a learning store into being.** Reporting on a
    /// machine that has never learned anything must leave it looking like a
    /// machine that has never learned anything.
    #[test]
    fn checking_an_absent_store_creates_nothing() {
        let root =
            std::env::temp_dir().join(format!("mecha-doctor-nostore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // An absent store reads as an empty rule set — a fact — rather than
        // as unknown, and reading it creates nothing.
        assert!(read_learned_rules(&root, "behavior").is_some_and(|r| r.is_empty()));
        assert!(
            !root.exists(),
            "reading rules must not create the store directory"
        );
    }

    /// **A queue nobody has answered is a finding.** Before this, review
    /// latency was invisible: the starved-learner check next door measures
    /// origin exclusion, so four proposals sitting six days while `learn`
    /// skipped every night read exactly like a healthy loop.
    #[test]
    fn an_unreviewed_queue_is_reported_with_what_it_is_holding() {
        let root = store_at("stale");
        write_proposal(&root, "p-old", "2026-08-23T12:00:00Z", &[], 10);

        let out = check_proposal_review(&root, now());
        assert_eq!(out.len(), 1, "one latency finding: {out:?}");
        assert!(out[0].summary.contains("6 day(s)"), "{}", out[0].summary);
        assert!(
            out[0].summary.contains("holding 10 reflection(s)"),
            "the cost is the held evidence, not the count of proposals: {}",
            out[0].summary
        );
    }

    /// A proposal staged last night is ordinary, not a finding — or the check
    /// fires every morning and trains the reader to skip the component.
    #[test]
    fn a_fresh_proposal_is_not_a_finding() {
        let root = store_at("fresh");
        write_proposal(&root, "p-new", "2026-08-29T03:30:00Z", &[], 4);
        assert!(check_proposal_review(&root, now()).is_empty());
    }

    /// A proposal measured against rules that have since moved is not a
    /// decision waiting on anyone — `accept` would refuse it. Reported
    /// separately, because its remedy is exact where latency's is a judgement.
    #[test]
    fn an_unappliable_proposal_is_named_with_the_verb_that_frees_it() {
        let root = store_at("unappliable");
        // Live rules are empty (no rules file), so a proposal measured against
        // a non-empty baseline can no longer be applied.
        write_proposal(&root, "p-stale", "2026-08-29T03:30:00Z", &["was live"], 7);

        let out = check_proposal_review(&root, now());
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].summary.contains("can no longer be applied"));
        assert!(out[0].detail.contains("p-stale"));
        // Supersede, never reject: rejecting marks the reflections processed
        // and loses corrections the owner never ruled on.
        let argv = &out[0].remedy.as_ref().unwrap().argv;
        assert!(argv.contains(&"supersede".to_string()), "{argv:?}");
        assert!(!argv.contains(&"reject".to_string()), "{argv:?}");
    }

    /// No proposals directory is a young install, not a fault.
    #[test]
    fn a_store_that_has_never_staged_one_is_silent() {
        let dir = std::env::temp_dir().join("mecha-doctor-proposals-absent");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(check_proposal_review(&dir, now()).is_empty());
    }

    #[test]
    fn a_window_dominated_by_smoke_tests_is_a_finding_not_a_healthy_store() {
        use crate::session::{Record, RunStats, Session, SessionKind, SessionMeta};
        let dir = std::env::temp_dir().join(format!("doctor-hidden-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |id: &str, kind: SessionKind, stamp: &str| {
            let s = Session::create(
                &dir,
                SessionMeta {
                    id: id.into(),
                    created_at: chrono::DateTime::parse_from_rfc3339(stamp)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: Some(kind),
                },
            )
            .unwrap();
            s.append(&Record::Outcome(RunStats::default())).unwrap();
        };
        mk(
            "20260801T000003-web",
            SessionKind::Web,
            "2026-08-01T00:00:03Z",
        );
        mk(
            "20260801T000002-t1",
            SessionKind::Test,
            "2026-08-01T00:00:02Z",
        );
        mk(
            "20260801T000001-t2",
            SessionKind::Test,
            "2026-08-01T00:00:01Z",
        );
        let findings = check_runs(&dir, None);
        assert!(
            findings
                .iter()
                .any(|f| f.summary.contains("2 smoke-test session(s) it hid")),
            "{findings:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
