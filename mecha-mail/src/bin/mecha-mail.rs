//! `mecha-mail` — every configured mail/calendar account behind one
//! provider-neutral MCP surface. Default mode serves MCP over stdio;
//! the subcommands manage the account registry (`~/.mecha/mail/`).

use anyhow::{bail, Context, Result};
use clap::Parser;
use mecha_mail::accounts::{self, AccountEntry, Provider};
use mecha_mail::freebusy::{classify_partial, PartialCoverage};
use mecha_mail::unified::MailTools;
use mecha_mail::{google, mcp, microsoft, token};

/// The exit code for a **permanent** credential failure — a refresh token the
/// provider says is expired or revoked, which no retry ever fixes. Distinct
/// from the generic 1 so a systemd unit or a script can alert "re-auth
/// needed" instead of blind-retrying: the 2026-08-11 revocation failed a
/// two-minute timer identically for three days because nothing downstream
/// could tell this class from transient weather. 77 is sysexits' EX_NOPERM.
const EXIT_AUTH_REVOKED: u8 = 77;

#[derive(Parser, Debug)]
#[command(
    name = "mecha-mail",
    about = "All configured mail and calendar accounts as one MCP tool surface",
    after_help = "Exit codes: 77 means a stored refresh token is expired or revoked \
                  (permanent — run `mecha-mail auth <account> --provider <provider>`); \
                  `mecha-mail accounts` names the dead account."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Sign an account in and add it to the registry.
    Auth {
        /// Account name the model will use (lowercase, digits, - and _).
        name: String,
        /// google (loopback OAuth) or outlook (device code).
        #[arg(long)]
        provider: Provider,
        /// OAuth client id (Google Desktop app / Entra application id).
        /// Env: GMAIL_CLIENT_ID / OUTLOOK_CLIENT_ID per provider. Without
        /// one, the whole client config is taken from this account's stored
        /// login, else from a configured sibling of the same provider — a
        /// second mailbox on the same app registration needs no flags.
        #[arg(long)]
        client_id: Option<String>,
        /// The Google Desktop client's pseudo-secret. Google only.
        /// Env: GMAIL_CLIENT_SECRET.
        #[arg(long)]
        client_secret: Option<String>,
        /// Entra directory (tenant) id. Outlook only.
        /// Env: OUTLOOK_TENANT_ID.
        #[arg(long)]
        tenant: Option<String>,
        /// Loopback port for the Google OAuth redirect.
        #[arg(long, default_value_t = google::auth::DEFAULT_REDIRECT_PORT)]
        port: u16,
    },
    /// Copy an existing mecha-google / mecha-outlook login into the registry.
    Import {
        /// Account name to register it under.
        name: String,
        /// Which legacy store to copy: google (~/.mecha/google) or outlook
        /// (~/.mecha/outlook).
        #[arg(long)]
        provider: Provider,
    },
    /// List configured accounts.
    Accounts,
    /// Set (or with no argument, show) the default account for new mail and
    /// events sent without an explicit account.
    Default { name: Option<String> },
    /// Merged busy intervals across every account, as data. Built for the
    /// slot-refresh pipeline (`mecha-mail freebusy --json | …`), which is a
    /// scheduled command with no model in it — so unlike the MCP surface,
    /// this fails when any account is *transiently* unreadable: a mailbox
    /// that could not be read is not a mailbox with free time, and a booking
    /// page built from a partial answer offers strangers slots the user does
    /// not have. A permanently *revoked* login is the one exception —
    /// already alarmed via its auth_error.json marker, `mecha doctor` and
    /// exit 77, and never fixed by waiting — so its calendar is skipped with
    /// a loud stderr warning instead of halting slot publishing for days.
    Freebusy {
        /// Days ahead to query, starting now.
        #[arg(long, default_value_t = 60)]
        days: u32,
        /// Window start, RFC 3339 (with --to, replaces --days).
        #[arg(long, requires = "to")]
        from: Option<String>,
        /// Window end, RFC 3339.
        #[arg(long, requires = "from")]
        to: Option<String>,
        /// Query one account instead of all of them.
        #[arg(long)]
        account: Option<String>,
        /// Machine-readable output (the pipeline contract).
        #[arg(long)]
        json: bool,
    },
    /// Turn drained booking records into calendar events. The inbound
    /// sibling of `freebusy`: deterministic, no model, run by the drain
    /// timer. Idempotent against `~/.mecha/mail/bookings.jsonl` — a record
    /// already ledgered is skipped, so re-running after a partial failure
    /// picks up exactly where it stopped.
    Bookings {
        /// The drained-request store. Defaults to `~/.mecha/requests`.
        #[arg(long)]
        requests: Option<std::path::PathBuf>,
        /// The account whose calendar gets the events. Defaults to the
        /// configured default account.
        #[arg(long)]
        account: Option<String>,
        /// Report what would be created, creating nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Serve MCP over stdio (the default when no subcommand is given).
    Serve,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    mecha_mail::init_tracing();
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::ExitCode::from(exit_code_for(&e))
        }
    }
}

