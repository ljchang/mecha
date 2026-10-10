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

/// `~` and `~/…` as a shell would expand them; anything else as typed.
fn expand_home(typed: &str) -> String {
    match (typed, dirs::home_dir()) {
        ("~", Some(h)) => h.display().to_string(),
        (t, Some(h)) if t.starts_with("~/") => h.join(&t[2..]).display().to_string(),
        (t, _) => t.to_string(),
    }
}

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
        match &r.state {
            mecha_core::sidecar::SidecarState::Provided { by } => {
                println!(
                    "This machine already serves a chat model through a router installed by \
                     hand ({by}) — nothing is installed over it. `mecha model list` shows what \
                     it serves; `mecha setup` checks the config agrees with it."
                );
                return Ok(());
            }
            // Nothing is installed over an unknown, as `features enable`
            // says: whose the router is cannot be told, and finding out at
            // `write_owned` comes after the model's download (found on
            // review of #618).
            mecha_core::sidecar::SidecarState::Unknown { why } => {
                println!(
                    "The chat router here could not be checked ({why}), so nothing is installed \
                     over it."
                );
                return Ok(());
            }
            _ => {}
        }
    }

    // A server on the router's port that mecha did not install — a router started from a
    // terminal, or another program — is the owner's too: the unit check above
    // cannot see it (found on review of #568).
    let naming = Naming::shipped();
    if !router_unit::installed_by_mecha(&m.mecha_home)? && router_unit::port_answers(naming.port) {
        println!(
            "Something already answers on :{} and mecha did not install it — nothing is \
             installed over it. `mecha setup --write` reads a running server's settings into \
             the config.",
            naming.port
        );
        return Ok(());
    }

    // A machine no pinned engine build fits is told so before it is asked
    // anything — the answer is to fix the driver, not to say yes (found on
    // review of #568). A machine with a llama-server of its own probes no
    // driver.
    // Its download is priced from the pin for this machine's build, never
    // remembered: a CPU build is a fiftieth of a CUDA one.
    let engine_bytes = if mecha_core::llama_units::has_engine(&m) {
        None
    } else {
        let target = mecha_core::engine::choose(
            std::env::consts::OS,
            std::env::consts::ARCH,
            mecha_core::engine::read_nvidia(),
        )
        .map_err(|why| anyhow::anyhow!("the router needs the llama.cpp engine, and {why}"))?;
        Some(mecha_core::engine::download_bytes(target))
    };

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
            // Per slot first: that is the window a run gets, and the figure
            // the config will say (`-c` is divided across slots).
            geo.map(|g| if g.slots == 1 {
                format!(", served with a {}-token context", g.ctx)
            } else {
                format!(
                    ", served with a {}-token context in each of {} slots ({} in all)",
                    g.ctx / g.slots,
                    g.slots,
                    g.ctx
                )
            })
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
            // Read off the terminal, not through a shell: a leading `~` is
            // expanded here or it names a directory called `~` (found on
            // review of #568).
            let model = expand_home(&ask("Path to the model's GGUF: ")?);
            if model.is_empty() {
                bail!("no model path — nothing was installed");
            }
            let mmproj = expand_home(&ask(
                "Path to its vision projector (mmproj), or Enter for none: ",
            )?);
            Choice::Own {
                model: model.into(),
                mmproj: (!mmproj.is_empty()).then(|| mmproj.into()),
            }
        }
        _ => bail!("`{pick}` is not one of the choices — nothing was installed"),
    };

    // The engine first, when the machine has none to run.
    println!(
        "\nThis installs{} the router (a systemd user service on :{}) serving that model, \
         starts it, and loads the model — a large one takes several minutes. Then it offers to \
         point mecha's config at it.",
        match engine_bytes {
            Some(b) => format!(
                " llama.cpp (mecha's pinned build, {:.0} MiB to download) and",
                b as f64 / 1_048_576.0
            ),
            None => String::new(),
        },
        naming.port
    );
    if !matches!(ask("Go ahead? [y/N] ")?.as_str(), "y" | "Y" | "yes") {
        println!("Nothing was changed.");
        return Ok(());
    }
    if engine_bytes.is_some() {
        mecha_core::engine::install_engine(&m, &mut |s| println!("  {s}"))
            .await
            .context("installing llama.cpp")?;
    }
    let alias = router_unit::install(&m, cfg, &choice, &naming, &machine, &hub, &mut |s| {
        println!("  {s}")
    })
    .await
    .context("installing the router")?;
    println!("\nThe router serves {alias} on :{}.", naming.port);

    // The provider, read back from the router — for the model installed:
    // the router's bare /props is a placeholder (CLAUDE.md, the local model
    // server), never the model's.
    let base = naming.base();
    let props = mecha_core::provider::preflight::fetch(&base, Some(&alias))
        .await
        .with_context(|| format!("asking the router about {alias}"))?;
    // A local provider already in the config — `setup chat` run again to
    // change the model: the table's `model` is the preset name the router
    // routes by, so it is brought in line with what was just installed, the
    // same show-and-ask as `mecha setup --write` (found on review of #568).
    // Only a table that names this router: a local provider on another
    // machine is someone else's server and is not rewritten from this one —
    // and nor is one with no `base_url`, which `Openai::new` sends to
    // api.openai.com (found on review of #618; `router::follows_here` is
    // strict for the same reason).
    if let Some((name, table)) = owners_router_table(cfg, naming.port) {
        println!();
        // The default moves only onto a table that names what the router
        // serves: already agreeing, or rewritten now. Declined, its `model`
        // is a preset that is gone. Agreement is checked first, so a re-run
        // that changed nothing is not asked an identical rewrite whose "n"
        // would also skip the default (found on review of #618).
        let settings = mecha_core::onboarding::verified_settings(&props);
        let current = if mecha_core::onboarding::table_agrees(table, &settings) {
            println!("[providers.{name}] already names what the router serves.");
            true
        } else {
            super::setup::offer_settings(name, &props)?
        };
        if current {
            super::setup::offer_default(name, &cfg.default_provider)?;
        } else {
            // Declined: the table names what the router served before, and
            // the router now serves only `alias` — said here, or a run that
            // goes through it meets a bare 404 later (found on review of
            // #618).
            println!(
                "[providers.{name}] was left as it was, and the router now serves only {alias} — \
                 a run through `{name}` fails until `mecha setup --write --provider {name}` \
                 writes it."
            );
        }
        return Ok(());
    }
    if let Some((name, _)) = cfg.providers.iter().find(|(_, p)| p.configured_local()) {
        println!(
            "Your config's local provider `{name}` names a server elsewhere, so it is left as it \
             is — the router here serves {alias} on {base}."
        );
        return Ok(());
    }
    let found = mecha_core::onboarding::LocalServer {
        base_url: base,
        props,
    };
    if !super::setup::write_local_provider(&found)? {
        // Declined, or no terminal to ask at: the default may still name
        // this router — the starter's does, and so does the built-in entry
        // with no file at all (ruling F12) — with no model, and a run would
        // send a model name the router does not serve. Said here, not met
        // as a bare 404 after the download (found on review of #627; the
        // #618 incident, on the path F12 made the common one).
        if let Some(p) = cfg.providers.get(&cfg.default_provider) {
            if router_unit::names_this_router(p, naming.port) && p.model.as_deref() != Some(&alias)
            {
                println!(
                    "The default provider `{}` names this router but not {alias}, so a run \
                     through it fails until `mecha setup --write` writes it.",
                    cfg.default_provider
                );
            }
        }
    }
    Ok(())
}

