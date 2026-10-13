//! One resident user service on whichever manager this system has
//! (`docs/FEATURES-DESIGN.md` §12.2): a systemd user unit on Linux, a launchd
//! agent on macOS.
//!
//! **The unit is the source.** Each installer renders its systemd unit as it
//! always has; on a Mac the agent is read out of that same text
//! (`launchd::agent_from_unit`), so what a Mac runs is what Linux runs, and a
//! change to a unit's command reaches both. The router and the on-demand
//! servers keep their own launchd halves: theirs differ in shape, not only
//! in manager.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::sidecar::Machinery;

/// The file a service is defined by here: `<stem>.service` in the user unit
/// directory, or `ai.mecha-factory.<stem>.plist` in `~/Library/LaunchAgents`.
pub fn file(m: &Machinery, stem: &str) -> Result<PathBuf> {
    let dir = crate::engine_gate::unit_dir(m)?;
    Ok(if cfg!(target_os = "macos") {
        dir.join(crate::launchd::plist_name(stem))
    } else {
        dir.join(format!("{stem}.service"))
    })
}

/// Write a service's definition from its rendered unit, recorded under `id`.
pub fn write(m: &Machinery, id: &str, stem: &str, unit: &str) -> Result<()> {
    let path = file(m, stem)?;
    let text = if cfg!(target_os = "macos") {
        let log = crate::launchd::log_path(&m.home, stem);
        crate::launchd::plist_text(&crate::launchd::agent_from_unit(
            &crate::launchd::label(stem),
            unit,
            &m.home,
            log,
        )?)
    } else {
        unit.to_string()
    };
    crate::llama_units::write_owned(&m.mecha_home, id, &path, &text, 0o644)
}

/// Enable and (re)start services, in order: a restart, never a start alone,
/// so a reinstall reaches a running server with its new files.
pub fn restart(m: &Machinery, stems: &[&str]) -> Result<()> {
    if cfg!(target_os = "macos") {
        // launchd opens the log before it runs anything and does not make
        // its directory.
        if let Some(dir) = crate::launchd::log_path(&m.home, "x").parent() {
            std::fs::create_dir_all(dir)?;
        }
        for stem in stems {
            crate::launchd::load(&file(m, stem)?, &crate::launchd::label(stem))?;
        }
        Ok(())
    } else {
        let names: Vec<String> = stems.iter().map(|s| format!("{s}.service")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        crate::engine_gate::systemctl("daemon-reload", &[])?;
        crate::engine_gate::systemctl("enable", &names)?;
        crate::engine_gate::systemctl("restart", &names)
    }
}

/// Stop services and keep them from starting again.
pub fn stop(stems: &[&str]) -> Result<()> {
    if cfg!(target_os = "macos") {
        let mut first = Ok(());
        for stem in stems {
            let r = crate::launchd::unload(&crate::launchd::label(stem));
            if first.is_ok() {
                first = r;
            }
        }
        first
    } else {
        let mut args = vec!["--now".to_string()];
        args.extend(stems.iter().map(|s| format!("{s}.service")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::engine_gate::systemctl("disable", &args)
    }
}

/// After definitions were removed: systemd rereads its directories; launchd
/// has nothing to reread.
pub fn after_remove() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        crate::engine_gate::systemctl("daemon-reload", &[])
    }
}

/// The command that stops them by hand, for an error that could not.
pub fn stop_by_hand(stems: &[&str]) -> String {
    if cfg!(target_os = "macos") {
        stems
            .iter()
            .map(|s| {
                format!(
                    "launchctl bootout gui/$(id -u)/{}",
                    crate::launchd::label(s)
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    } else {
        format!(
            "systemctl --user disable --now {}",
            stems
                .iter()
                .map(|s| format!("{s}.service"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

/// Where any of them says why, for a check that knows the port, not the
/// unit.
pub fn logs_anywhere() -> &'static str {
    if cfg!(target_os = "macos") {
        "its log in ~/Library/Logs/mecha/ says why"
    } else {
        "`journalctl --user -u <its unit>` says why"
    }
}

/// Where a service's own output says why it failed.
pub fn logs_hint(home: &Path, stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!(
            "{} says why",
            crate::launchd::log_path(home, stem).display()
        )
    } else {
        format!("`journalctl --user -u {stem}` says why")
    }
}

#[cfg(test)]
mod tests {
    /// Every shipped unit that runs on a Mac reads as an agent: a command, no
    /// shell quoting, and its environment and directory carried over.
    #[test]
    fn the_voice_units_read_as_agents() {
        let home = std::path::Path::new("/Users/someone");
        for (name, unit) in [
            (
                "parakeet",
                include_str!("../units/mecha-parakeet.service")
                    .replace("@VENV@", "/v")
                    .replace("@DIR@", "/d")
                    .replace("@MODEL@", "/m")
                    .replace("@PORT@", "8090"),
            ),
            (
                "worker",
                include_str!("../units/mecha-voice-worker.service")
                    .replace("@VENV@", "/v")
                    .replace("@DIR@", "/d")
                    .replace("@NO_PAGE@", "http://no.page")
                    .replace("@PORT@", "7860"),
            ),
            (
                "adapter",
                include_str!("../units/mecha-breeze-adapter.service")
                    .replace("@DIR@", "/d")
                    .replace("@ENGINE_UNIT@", "e")
                    .replace("@ENGINE_PORT@", "8886")
                    .replace("@ADAPTER_PORT@", "8887")
                    .replace("@MODEL_NAME@", "m"),
            ),
        ] {
            let a = crate::launchd::agent_from_unit(name, &unit, home, "/l".into())
                .unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert!(!a.program.is_empty(), "{name}");
            assert!(a.env.iter().any(|(k, _)| k == "PATH"), "{name} has a PATH");
            assert!(
                !a.program.iter().any(|p| p.contains('@')),
                "{name}: {:?}",
                a.program
            );
        }
        let adapter = crate::launchd::agent_from_unit(
            "a",
            &include_str!("../units/mecha-breeze-adapter.service")
                .replace("@DIR@", "/d")
                .replace("@ENGINE_PORT@", "8886")
                .replace("@ADAPTER_PORT@", "8887")
                .replace("@MODEL_NAME@", "m"),
            home,
            "/l".into(),
        )
        .unwrap();
        assert_eq!(
            adapter.workdir.as_deref(),
            Some(std::path::Path::new("/d/adapter"))
        );
        assert!(adapter
            .env
            .contains(&("BREEZE_TTS_URL".into(), "http://127.0.0.1:8886".into())));
        assert_eq!(
            adapter.program,
            ["/d/venv/bin/python", "-B", "breeze_server.py"]
        );
    }

    /// A command a shell would have to unquote is refused, never split.
    #[test]
    fn a_quoted_command_is_refused() {
        let unit = "[Service]\nExecStart=/bin/sh -c \"echo hi\"\n";
        assert!(crate::launchd::agent_from_unit("q", unit, "/h".as_ref(), "/l".into()).is_err());
        let unit = "[Service]\nExecStart=%h/bin/run --x\n";
        let a =
            crate::launchd::agent_from_unit("h", unit, "/Users/a".as_ref(), "/l".into()).unwrap();
        assert_eq!(a.program, ["/Users/a/bin/run", "--x"]);
    }
}