/// Permanent credential death exits distinctly; everything else is a 1.
/// Keyed on the sentinel because by the time an error reaches here it has
/// crossed the fan-out's string boundary — the typed variant does not
/// survive, the phrase does, and a test in `types.rs` pins the two together.
fn exit_code_for(e: &anyhow::Error) -> u8 {
    if format!("{e:#}").contains(mecha_mail::types::AUTH_REVOKED) {
        EXIT_AUTH_REVOKED
    } else {
        1
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Auth {
            name,
            provider,
            client_id,
            client_secret,
            tenant,
            port,
        }) => auth(name, provider, client_id, client_secret, tenant, port).await,
        Some(Command::Import { name, provider }) => import(name, provider),
        Some(Command::Accounts) => list_accounts(),
        Some(Command::Default { name }) => set_default(name),
        Some(Command::Freebusy {
            days,
            from,
            to,
            account,
            json,
        }) => freebusy(days, from, to, account, json).await,
        Some(Command::Bookings {
            requests,
            account,
            dry_run,
        }) => bookings(requests, account, dry_run).await,
        Some(Command::Serve) | None => mcp::serve(MailTools::load()?).await,
    }
}

/// The freebusy scope for a booking's re-verify: every account, regardless
/// of which one the event lands on. A slot free on the landing calendar but
/// busy on another is exactly the collision the re-verify exists to catch,
/// so the answer is `None` (fan out) for any input — the parameter exists to
/// make that contract testable against the recorded regression, where this
/// decision returned the event's account and cross-account collisions
/// double-booked silently.
fn reverify_scope(_event_account: Option<&str>) -> Option<&str> {
    None
}