/// The table the owner wrote that names the router on `port` — never the
/// default's built-in entry (ruling F12), which names it too but is in no
/// file: rewriting it bailed after the download on any config file without a
/// `[providers.local]`, which is every one the older starter wrote (found on
/// review of #627). With none, `write_local_provider` writes one.
fn owners_router_table(
    cfg: &mecha_core::config::Config,
    port: u16,
) -> Option<(&String, &mecha_core::config::ProviderConfig)> {
    cfg.providers
        .iter()
        .find(|(_, p)| !p.built_in && router_unit::names_this_router(p, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default's own entry names the router but is no table to rewrite;
    /// one the owner wrote is (review of #627).
    #[test]
    fn only_the_owners_table_is_the_router_s_to_rewrite() {
        let mut cfg = mecha_core::config::Config::default();
        assert!(router_unit::names_this_router(
            &cfg.providers["local"],
            8080
        ));
        assert!(owners_router_table(&cfg, 8080).is_none());
        cfg.providers.get_mut("local").unwrap().built_in = false;
        assert_eq!(owners_router_table(&cfg, 8080).unwrap().0, "local");
    }

    /// `chat` is a noun in `setup`'s feature position, as `engine` is, so no
    /// feature may ever be called that.
    #[test]
    fn no_feature_is_called_chat() {
        assert!(mecha_core::feature::Feature::parse("chat").is_none());
    }

    #[test]
    fn a_typed_home_is_expanded_and_nothing_else() {
        let h = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~"), h.display().to_string());
        assert_eq!(
            expand_home("~/models/m.gguf"),
            h.join("models/m.gguf").display().to_string()
        );
        assert_eq!(expand_home("~bob/m.gguf"), "~bob/m.gguf");
        assert_eq!(expand_home("models/m.gguf"), "models/m.gguf");
        assert_eq!(expand_home(""), "");
    }
}
