//! `mecha setup chat` (ruling F11, `docs/FEATURES-DESIGN.md` §10.6 7c-2): a
//! fresh machine's door to a local chat model. It shows the chat row
//! recommended for this machine's tier — the model, its download, the context
//! and slots the router will serve it with — or takes a GGUF the owner brings;
//! installs mecha's engine first if the machine has no llama-server; installs
//! and starts the router; and then hands the router's own `/props`, for the
//! model it installed, to `mecha setup`'s existing write-and-confirm flow, so
//! the provider's settings are read back from the server, never typed.
//!
//! `chat` is a reserved noun in `setup`'s feature position, never a feature
//! id. A machine whose router was installed by hand is told so and left alone.

use anyhow::{bail, Context, Result};
use mecha_core::router_unit::{self, Choice, Naming};
use std::io::{IsTerminal, Write};

fn ask(question: &str) -> Result<String> {
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub async fn run(cfg: &mecha_core::config::Config) -> Result<()> {
    anyhow::ensure!(
        std::io::stdin().is_terminal(),
        "`mecha setup chat` installs a model server and asks which model, so it runs only at a \
         terminal"
    );
    // Refused before anything is asked or fetched: the router is a systemd
    // user unit, and the engine download ahead of it would otherwise run
    // and then fail (found on review of #568).
    anyhow::ensure!(
        cfg!(target_os = "linux"),
        "`mecha setup chat` installs the router as a systemd user unit, which this system does \
         not have — start llama-server by hand and run `mecha setup --write` (FEATURES-DESIGN \
         §10.5)"
    );
    let m = mecha_core::sidecar::Machinery::real()?;
    let machine = mecha_core::recommend::Machine::read()?;
    let hub = mecha_core::fetch::hub_dir()?;

    // A router installed by hand is the owner's: nothing is installed over it.
    let plan = mecha_core::sidecar::plan(
        mecha_core::feature::Feature::Messages,
        &m,
        &machine,
        &hub,
        false,
    )?;
    if let Some(r) = plan.sidecars.iter().find(|s| s.id == router_unit::ID) {
        if let mecha_core::sidecar::SidecarState::Provided { by } = &r.state {
            println!(
                "This machine already serves a chat model through a router installed by hand \
                 ({by}) — nothing is installed over it. `mecha model list` shows what it serves; \
                 `mecha setup` checks the config agrees with it."
            );
            return Ok(());
        }
    }

    // A machine no pinned engine build fits is told so before it is asked
    // anything — the answer is to fix the driver, not to say yes (found on
    // review of #568). A machine with a llama-server of its own probes no
    // driver.
    let needs_engine = !mecha_core::llama_units::has_engine(&m);
    if needs_engine {
        mecha_core::engine::choose(
            std::env::consts::OS,
            std::env::consts::ARCH,
            mecha_core::engine::read_nvidia(),
        )
        .map_err(|why| anyhow::anyhow!("the router needs the llama.cpp engine, and {why}"))?;
    }

    // The choice: the tier's row, or the owner's own GGUF.
    let slot = mecha_core::recommend::SLOTS
        .iter()
        .find(|s| s.id == "chat")
        .context("no chat slot")?;
    let row = mecha_core::recommend::row_for(slot, &machine).map(|(r, _)| r);
    println!("Chat model for this machine:");
    if let Some(row) = row {
        let geo = mecha_core::recommend::chat_geometry(row.tier_gb);
        let bytes: u64 = row
            .sources
            .iter()
            .filter_map(|s| match s {
                mecha_core::recommend::Source::HuggingFace { files, .. } => {
                    Some(files.iter().map(|f| f.bytes).sum::<u64>())
                }
                _ => None,
            })
            .sum();
        println!(
            "  1) {} — {:.1} GiB to download (less what is already in the cache){}   (recommended)",
            row.model,
            bytes as f64 / 1_073_741_824.0,
            geo.map(|g| format!(
                ", served with {} tokens across {} slot{}",
                g.ctx,
                g.slots,
                if g.slots == 1 { "" } else { "s" }
            ))
            .unwrap_or_default()
        );
    } else {
        println!("  (no chat model is recommended at this machine's memory tier)");
    }
    println!("  2) a GGUF you already have");
    let pick = ask(if row.is_some() {
        "Which? [1] "
    } else {
        "Which? [2] "
    })?;
    let choice = match (pick.as_str(), row.is_some()) {
        ("" | "1", true) => Choice::Recommended,
        ("" | "2", _) => {
            let model = ask("Path to the model's GGUF: ")?;
            if model.is_empty() {
                bail!("no model path — nothing was installed");
            }
            let mmproj = ask("Path to its vision projector (mmproj), or Enter for none: ")?;
            Choice::Own {
                model: model.into(),
                mmproj: (!mmproj.is_empty()).then(|| mmproj.into()),
            }
        }
        _ => bail!("`{pick}` is not one of the choices — nothing was installed"),
    };

    // The engine first, when the machine has none to run.
    println!(
        "\nThis installs{} the router (a systemd user service on :8080) serving that model, \
         starts it, and loads the model — a large one takes several minutes. Then it offers to \
         point mecha's config at it.",
        if needs_engine {
            " llama.cpp (mecha's pinned build, about 700 MB) and"
        } else {
            ""
        }
    );
    if !matches!(ask("Go ahead? [y/N] ")?.as_str(), "y" | "Y" | "yes") {
        println!("Nothing was changed.");
        return Ok(());
    }
    if needs_engine {
        mecha_core::engine::install_engine(&m, &mut |s| println!("  {s}"))
            .await
            .context("installing llama.cpp")?;
    }
    let naming = Naming::shipped();
    let alias = router_unit::install(&m, &choice, &naming, &machine, &hub, &mut |s| {
        println!("  {s}")
    })
    .await
    .context("installing the router")?;
    println!("\nThe router serves {alias} on :{}.", naming.port);

    // The provider, read back from the router — for the model installed:
    // the router's bare /props is a placeholder (CLAUDE.md, the local model
    // server), never the model's.
    if cfg.providers.values().any(|p| p.kind == "local") {
        println!(
            "Your config already names a local provider — `mecha setup` checks it agrees with \
             the router, and `mecha setup --write` brings it in line."
        );
        return Ok(());
    }
    let base = naming.base();
    let props = mecha_core::provider::preflight::fetch(&base, Some(&alias))
        .await
        .with_context(|| format!("asking the router about {alias}"))?;
    let found = mecha_core::onboarding::LocalServer {
        base_url: base,
        props,
    };
    super::setup::write_local_provider(&found)
}
