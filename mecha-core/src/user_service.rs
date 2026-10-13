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

use anyhow::{Context, Result};
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
        // The unit's paths are rendered under the home and split on spaces
        // into the agent's arguments, so a home with a space in it would
        // come apart wrongly; refused by name instead (review of #679).
        // systemd would mis-split it too.
        // Both: the units are rendered under the mecha home, and `%h` is the
        // home (review of #679: MECHA_HOME can be set apart from it).
        for root in [&m.mecha_home, &m.home] {
            if root.to_string_lossy().contains(char::is_whitespace) {
                anyhow::bail!(
                    "{} has a space in it, which this install cannot write into a service's \
                     command line — install it by hand",
                    root.display()
                );
            }
        }
        let log = crate::launchd::log_path(&m.home, stem);
        let mut agent =
            crate::launchd::agent_from_unit(&crate::launchd::label(stem), unit, &m.home, log)?;
        // Under the log cap, as the router and on-demand agents are
        // (`log_cap.rs`): launchd never rotates a log.
        let mecha = std::env::current_exe().context("finding the mecha binary for the agent")?;
        agent.program = crate::log_cap::wrap(&mecha, &agent.log, agent.program);
        crate::launchd::plist_text(&agent)
    } else {
        unit.to_string()
    };
    crate::llama_units::write_owned(&m.mecha_home, id, &path, &text, 0o644)
}

