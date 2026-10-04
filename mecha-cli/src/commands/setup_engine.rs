//! `mecha setup engine` (`docs/FEATURES-DESIGN.md` §10.3): which llama.cpp
//! each server runs, and the two moves 7b-3a builds — `--adopt`, which
//! measures the shipped pin against the engine the units run today and moves
//! them onto it when it is no slower, and `--rollback`, which moves an
//! adopted machine back onto the hand install it left in place.
//!
//! `engine` is a reserved noun in `setup`'s feature position, never a
//! feature id. Both moves change what every server runs, so both refuse
//! without a terminal: a flag typed into a trigger, a hook or `ssh host …` is
//! not the owner at a terminal.

use anyhow::{bail, Context, Result};
use mecha_core::engine_gate::{self as gate, LedgerRow, Outcome, Role, Smoke};
use std::io::{IsTerminal, Write};

/// The router a local provider points at, and the model it names: the
/// default provider when it is local, else the first local one, else the
/// router's usual address.
fn router_of(cfg: &mecha_core::config::Config) -> (String, Option<String>) {
    let local = cfg
        .providers
        .get(&cfg.default_provider)
        .filter(|p| p.kind == "local")
        .or_else(|| cfg.providers.values().find(|p| p.kind == "local"));
    match local {
        Some(p) => (
            p.base_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8080".into()),
            p.model.clone(),
        ),
        None => ("http://127.0.0.1:8080".into(), None),
    }
}

pub async fn run(
    cfg: &mecha_core::config::Config,
    json: bool,
    adopt: bool,
    rollback: bool,
    now: bool,
    force: bool,
) -> Result<()> {
    anyhow::ensure!(!json, "`mecha setup engine` has no --json form yet");
    let m = mecha_core::sidecar::Machinery::real()?;
    let (base, model) = router_of(cfg);
    if adopt || rollback {
        this_machine(&base)?;
        anyhow::ensure!(
            std::io::stdin().is_terminal(),
            "`mecha setup engine --{}` changes what every server runs, so it runs only at a \
             terminal",
            if adopt { "adopt" } else { "rollback" }
        );
    }
    if adopt {
        return run_adopt(&m, &base, model.as_deref(), force).await;
    }
    if rollback {
        return run_rollback(&m, &base, model.as_deref(), now).await;
    }
    print!("{}", status(&m)?);
    Ok(())
}

/// `kind = "local"` is a dialect, not a place: the gate stops and moves this
/// machine's units, so the router it keys the switch on, reads the resident
/// model of and asks about afterwards must be this machine's.
fn this_machine(base: &str) -> Result<()> {
    anyhow::ensure!(
        mecha_core::provider::router::is_loopback(base),
        "the local provider's router is {base}, not this machine — `mecha setup engine` moves \
         this machine's llama.cpp units, so run it on the machine that serves the model"
    );
    Ok(())
}

fn label(role: Role) -> &'static str {
    match role {
        Role::Router => "the router",
        Role::Embeddings => "embeddings",
        Role::Ocr => "OCR",
    }
}