async fn bookings(
    requests: Option<std::path::PathBuf>,
    account: Option<String>,
    dry_run: bool,
) -> Result<()> {
    use mecha_mail::bookings as bk;

    let requests = match requests {
        Some(dir) => dir,
        None => dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".mecha")
            .join("requests"),
    };
    if !requests.is_dir() {
        // An absent store is "nothing drained yet", not a broken setup —
        // this runs on a timer that must not cry wolf.
        println!("no request store at {}; nothing to do", requests.display());
        return Ok(());
    }
    let ledger = bk::ledger_path()?;
    // One sweep at a time, and the lock comes before the ledger read it
    // protects: the drain loop and the fifteen-minute timer both run this,
    // and two readers who each saw "not handled" would each create the
    // event. Blocking, not try: the other sweep finishes in seconds and
    // whoever waited picks up whatever it did not.
    let _sweep = bk::lock_sweep(&ledger)?;
    let done = bk::handled(&ledger);
    let waiting: Vec<_> = bk::scan(&requests)?
        .into_iter()
        .filter(|b| !done.contains(&b.booking_id))
        .collect();

    if dry_run {
        println!("{} booking(s) would get events:", waiting.len());
        for booking in &waiting {
            let (title, _) = bk::event_text(booking);
            println!("  #{:<4} {}  {}", booking.seq, booking.start, title);
        }
        let cancels = bk::scan_cancellations(&requests)?
            .into_iter()
            .filter(|(_, id)| !bk::cancelled(&ledger).contains(id))
            .count();
        let reminders = bk::reminders_due(
            &bk::scan(&requests)?,
            &bk::entries(&ledger),
            chrono::Utc::now(),
        );
        println!(
            "{cancels} cancellation(s) pending, {} reminder(s) due",
            reminders.len()
        );
        return Ok(());
    }

    let tools = MailTools::load()?;
    if waiting.is_empty() {
        println!("no new bookings");
    }
    // Resolve where the events will land once, before any is created — and
    // only when something is waiting, so a sweep with nothing to create
    // never fails over an unset default.
    let event_account = match waiting.is_empty() {
        true => None,
        false => Some(
            tools
                .create_account_name(account.as_deref())
                .map_err(|e| anyhow::anyhow!(e))
                .context("resolving the account for booking events")?,
        ),
    };
    let mut created = 0usize;
    for booking in &waiting {
        // Re-verify against *live* freebusy before creating anything: the
        // box sold this slot from a cache up to minutes old, and home
        // always holds fresher truth. An event that landed directly on a
        // calendar meanwhile makes this a collision — parked loudly for a
        // human, never silently double-booked. Fail-closed like the slot
        // pipeline: an unreadable calendar is never a free one.
        //
        // A fan-out over EVERY account, never just the one the event lands
        // on: a slot free on the landing calendar but busy on another is
        // exactly the collision this check exists to catch, and scoping the
        // read to one account silently dropped that. The dead-token incident
        // the scoping once answered (one revoked refresh token blocked every
        // booking for three days) is answered by classification instead: a
        // failure carrying the AUTH_REVOKED sentinel is permanent — the
        // human is already alerted via the marker, doctor and exit 77 — so
        // that account's calendar is skipped loudly and the booking
        // proceeds on the rest. Any other failure stays fail-closed and
        // defers the booking: transient errors recover, revoked ones never
        // do.
        let (start, end) = (
            chrono::DateTime::parse_from_rfc3339(&booking.start)
                .with_context(|| format!("booking {} start", booking.booking_id))?
                .with_timezone(&chrono::Utc),
            chrono::DateTime::parse_from_rfc3339(&booking.end)
                .with_context(|| format!("booking {} end", booking.booking_id))?
                .with_timezone(&chrono::Utc),
        );
        let (busy, failures) = tools
            .freebusy(
                &booking.start,
                &booking.end,
                reverify_scope(event_account.as_deref()),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("re-verifying booking {}", booking.booking_id))?;
        // `freebusy` returned Ok, so at least one account was readable — an
        // all-failed fan-out is its Err path, which `?` above propagates and
        // the sentinel-keyed exit code turns into 77 when every login is
        // revoked.
        match classify_partial(true, &failures) {
            PartialCoverage::SkipRevoked(revoked) => {
                for failure in &revoked {
                    eprintln!(
                        "WARNING: {failure} — permanently revoked login; skipping that \
                         account's calendar in booking {}'s collision check until it is \
                         re-authenticated",
                        booking.booking_id
                    );
                }
            }
            PartialCoverage::Defer(failures) | PartialCoverage::AllRevoked(failures) => {
                anyhow::bail!(
                    "refusing to create an event for booking {} without full freebusy:\n{}",
                    booking.booking_id,
                    failures.join("\n")
                );
            }
        }
        if bk::busy_overlaps(&busy, start, end) {
            bk::append(
                &ledger,
                &bk::LedgerEntry {
                    booking_id: booking.booking_id.clone(),
                    event_id: String::new(),
                    account: String::new(),
                    seq: booking.seq,
                    created_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    action: "conflict".into(),
                },
            )?;
            eprintln!(
                "CONFLICT: booking #{} ({}, {} – {}) overlaps something now on the \
                 calendar — no event created, no invite sent. Resolve by hand: \
                 the visitor has a confirmation page but no invite.",
                booking.seq, booking.name, booking.start, booking.end
            );
            continue;
        }
        let (title, description) = bk::event_text(booking);
        // The resolved name, not the raw flag: one resolution, made before
        // the loop, decides where every event lands.
        let (account_name, event_id) = tools
            .create_event_invite(
                event_account.as_deref(),
                &title,
                &description,
                &booking.start,
                &booking.end,
                booking.email.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))
            // Named, because the fix differs per booking: the ledger holds
            // what already succeeded, and a re-run resumes here.
            .with_context(|| {
                format!("booking {} (request #{})", booking.booking_id, booking.seq)
            })?;
        bk::append(
            &ledger,
            &bk::LedgerEntry {
                booking_id: booking.booking_id.clone(),
                event_id: event_id.clone(),
                account: account_name.clone(),
                seq: booking.seq,
                created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                action: "created".into(),
            },
        )?;
        created += 1;
        println!(
            "✓ #{} {} → event {event_id} on `{account_name}`",
            booking.seq, title
        );
    }
    if created > 0 {
        println!("{created} event(s) created");
    }
    cancel_drained(&tools, &requests, &ledger).await?;
    remind_due(&tools, &requests, &ledger).await
}

