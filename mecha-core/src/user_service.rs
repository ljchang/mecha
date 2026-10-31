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

/// Each sidecar's service revision: the number its service definitions
/// carry (`stamp`), raised whenever what an install writes for it changes.
/// An installed sidecar whose definitions carry an older number — or none,
/// as everything installed before the stamp does — is stale, and `enable` and
/// `setup` offer to refresh it: its install run again, which fetches nothing
/// that is current and rewrites and restarts its services (the owner's
/// ruling, 2026-10-14; FEATURES-DESIGN §14). Each module's own test holds its
/// number to a hash of what it renders, so a changed service fails the build
/// until its number is raised.
pub const REVS: &[(&str, u32)] = &[
    ("router", 1),
    ("embed-server", 1),
    ("ocr-server", 1),
    ("stt", 1),
    ("voice-worker", 1),
    ("speech", 1),
    ("comfyui", 1),
];

/// A sidecar's current service revision, if it has services.
pub fn rev(id: &str) -> Option<u32> {
    REVS.iter().find(|(i, _)| *i == id).map(|(_, r)| *r)
}

/// The marker a service definition carries, as written into it.
fn marker(id: &str, rev: u32) -> String {
    format!("mecha-service-rev: {id} {rev}")
}

/// `text`, a systemd unit or a launchd plist, carrying `id`'s current
/// revision: a comment on the unit's first line, or after the plist's XML
/// declaration. A sidecar with no revision is written as it is.
pub fn stamp(id: &str, text: &str) -> String {
    let Some(rev) = rev(id) else {
        return text.to_string();
    };
    let m = marker(id, rev);
    if let Some(rest) = text.strip_prefix("<?xml") {
        // After the declaration's line, which must stay first.
        match rest.split_once('\n') {
            Some((decl, body)) => format!("<?xml{decl}\n<!-- {m} -->\n{body}"),
            None => format!("{text}\n<!-- {m} -->\n"),
        }
    } else {
        format!("# {m}\n{text}")
    }
}