/// Which engine each server runs, mecha's builds, and the last measurement.
fn status(m: &mecha_core::sidecar::Machinery) -> Result<String> {
    let (servers, unreadable) = gate::Servers::read_or_none();
    let managed = gate::managed_binary(&m.mecha_home);
    let mut out = String::from("llama.cpp on this machine:\n");
    match (&unreadable, servers.present.is_empty()) {
        (Some(why), _) => out.push_str(&format!(
            "  no units read — systemd's user manager could not be asked ({why})\n"
        )),
        (None, true) => {
            out.push_str("  no llama.cpp units — they arrive with their features (step 7c)\n")
        }
        _ => {}
    }
    for (s, l) in &servers.present {
        let line = match l.engine(&servers.base_env) {
            Some(e) => format!(
                "{} — {}{}",
                e.display(),
                gate::version_of(&e),
                if e == managed {
                    " (mecha's)"
                } else {
                    " (provided, left alone)"
                }
            ),
            None => "no llama-server on its PATH".into(),
        };
        out.push_str(&format!("  {:<12} {:<22} {line}\n", label(s.role), s.unit));
    }
    let manifest = mecha_core::sidecar::Manifest::read(&m.mecha_home)?;
    let builds = manifest
        .entries
        .iter()
        .find(|e| e.sidecar == "llama")
        .map(|e| e.builds.clone())
        .unwrap_or_default();
    if builds.is_empty() {
        out.push_str("mecha's engines: none installed\n");
    } else {
        let current = std::fs::read_link(mecha_core::engine::current(&m.mecha_home)).ok();
        let list: Vec<String> = builds
            .iter()
            .map(|b| {
                let here = current.as_deref() == Some(std::path::Path::new(&b.tag));
                format!(
                    "{} ({}){}",
                    b.tag,
                    &b.commit[..b.commit.len().min(9)],
                    if here { " — current" } else { "" }
                )
            })
            .collect();
        out.push_str(&format!("mecha's engines: {}\n", list.join(", ")));
    }
    match gate::read_ledger(&m.mecha_home) {
        Ok(ledger) => {
            match ledger.rows.last() {
                Some(row) => out.push_str(&format!(
                    "last measured: {} {} — {}\n",
                    row.at.format("%Y-%m-%d %H:%MZ"),
                    row.action,
                    outcome_text(&row.outcome)
                )),
                None => out.push_str("last measured: never\n"),
            }
            if ledger.skipped > 0 {
                out.push_str(&format!(
                    "  ({} ledger line(s) this build cannot read)\n",
                    ledger.skipped
                ));
            }
        }
        Err(e) => out.push_str(&format!("last measured: unknown — {e:#}\n")),
    }
    if !servers.present.is_empty() && !gate::adopted(&servers, &m.mecha_home) {
        out.push_str(&format!(
            "\n`mecha setup engine --adopt` measures llama.cpp {} against the engine these units \
             run, and moves them onto it if it is no slower — the router stops for the \
             measurement.\n",
            mecha_core::engine::PIN.tag
        ));
    }
    Ok(out)
}

fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Promoted => "promoted".into(),
        Outcome::PromotedByForce { why } => format!("promoted with --force, although {why}"),
        Outcome::Kept { why } => format!("kept the old engine: {why}"),
        Outcome::Partial {
            step,
            error,
            finish,
        } => format!("STOPPED PART WAY at {step}: {error} — finish with: {finish}"),
        Outcome::Unknown => "an outcome this build does not know".into(),
    }
}