/// The other direction: cancellation records become event deletions,
/// through the ledger's booking→event join, with the provider mailing the
/// retraction. A cancellation whose event was never created (the create
/// failed and was never retried, or predates the ledger) is closed with an
/// empty event id rather than retried forever — the calendar holds nothing
/// to remove, and the record said so.
async fn cancel_drained(
    tools: &MailTools,
    requests: &std::path::Path,
    ledger: &std::path::Path,
) -> Result<()> {
    use mecha_mail::bookings as bk;

    let done = bk::cancelled(ledger);
    let created: std::collections::HashMap<String, bk::LedgerEntry> = bk::entries(ledger)
        .into_iter()
        .filter(|e| e.action == "created")
        .map(|e| (e.booking_id.clone(), e))
        .collect();
    let waiting: Vec<(i64, String)> = bk::scan_cancellations(requests)?
        .into_iter()
        .filter(|(_, id)| !done.contains(id))
        .collect();
    if waiting.is_empty() {
        return Ok(());
    }
    let mut removed = 0usize;
    for (seq, booking_id) in &waiting {
        let entry = created.get(booking_id);
        if let Some(entry) = entry {
            tools
                .delete_event_quiet(&entry.account, &entry.event_id)
                .await
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("cancelling booking {booking_id}"))?;
            removed += 1;
            println!(
                "✗ #{seq} booking {booking_id} → event {} withdrawn from `{}`",
                entry.event_id, entry.account
            );
        } else {
            println!("· #{seq} booking {booking_id} cancelled, but no event was ever created");
        }
        bk::append(
            ledger,
            &bk::LedgerEntry {
                booking_id: booking_id.clone(),
                event_id: entry.map(|e| e.event_id.clone()).unwrap_or_default(),
                account: entry.map(|e| e.account.clone()).unwrap_or_default(),
                seq: *seq,
                created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                action: "cancelled".into(),
            },
        )?;
    }
    if removed > 0 {
        println!("{removed} event(s) withdrawn");
    }
    Ok(())
}