/// Enable and (re)start services, in order: a restart, never a start alone,
/// so a reinstall reaches a running server with its new files.
///
/// On a Mac loaded with nobody at the screen, says so, as the router does:
/// the agents return after a restart only at a console login (review of
/// #679).
pub fn restart(m: &Machinery, stems: &[&str], say: crate::install::Say<'_>) -> Result<()> {
    if cfg!(target_os = "macos") {
        // launchd opens the log before it runs anything and does not make
        // its directory.
        if let Some(dir) = stems.first().and_then(|s| {
            crate::launchd::log_path(&m.home, s)
                .parent()
                .map(Path::to_path_buf)
        }) {
            std::fs::create_dir_all(dir)?;
        }
        let mut headless = false;
        for stem in stems {
            let domain = crate::launchd::load(&file(m, stem)?, &crate::launchd::label(stem))?;
            headless |= domain == crate::launchd::Domain::User;
        }
        if headless {
            say(
                "nobody is logged in at this Mac's screen, so after a restart these servers come \
                 back only once someone logs in there",
            );
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
            let r = crate::launchd::disable(&crate::launchd::label(stem));
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
        // Both domains, as `launchd::disable` tries them: an install over SSH
        // loads into `user/` (review of #679).
        stems
            .iter()
            .map(|s| {
                let l = crate::launchd::label(s);
                format!(
                    "launchctl bootout gui/$(id -u)/{l}; launchctl bootout user/$(id -u)/{l}; \
                     launchctl disable gui/$(id -u)/{l}; launchctl disable user/$(id -u)/{l}"
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
                "engine",
                include_str!("../units/mecha-breeze-tts.service")
                    .replace("@DIR@", "/d")
                    .replace("@MODEL@", "m.gguf")
                    .replace("@CODEC@", "c.gguf")
                    .replace("@ENGINE_PORT@", "8886"),
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

    /// `Environment=` as systemd reads it: several assignments a line, one
    /// with a space in it quoted whole; what this reader cannot read is
    /// refused, never folded into a neighbour's value (review of #679).
    #[test]
    fn environment_lines_read_as_systemd_reads_them() {
        let unit = "[Service]\nExecStart=/bin/run\nRestart=on-failure\nEnvironment=A=1 B=two\n\
                    Environment=\"GREETING=hello there\" C=%h/c\nRestartSec=5\n";
        let a =
            crate::launchd::agent_from_unit("e", unit, "/Users/a".as_ref(), "/l".into()).unwrap();
        for want in [
            ("A", "1"),
            ("B", "two"),
            ("GREETING", "hello there"),
            ("C", "/Users/a/c"),
        ] {
            assert!(
                a.env.contains(&(want.0.into(), want.1.into())),
                "{want:?} in {:?}",
                a.env
            );
        }
        assert_eq!(a.throttle_secs, 5);
        for bad in [
            "Environment=A='x y'\n",
            "Environment=A=x\\ y\n",
            "Environment=\"A=x\n",
            "Environment=JUSTAKEY\n",
        ] {
            let unit = format!("[Service]\nExecStart=/bin/run\nRestart=on-failure\n{bad}");
            assert!(
                crate::launchd::agent_from_unit("e", &unit, "/h".as_ref(), "/l".into()).is_err(),
                "{bad}"
            );
        }
    }

    /// A directive the reader has no answer for is refused, not dropped.
    #[test]
    fn an_unknown_directive_is_refused() {
        let unit = "[Service]\nExecStart=/bin/run\nExecStartPre=/bin/prep\n";
        let e = crate::launchd::agent_from_unit("u", unit, "/h".as_ref(), "/l".into())
            .unwrap_err()
            .to_string();
        assert!(e.contains("ExecStartPre"), "{e}");
        let unit = "[Unit]\nDescription=x\nAfter=y\n[Service]\nType=exec\nExecStart=/bin/run\n\
                    Restart=on-failure\n[Install]\nWantedBy=default.target\n";
        assert!(crate::launchd::agent_from_unit("k", unit, "/h".as_ref(), "/l".into()).is_ok());
        // The two read by value: a meaning an agent does not have is refused.
        for bad in ["Type=oneshot", "Restart=always", "Restart=no"] {
            let unit = format!("[Service]\nExecStart=/bin/run\n{bad}\n");
            assert!(
                crate::launchd::agent_from_unit("v", &unit, "/h".as_ref(), "/l".into()).is_err(),
                "{bad}"
            );
        }
    }

    /// Each voice sidecar's evidence names the plist its install writes: a
    /// renamed stem would otherwise read every Mac's server as absent
    /// (#678's test, for the voice agents; review of #679).
    #[test]
    fn the_voice_agents_are_their_sidecars_evidence() {
        let speech = crate::speech_unit::Naming::shipped();
        for (id, stem) in [
            (crate::stt_unit::ID, crate::stt_unit::Naming::shipped().unit),
            (
                crate::voice_worker_unit::ID,
                crate::voice_worker_unit::Naming::shipped().unit,
            ),
            (crate::speech_unit::ID, speech.engine_unit.clone()),
            (crate::speech_unit::ID, speech.adapter_unit.clone()),
        ] {
            let name = crate::launchd::plist_name(&stem);
            let s = crate::sidecar::SIDECARS
                .iter()
                .find(|s| s.id == id)
                .unwrap();
            assert!(
                s.evidence
                    .iter()
                    .any(|e| matches!(e, crate::sidecar::Evidence::UserUnit(n) if *n == name)),
                "{name} is not in {id}'s evidence"
            );
        }
    }

    /// A command a shell would have to unquote is refused, never split.
    #[test]
    fn a_quoted_command_is_refused() {
        let unit = "[Service]\nExecStart=-/bin/run\nRestart=on-failure\n";
        assert!(
            crate::launchd::agent_from_unit("p", unit, "/h".as_ref(), "/l".into()).is_err(),
            "a systemd prefix is refused, not taken as the program's name"
        );
        let unit = "[Service]\nExecStart=/bin/sh -c \"echo hi\"\n";
        assert!(crate::launchd::agent_from_unit("q", unit, "/h".as_ref(), "/l".into()).is_err());
        let unit = "[Service]\nExecStart=%h/bin/run --x\nRestart=on-failure\n";
        let a =
            crate::launchd::agent_from_unit("h", unit, "/Users/a".as_ref(), "/l".into()).unwrap();
        assert_eq!(a.program, ["/Users/a/bin/run", "--x"]);
    }
}