/// A measured row, in words: both engines, both rates, every smoke test, and
/// what came of it.
fn render(row: &LedgerRow) -> String {
    let mut out = String::new();
    for (name, leg) in [("old", &row.old), ("new", &row.new)] {
        out.push_str(&format!(
            "{name}: {} — {}\n",
            leg.engine.display(),
            leg.version
        ));
        match &leg.bench {
            Some(b) => out.push_str(&format!(
                "  generation {:.1} tok/s (runs within {:.0} %), prefill {:.0} tok/s (within {:.0} %), median of {}\n",
                b.generation_tps.median(),
                b.generation_tps.spread() * 100.0,
                b.prefill_tps.median(),
                b.prefill_tps.spread() * 100.0,
                b.generation_tps.runs.len()
            )),
            None => out.push_str("  chat model not measured\n"),
        }
        for (smoke, result) in &leg.smoke {
            let r = match result {
                Smoke::Passed => "passed".to_string(),
                Smoke::Failed { error } => format!("FAILED — {error}"),
                Smoke::NotRun { why } => format!("not run — {why}"),
                Smoke::Unknown => "a result this build does not know".into(),
            };
            out.push_str(&format!("  {smoke:<11} {r}\n"));
        }
    }
    if let Some(model) = &row.model {
        out.push_str(&format!("measured on {model}\n"));
    }
    out.push_str(&format!("outcome: {}\n", outcome_text(&row.outcome)));
    out
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

async fn run_adopt(
    m: &mecha_core::sidecar::Machinery,
    base: &str,
    model: Option<&str>,
    force: bool,
) -> Result<()> {
    println!(
        "This installs llama.cpp {} beside the engine your servers run, stops the router, and \
         measures the model it has loaded on both engines in turn — a few minutes in which chat \
         does not answer. If the new engine is no slower and passes every check the old one \
         passes{}, the router, embeddings and OCR units are pointed at it; the engine they run \
         today stays in place as the way back (`mecha setup engine --rollback`).",
        mecha_core::engine::PIN.tag,
        if force {
            " — or whatever it measures, with --force"
        } else {
            ""
        }
    );
    // The refusals that move nothing, before the owner is asked to agree to
    // a router stop.
    let servers = gate::Servers::read()?;
    let holds = mecha_core::hold::Holds::new(mecha_core::hold::dir_under(&m.mecha_home));
    gate::check_adoptable(
        m,
        &servers,
        &holds,
        &mecha_core::provider::router::base(base),
    )?;
    if !confirm("Go ahead?")? {
        println!("Nothing was changed.");
        return Ok(());
    }
    // From here an interrupt is the owner's "stop": during the download or
    // the measurement it unwinds (the servers it started stopped, the router
    // restarted on its engine); during the promotion it is held until the
    // promotion has finished, never leaving the drop-ins half written.
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("installing the interrupt handler for the adopt")?;
    let on_interrupt = cancel.clone();
    let listener = tokio::spawn(async move {
        while interrupt.recv().await.is_some() {
            eprintln!("\ninterrupted — stopping what is running and putting the router back");
            on_interrupt.cancel();
        }
    });
    let row = gate::adopt(m, &servers, &holds, base, model, force, &cancel, &mut |s| {
        println!("  {s}")
    })
    .await;
    listener.abort();
    let row = row.context("adopting the engine")?;
    print!("\n{}", render(&row));
    if let Outcome::Partial { .. } = row.outcome {
        bail!(
            "the adopt stopped part way — see above for the step and the command that finishes it"
        );
    }
    Ok(())
}

async fn run_rollback(
    m: &mecha_core::sidecar::Machinery,
    base: &str,
    model: Option<&str>,
    now: bool,
) -> Result<()> {
    let base = mecha_core::provider::router::base(base);
    let servers = gate::Servers::read()?;
    let dir = gate::unit_dir(m)?;
    let adopted: Vec<&'static str> = servers
        .present
        .iter()
        .map(|(s, _)| s.unit)
        .filter(|u| gate::drop_in_path(dir, u).exists())
        .collect();
    if adopted.is_empty() {
        // A partial adopt can record a drop-in it never wrote; forget those,
        // or the engine reads as a half-finished install for good.
        let mut cleared = 0;
        for (srv, _) in &servers.present {
            let path = gate::drop_in_path(dir, srv.unit);
            let before = mecha_core::sidecar::Manifest::read(&m.mecha_home)?;
            if before.entries.iter().any(|e| e.wrote.contains(&path)) {
                mecha_core::sidecar::Manifest::unrecord(&m.mecha_home, "llama", &path)?;
                cleared += 1;
            }
        }
        if cleared > 0 {
            println!("no drop-ins were on disk; cleared {cleared} recorded but never written");
            return Ok(());
        }
        bail!(
            "no unit here runs mecha's engine through an adopt — nothing to roll back (moving \
             back from an upgrade arrives with `--upgrade`)"
        );
    }
    // What the router runs once the drop-ins are gone, read before anything
    // moves: a lookup that fails stops the rollback before it starts, never
    // after it succeeded.
    let provided = {
        let (_, l) = servers
            .get(Role::Router)
            .context("there is no llama-local.service")?;
        let mut bare = l.clone();
        bare.env.retain(|(k, _)| k != "LLAMA_SERVER");
        bare.engine(&servers.base_env).context(
            "without mecha's drop-in the router's unit names no llama-server it can find — \
             nothing was rolled back",
        )?
    };
    // A rollback waits for runs, as any switch does; `--now` asks them to
    // stop (hold.rs's ruling, which the measurement alone departs from).
    let holds = mecha_core::hold::Holds::new(mecha_core::hold::dir_under(&m.mecha_home));
    let switching =
        match holds.begin_switch(&base, Some("llama.cpp (mecha's)"), "llama.cpp (provided)")? {
            Ok(s) => s,
            Err(other) => bail!(
            "a switch to {} is already waiting on {base} (pid {}) — let it finish, or withdraw it \
             with `mecha model cancel-switch`",
            other.to,
            other.pid
        ),
        };
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("installing the interrupt handler for the rollback")?;
    if now {
        let asked = holds.cancel_holders(&base);
        if asked > 0 {
            eprintln!("asking {asked} run(s) to stop now");
        }
    }
    super::model::wait_for_runs(
        &holds,
        &switching,
        &base,
        now.then_some(super::model::NOW_GRACE),
        &mut interrupt,
    )
    .await
    .context("nothing was rolled back")?;
    switching.past_the_wait()?;
    // What the router has loaded now, read before the restart: the model the
    // router is asked about afterwards — and loaded again — is the one the
    // owner had, not only the configured one.
    let listed = mecha_core::provider::router::models(&base).await;
    let (check, unchecked) = match gate::measured_model(listed.as_deref(), model) {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };

    // Every present unit's drop-in path is forgotten, written or not: a
    // partial adopt records before it writes.
    for (srv, _) in &servers.present {
        let path = gate::drop_in_path(dir, srv.unit);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        mecha_core::sidecar::Manifest::unrecord(&m.mecha_home, "llama", &path)?;
    }
    let finish = "systemctl --user daemon-reload && systemctl --user restart llama-local.service";
    gate::systemctl("daemon-reload", &[]).with_context(|| format!("finish with: {finish}"))?;
    let backends: Vec<&str> = adopted
        .iter()
        .copied()
        .filter(|u| *u != "llama-local.service")
        .collect();
    if !backends.is_empty() {
        gate::systemctl("stop", &backends).with_context(|| format!("finish with: {finish}"))?;
    }
    gate::systemctl("restart", &["llama-local.service"])
        .with_context(|| format!("finish with: {finish}"))?;
    gate::router_back(&base, check.as_deref(), &provided)
        .await
        .with_context(|| {
            format!(
                "the router did not come back on {} — finish with: {finish}",
                provided.display()
            )
        })?;
    drop(switching);
    match &check {
        Some(model) => println!(
            "rolled back: the router serves {model} on {} again, and the embeddings and OCR \
             servers start on it at their next request; mecha's engine stays installed for \
             another `--adopt`",
            provided.display()
        ),
        None => println!(
            "rolled back: the drop-ins are gone and the router answers, but which engine runs \
             it was not asked ({}) — `mecha setup engine` shows what each unit names; mecha's \
             engine stays installed for another `--adopt`",
            unchecked.as_deref().unwrap_or("no model to ask about")
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `engine` is a noun in `setup`'s feature position, so no feature may
    /// ever be called that.
    #[test]
    fn no_feature_is_called_engine() {
        assert!(mecha_core::feature::Feature::parse("engine").is_none());
    }

    #[test]
    fn the_router_is_the_local_provider_s() {
        let mut cfg = mecha_core::config::Config::default();
        assert_eq!(router_of(&cfg).0, "http://127.0.0.1:8080");
        let name = cfg.default_provider.clone();
        let p = cfg.providers.get_mut(&name).unwrap();
        p.kind = "local".into();
        p.base_url = Some("http://127.0.0.1:9090/v1".into());
        p.model = Some("qwen".into());
        assert_eq!(
            router_of(&cfg),
            ("http://127.0.0.1:9090/v1".into(), Some("qwen".into()))
        );
    }

    /// A `local` provider on another host is refused, never measured here
    /// and moved there.
    #[test]
    fn only_this_machine_s_router_is_adopted() {
        assert!(this_machine("http://127.0.0.1:8080").is_ok());
        assert!(this_machine("http://localhost:8080/v1").is_ok());
        let err = this_machine("http://100.64.0.7:8080")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not this machine"), "{err}");
    }

    #[test]
    fn a_partial_outcome_says_how_to_finish() {
        let t = outcome_text(&Outcome::Partial {
            step: "starting the router".into(),
            error: "exit 1".into(),
            finish: "systemctl --user start llama-local.service".into(),
        });
        assert!(
            t.contains("STOPPED PART WAY") && t.contains("systemctl --user start"),
            "{t}"
        );
    }
}