/// Reminders, from the same sweep: templated, from the user's own account
/// (the one that made the event), each tier fired once and remembered in
/// the ledger. Runs on the same timers as everything else here — the
/// 15-minute slot refresh gives the 1-hour tier its resolution.
async fn remind_due(
    tools: &MailTools,
    requests: &std::path::Path,
    ledger: &std::path::Path,
) -> Result<()> {
    use mecha_mail::bookings as bk;

    let entries = bk::entries(ledger);
    let bookings = bk::scan(requests)?;
    let due = bk::reminders_due(&bookings, &entries, chrono::Utc::now());
    if due.is_empty() {
        return Ok(());
    }
    let account_of: std::collections::HashMap<&str, &bk::LedgerEntry> = entries
        .iter()
        .filter(|e| e.action == "created")
        .map(|e| (e.booking_id.as_str(), e))
        .collect();
    let tz = mecha_mail::time::configured_zone();
    for (booking, action) in &due {
        let Some(entry) = account_of.get(booking.booking_id.as_str()) else {
            continue;
        };
        let Some(to) = booking.email.as_deref() else {
            continue;
        };
        let when = mecha_mail::time::in_zone(&booking.start, tz);
        let manage = booking
            .manage_url
            .as_deref()
            .map(|url| format!("\n\nNeed to change or cancel? {url}"))
            .unwrap_or_default();
        tools
            .send_mail_quiet(
                &entry.account,
                to,
                &format!("Reminder: your meeting on {when}"),
                &format!("A reminder that your meeting is coming up: **{when}**.{manage}"),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("reminding booking {}", booking.booking_id))?;
        bk::append(
            ledger,
            &bk::LedgerEntry {
                booking_id: booking.booking_id.clone(),
                event_id: entry.event_id.clone(),
                account: entry.account.clone(),
                seq: booking.seq,
                created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                action: (*action).to_string(),
            },
        )?;
        println!("⏰ #{} {action} → {to}", booking.seq);
    }
    Ok(())
}

async fn freebusy(
    days: u32,
    from: Option<String>,
    to: Option<String>,
    account: Option<String>,
    json: bool,
) -> Result<()> {
    let now = chrono::Utc::now();
    let stamp =
        |t: chrono::DateTime<chrono::Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (time_min, time_max) = match (from, to) {
        (Some(from), Some(to)) => (from, to),
        _ => (
            stamp(now),
            stamp(now + chrono::Duration::days(i64::from(days))),
        ),
    };

    let tools = MailTools::load()?;
    let (busy, failures) = tools
        .freebusy(&time_min, &time_max, account.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Fail closed on anything transient: an unreadable account is not a
    // free one, and `--account` scopes the query when partial coverage is
    // genuinely wanted. A permanently *revoked* login is classified out of
    // that rule — no amount of waiting recovers it, its outage is already
    // alarmed via the marker, doctor and exit 77, and refusing over it
    // halts slot publishing for days — so it is skipped with a loud stderr
    // warning instead (stdout keeps the pipeline contract). An all-revoked
    // registry never reaches here: the fan-out itself errors, and the
    // sentinel-keyed exit code makes that a 77.
    match classify_partial(true, &failures) {
        PartialCoverage::SkipRevoked(revoked) => {
            for failure in &revoked {
                eprintln!(
                    "WARNING: {failure} — permanently revoked login; its busy time is \
                     missing from this answer until the account is re-authenticated"
                );
            }
        }
        PartialCoverage::Defer(failures) | PartialCoverage::AllRevoked(failures) => {
            bail!(
                "refusing a partial answer — busy time would be missing:\n{}",
                failures.join("\n")
            );
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "generated_at": stamp(now),
                "time_min": time_min,
                "time_max": time_max,
                "busy": busy,
            }))?
        );
    } else {
        let tz = mecha_mail::time::configured_zone();
        println!("busy {time_min} → {time_max} ({} intervals)", busy.len());
        for iv in &busy {
            let render =
                |t: &chrono::DateTime<chrono::Utc>| mecha_mail::time::in_zone(&t.to_rfc3339(), tz);
            println!("  {} — {}", render(&iv.start), render(&iv.end));
        }
    }
    Ok(())
}

/// Load the registry, or start an empty one — `auth` and `import` are how
/// the file comes to exist.
fn load_or_empty() -> Result<accounts::AccountsFile> {
    match accounts::file_path()?.exists() {
        true => accounts::load(),
        false => Ok(accounts::AccountsFile::default()),
    }
}

/// Register `name` as `provider`, refusing to silently repurpose a name that
/// already means a different provider's mailbox.
fn register(file: &mut accounts::AccountsFile, name: &str, provider: Provider) -> Result<()> {
    if !accounts::valid_name(name) {
        bail!("account name `{name}` is invalid: lowercase letters, digits, `-` and `_` only");
    }
    match file.accounts.iter().find(|a| a.name == name) {
        Some(existing) if existing.provider != provider => bail!(
            "account `{name}` already exists as {} — pick another name or remove it first",
            existing.provider
        ),
        Some(_) => {}
        None => file.accounts.push(AccountEntry {
            name: name.to_string(),
            provider,
        }),
    }
    Ok(())
}