/// The revision a written definition carries for `id`, if any.
pub fn rev_of(text: &str, id: &str) -> Option<u32> {
    let key = format!("mecha-service-rev: {id} ");
    text.lines()
        .find_map(|l| l.split_once(&key).map(|(_, r)| r))
        .and_then(|r| {
            r.trim_end_matches("-->")
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

/// Whether a file mecha wrote is one of a service's definitions — what
/// carries the stamp.
pub fn is_definition(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("service" | "socket" | "timer" | "plist")
    )
}

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

/// What a service install can know before anything is fetched: on a Mac, a
/// home or mecha home with a space in it cannot become an agent's command
/// line (the unit's paths are split on spaces), so it is refused here,
/// before the build and the downloads (review of #679). systemd would
/// mis-split it too, but Linux keeps its own behaviour.
pub fn preflight(m: &Machinery) -> Result<()> {
    if cfg!(target_os = "macos") {
        for root in [&m.mecha_home, &m.home] {
            if root.to_string_lossy().contains(char::is_whitespace) {
                anyhow::bail!(
                    "{} has a space in it, which this install cannot write into a service's \
                     command line — install it by hand",
                    root.display()
                );
            }
        }
    }
    Ok(())
}

/// A unit as the launchd agent a Mac runs instead, its plist's text: the
/// unit's command under the log cap, as the router and on-demand agents are
/// (`log_cap.rs`) — launchd never rotates a log.
fn agent_text(
    stem: &str,
    unit: &str,
    home: &Path,
    mecha_home: &Path,
    mecha: &Path,
) -> Result<String> {
    let log = crate::launchd::log_path(home, stem);
    let mut agent = crate::launchd::agent_from_unit(&crate::launchd::label(stem), unit, home, log)?;
    agent.program = crate::log_cap::wrap(mecha, &agent.log, agent.program);
    // The wrapper's own-program check resolves the mecha home from the
    // environment: an agent carries it, or one under a MECHA_HOME other than
    // ~/.mecha refuses its own program (review of #679).
    agent
        .env
        .push(("MECHA_HOME".into(), mecha_home.display().to_string()));
    Ok(crate::launchd::plist_text(&agent))
}

/// Write a service's definition from its rendered unit, recorded under `id`.
pub fn write(m: &Machinery, id: &str, stem: &str, unit: &str) -> Result<()> {
    let path = file(m, stem)?;
    let text = stamp(
        id,
        &if cfg!(target_os = "macos") {
            // Asked again here, the last door before a command line is split;
            // the installs ask it first, before anything is fetched.
            preflight(m)?;
            let mecha =
                std::env::current_exe().context("finding the mecha binary for the agent")?;
            agent_text(stem, unit, &m.home, &m.mecha_home, &mecha)?
        } else {
            unit.to_string()
        },
    );
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

/// Holds a sidecar's revision to what its install writes: the hash of every
/// rendered definition, launcher, drop-in and program file, pinned beside the
/// number. A change to any of them fails here until the number in [`REVS`] is
/// raised — so an installed service is never quietly older than the build
/// that reads it. Not the dependency locks or the pinned commits — and
/// nothing else detects a change there either: a lock or commit bump alone
/// reaches no installed machine until its install is run again by hand
/// (`enable --reinstall`). A known gap (review of #691, pass 7), not a
/// handled case. The same holds for the router's catalog sections
/// (`router_unit::presets_with_catalog`): only the base `presets_text` is
/// hashed, so a change to an installed catalog entry's pins or lines reaches
/// a router at its next install (`mecha model add`, `setup chat`), and no
/// revision reports it older before then (review of #727).
#[cfg(test)]
pub(crate) fn assert_rev(id: &str, pinned: (u32, &str), rendered: &[&str]) {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for r in rendered {
        h.update(r.as_bytes());
        h.update([0u8]);
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        rev(id),
        Some(pinned.0),
        "{id}'s pinned revision is not user_service::REVS's: pin the number REVS has"
    );
    assert_eq!(
        got,
        pinned.1,
        "what {id}'s install writes has changed: raise its number in user_service::REVS to {}, \
         and pin ({}, \"{got}\") here",
        pinned.0 + 1,
        pinned.0 + 1
    );
}

/// Fixed inputs for [`agent_text`] in the revision tests: a Mac's paths.
#[cfg(test)]
pub(crate) fn test_agent_text(stem: &str, unit: &str) -> String {
    agent_text(
        stem,
        unit,
        Path::new("/Users/a"),
        Path::new("/Users/a/.mecha"),
        Path::new("/Users/a/.cargo/bin/mecha"),
    )
    .unwrap()
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

    /// Every sidecar whose evidence is a service definition has a revision,
    /// and every revision names a sidecar: one missing from [`REVS`] is
    /// stamped with nothing and read as current forever (review of #691,
    /// pass 5).
    #[test]
    fn every_sidecar_with_services_has_a_revision() {
        use crate::sidecar::{Evidence, SIDECARS};
        for s in SIDECARS {
            let services = s
                .evidence
                .iter()
                .any(|e| matches!(e, Evidence::UserUnit(_)));
            assert_eq!(
                super::rev(s.id).is_some(),
                services,
                "{}: a sidecar with services needs a row in REVS, and only one",
                s.id
            );
        }
        for (id, _) in super::REVS {
            assert!(SIDECARS.iter().any(|s| s.id == *id), "{id} is no sidecar");
        }
    }

    /// A stamp goes where each format keeps a comment and reads back; a
    /// definition with none, or another sidecar's, reads as none.
    #[test]
    fn a_stamp_reads_back_in_both_formats() {
        let unit = super::stamp("stt", "[Unit]\nDescription=x\n");
        assert!(unit.starts_with("# mecha-service-rev: stt "), "{unit}");
        assert_eq!(super::rev_of(&unit, "stt"), super::rev("stt"));
        let plist = super::stamp(
            "comfyui",
            "<?xml version=\"1.0\"?>\n<!DOCTYPE plist>\n<plist></plist>\n",
        );
        assert!(plist.starts_with("<?xml version=\"1.0\"?>\n<!-- mecha-service-rev: comfyui "));
        assert_eq!(super::rev_of(&plist, "comfyui"), super::rev("comfyui"));
        assert_eq!(super::rev_of("[Unit]\n", "stt"), None);
        assert_eq!(super::rev_of(&unit, "comfyui"), None);
        assert_eq!(super::stamp("layout", "x"), "x", "no services, no stamp");
    }
}