/// One coherent client registration. Resolved as a **unit** from a single
/// source — flags/env, this account's store, or one sibling account — never
/// assembled field-by-field across sources, because a client id paired with
/// a different registration's secret (or a Google id fed to Entra) fails
/// only after the user completes the whole browser flow.
struct ClientConfig {
    client_id: String,
    client_secret: String,
    tenant: Option<String>,
}

/// A stored login is usable as a client-config source for `provider` only
/// when it plausibly belongs to it: Entra logins record a tenant, Google
/// ones never do. Guards against a leftover oauth.json from a removed
/// account of the other provider being consumed silently.
fn consistent_with(creds: &token::StoredCredentials, provider: Provider) -> bool {
    match provider {
        Provider::Google => creds.tenant.is_none(),
        Provider::Outlook => creds.tenant.is_some(),
    }
}

fn resolve_client(
    provider: Provider,
    name: &str,
    file: &accounts::AccountsFile,
    client_id: Option<String>,
    client_secret: Option<String>,
    tenant: Option<String>,
) -> Result<ClientConfig> {
    // The legacy binaries' env names, per provider on purpose: one shared
    // variable would feed a Google client id to the Entra flow.
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let (id_env, secret_env, tenant_env) = match provider {
        Provider::Google => ("GMAIL_CLIENT_ID", Some("GMAIL_CLIENT_SECRET"), None),
        Provider::Outlook => ("OUTLOOK_CLIENT_ID", None, Some("OUTLOOK_TENANT_ID")),
    };

    let explicit_id = client_id.or_else(|| env(id_env));
    let explicit_secret = client_secret
        .or_else(|| secret_env.and_then(env))
        .unwrap_or_default();
    let explicit_tenant = tenant.or_else(|| tenant_env.and_then(env));

    if let Some(client_id) = explicit_id {
        let tenant = match provider {
            Provider::Outlook => Some(explicit_tenant.with_context(|| {
                format!("no tenant: pass --tenant or set {}", tenant_env.unwrap())
            })?),
            Provider::Google => None,
        };
        return Ok(ClientConfig {
            client_id,
            client_secret: explicit_secret,
            tenant,
        });
    }

    // No explicit id: take the whole registration from one stored login —
    // this account's own (re-auth needs no flags), else the first
    // provider-consistent sibling.
    let stored = accounts::credentials_path(name)
        .ok()
        .and_then(|p| token::load(&p).ok())
        .filter(|c| consistent_with(c, provider));
    let sibling = || {
        file.accounts
            .iter()
            .filter(|a| a.provider == provider && a.name != name)
            .find_map(|a| {
                accounts::credentials_path(&a.name)
                    .ok()
                    .and_then(|p| token::load(&p).ok())
                    .filter(|c| consistent_with(c, provider))
            })
    };
    let source = stored
        .or_else(sibling)
        .with_context(|| format!("no client id: pass --client-id or set {id_env}"))?;
    Ok(ClientConfig {
        client_id: source.client_id,
        client_secret: source.client_secret,
        tenant: source.tenant,
    })
}

async fn auth(
    name: String,
    provider: Provider,
    client_id: Option<String>,
    client_secret: Option<String>,
    tenant: Option<String>,
    port: u16,
) -> Result<()> {
    let mut file = load_or_empty()?;
    register(&mut file, &name, provider)?;
    let path = accounts::credentials_path(&name)?;
    let client = resolve_client(provider, &name, &file, client_id, client_secret, tenant)?;

    let creds = match provider {
        Provider::Google => {
            google::auth::interactive_flow(client.client_id, client.client_secret, port).await?
        }
        Provider::Outlook => {
            // resolve_client guarantees a tenant for Outlook.
            microsoft::auth::device_flow(client.client_id, client.tenant.unwrap()).await?
        }
    };

    let address = creds.account.clone();
    token::save(&path, &creds)?;
    accounts::save(&file)?;
    eprintln!(
        "\n✓ account `{name}` ({provider}) authenticated{}\n  credentials in {}",
        address.map(|a| format!(" as {a}")).unwrap_or_default(),
        path.display()
    );
    Ok(())
}

fn import(name: String, provider: Provider) -> Result<()> {
    let legacy = match provider {
        Provider::Google => token::provider_path("google", "MECHA_GOOGLE_DIR")?,
        Provider::Outlook => token::provider_path("outlook", "MECHA_OUTLOOK_DIR")?,
    };
    // Parse rather than copy bytes: a torn or foreign file should fail here,
    // not at first serve.
    let creds = token::load(&legacy)
        .with_context(|| format!("no importable {provider} login at {}", legacy.display()))?;

    let mut file = load_or_empty()?;
    register(&mut file, &name, provider)?;
    // Never overwrite a live login: `auth` re-authing in place is expected;
    // a byte-copy silently swapping which mailbox answers to `name` — and
    // destroying its working refresh token — is not.
    let existing = accounts::credentials_path(&name)?;
    if existing.exists() {
        bail!(
            "account `{name}` already has credentials at {} — pick another name, \
             or remove that file first if you mean to replace the login",
            existing.display()
        );
    }
    token::save(&accounts::credentials_path(&name)?, &creds)?;
    accounts::save(&file)?;
    eprintln!(
        "✓ imported {} as account `{name}`{}",
        legacy.display(),
        creds.account.map(|a| format!(" ({a})")).unwrap_or_default()
    );
    Ok(())
}

fn list_accounts() -> Result<()> {
    let file = accounts::load()?;
    for entry in &file.accounts {
        let path = accounts::credentials_path(&entry.name).ok();
        let address = path
            .as_deref()
            .and_then(|p| token::load(p).ok())
            .and_then(|c| c.account)
            .unwrap_or_else(|| "(no stored address)".into());
        let default = if file.default.as_deref() == Some(entry.name.as_str()) {
            "  [default]"
        } else {
            ""
        };
        println!(
            "{:<12} {:<8} {address}{default}",
            entry.name,
            entry.provider.to_string()
        );
        // A dead login is the first thing this listing exists to reveal:
        // the marker is written by the refresh path the moment a provider
        // says the token is revoked, and cleared by the next successful
        // refresh or re-auth — so this line costs no network call.
        if let Some(marker) = path.as_deref().and_then(token::load_auth_error) {
            println!("             !! DEAD since {}: {}", marker.at, marker.message);
        }
    }
    if file.default.is_none() {
        println!(
            "\nno default set — new mail/events need an explicit account \
             (set one: mecha-mail default <name>)"
        );
    }
    Ok(())
}

fn set_default(name: Option<String>) -> Result<()> {
    let mut file = accounts::load()?;
    match name {
        None => {
            match &file.default {
                Some(d) => println!("{d}"),
                None => println!("(none)"),
            }
            Ok(())
        }
        Some(name) => {
            if !file.accounts.iter().any(|a| a.name == name) {
                bail!(
                    "no account `{name}`; configured: {}",
                    file.accounts
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            file.default = Some(name.clone());
            accounts::save(&file)?;
            eprintln!("✓ default account: {name}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 2026-08-14 review finding: the re-verify had been scoped to the
    /// account the event lands on (to stop one dead token from blocking
    /// every booking), which silently dropped cross-account collision
    /// detection — a slot free on `dartmouth` but busy on `personal`
    /// double-booked with no record anywhere. The scope must stay the full
    /// fan-out; the dead-token case is `classify_partial`'s job now.
    #[test]
    fn the_booking_reverify_fans_out_over_every_account() {
        assert_eq!(
            reverify_scope(Some("dartmouth")),
            None,
            "a slot free on the event's account but busy on another must still collide"
        );
        assert_eq!(reverify_scope(None), None);
    }
}
