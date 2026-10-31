//! Code mode's interpreter, executed rather than described: a pinned monty
//! worker inside the platform's sandbox — bwrap on Linux, Seatbelt on a Mac —
//! driven through the `Interpreter` trait. The same tests run on both; where
//! the kernel has to be asked something (a process's state, its environment,
//! its confinement), `support/process.rs` asks `/proc` or libproc.
//!
//! Needs the pinned worker (`mecha_core::interpreter::worker`) and a bwrap
//! that can create namespaces, or `sandbox-exec`. The worker is found at
//! `MECHA_MONTY_WORKER`,
//! else where `worker::ensure` installs it under `MECHA_HOME` (or
//! `~/.mecha`); `MECHA_TEST_FETCH_MONTY=1` fetches the pinned wheel into a
//! scratch home first, which is what CI does. Every path is verified
//! against the pin before anything runs it. Missing either is a skip, and
//! `MECHA_TEST_REQUIRE_BACKENDS=1` makes it a failure.
//!
//! `m1_measure` is the M1 measurement (CODE-MODE-DESIGN.md §10), ignored by
//! default: `cargo test --release -p mecha-core --features monty --test
//! code_interpreter -- --ignored --nocapture m1_measure`.

#![cfg(feature = "monty")]

#[path = "support/process.rs"]
mod process;
mod support;

use mecha_core::interpreter::monty::{Confinement, Monty};
use mecha_core::interpreter::worker::{self, VerifiedWorker};
use mecha_core::interpreter::{
    Backstop, Backstops, End, ExternalCall, Finished, HostRaise, Interpreter, Step, Tag, Value,
};
use mecha_core::sandbox::{Backend, Sandbox, SandboxConfig};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use support::*;
use tokio_util::sync::CancellationToken;

static FETCHED: tokio::sync::OnceCell<Option<PathBuf>> = tokio::sync::OnceCell::const_new();

/// The pinned worker, verified, or `None` with the reason printed.
async fn pinned_worker() -> Option<VerifiedWorker> {
    if let Some(p) = std::env::var_os("MECHA_MONTY_WORKER") {
        return Some(
            worker::verify(&PathBuf::from(p)).expect("MECHA_MONTY_WORKER is not the pinned worker"),
        );
    }
    if std::env::var_os("MECHA_TEST_FETCH_MONTY").is_some() {
        let path = FETCHED
            .get_or_init(|| async {
                let home = std::env::temp_dir().join("mecha-test-monty-home");
                match worker::ensure(&home, &mut |_| {}).await {
                    Ok(w) => Some(w.path().to_path_buf()),
                    Err(e) => {
                        eprintln!("fetching the pinned worker failed: {e:#}");
                        None
                    }
                }
            })
            .await
            .clone()?;
        return worker::verify(&path).ok();
    }
    let home = std::env::var_os("MECHA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".mecha")))?;
    worker::installed(&home).ok()
}

/// The platform's sandbox, able to run something here: bwrap that can
/// build the worker's namespaces, or `sandbox-exec` that applies a profile.
fn sandbox_works() -> bool {
    let mut c = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("/usr/bin/sandbox-exec");
        c.args(["-p", "(version 1)(allow default)", "/usr/bin/true"]);
        c
    } else {
        let mut c = std::process::Command::new("bwrap");
        c.args([
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--ro-bind",
            "/",
            "/",
            "true",
        ]);
        c
    };
    c.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

const SANDBOX: &str = if cfg!(target_os = "macos") {
    "a working sandbox-exec"
} else {
    "a working bwrap"
};

/// `[sandbox] kind = "bwrap"`. On a Mac the worker's confinement ignores it
/// and is Seatbelt, which is what [`confinement`] then returns.
fn bwrap_sandbox() -> Sandbox {
    Sandbox::new(SandboxConfig {
        kind: Backend::Bwrap,
        ..Default::default()
    })
}

/// The platform's confinement for a worker, by the construction path.
fn confinement() -> Confinement {
    Confinement::for_sandbox(&bwrap_sandbox()).expect("the platform's worker confinement")
}

/// A confined interpreter with these backstops, or `None` (a skip).
async fn interpreter(backstops: Backstops) -> Option<Monty> {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) || unavailable(SANDBOX, sandbox_works())
    {
        return None;
    }
    let confinement = confinement();
    Some(
        Monty::new(&w.unwrap(), &confinement, backstops, 4)
            .await
            .expect("building the interpreter"),
    )
}

/// Run `source` to the end, answering each call with `answer`; every call
/// that surfaced is returned beside how the program ended.
async fn run_with(
    interp: &Monty,
    source: &str,
    externals: &[&str],
    mut answer: impl FnMut(&ExternalCall) -> Result<Value, HostRaise>,
) -> (Finished, Vec<ExternalCall>) {
    let names: Vec<String> = externals.iter().map(|s| s.to_string()).collect();
    let (mut program, mut step) = interp
        .start(source, &names, CancellationToken::new())
        .await
        .expect("starting a program");
    let mut calls = Vec::new();
    loop {
        match step {
            Step::Done(f) => return (f, calls),
            Step::Call(call) => {
                let reply = answer(&call);
                calls.push(call);
                step = match reply {
                    Ok(v) => program.resume(v).await,
                    Err(r) => program.raise(r).await,
                }
                .expect("answering a call");
            }
        }
    }
}

#[tokio::test]
async fn a_registered_function_is_called_and_its_answer_returns_into_the_program() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let (f, calls) = run_with(
        &interp,
        "x = double(21, label='n')\nprint('got', x)\nx + 1",
        &["double"],
        |c| Ok(Value::from(c.args[0].as_i64().unwrap() * 2)),
    )
    .await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "double");
    assert_eq!(calls[0].args, vec![Value::from(21)]);
    assert_eq!(
        calls[0].kwargs,
        vec![("label".to_string(), Value::from("n"))]
    );
    assert_eq!(f.printed, "got 42\n");
    assert_eq!(
        f.end,
        End::Complete {
            value: Some(Value::from(43)),
            repr: "43".into()
        }
    );
}

/// A host raise lands where the call is, catchable, carrying one message;
/// uncaught, it ends the program as the program's own exception.
#[tokio::test]
async fn a_host_raise_is_catchable_and_an_uncaught_one_is_not_a_backstop() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let src = "try:\n    read(1)\nexcept RuntimeError as e:\n    print('caught', str(e))\nread(2)";
    let (f, _) = run_with(&interp, src, &["read"], |_| {
        Err(HostRaise::tagged(
            Tag::ToolError,
            "tool `read` returned an error",
        ))
    })
    .await;
    assert_eq!(
        f.printed,
        "caught ToolError: tool `read` returned an error\n"
    );
    let End::Raised(r) = f.end else {
        panic!("{:?}", f.end)
    };
    assert_eq!(r.exc_type, "RuntimeError");
    assert_eq!(
        r.message.as_deref(),
        Some("ToolError: tool `read` returned an error")
    );
}

/// CODE-MODE-RESEARCH.md's soundness question, AUDIT-RESEARCH.md §3.12's
/// first test: a program reaches the host only through a registered name.
/// Every route that might turn a string into a call — by name, `getattr`,
/// `globals`, `eval`, `__import__`, the builtins — must fail inside the
/// program, and no call under any other name may surface to the host. A
/// backend that resolved an unknown `NameLookup` to a host function, or
/// passed every `FunctionCall` through, fails here.
#[tokio::test]
async fn an_unregistered_function_cannot_be_reached_by_name_getattr_or_string() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let routes = [
        "secret()",
        "getattr(allowed, 'secret')()",
        "getattr(__builtins__, 'secret')()",
        "globals()['secret']()",
        "vars()['secret']()",
        "eval('secret()')",
        "exec('secret()')",
        "__import__('secret')",
        "type(allowed)('secret')()",
        "allowed.__globals__['secret']()",
        "getattr(allowed, '__self__').secret()",
    ];
    let mut src = String::from("refused = 0\n");
    for r in routes {
        src.push_str(&format!(
            "try:\n    {r}\n    print('REACHED', {r:?})\nexcept BaseException as e:\n    refused += 1\n"
        ));
    }
    src.push_str("allowed('ok')\nrefused");
    let (f, calls) = run_with(&interp, &src, &["allowed"], |_| Ok(Value::Null)).await;
    let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["allowed"], "a call surfaced under another name");
    assert!(!f.printed.contains("REACHED"), "{}", f.printed);
    assert_eq!(
        f.end,
        End::Complete {
            value: Some(Value::from(routes.len())),
            repr: routes.len().to_string()
        },
        "printed: {}",
        f.printed
    );
}

/// The other half of the soundness answer: monty 1.4 has `eval` and `exec`,
/// so a *registered* name is reachable from a string the program builds.
/// What a program can reach is its registered set, never its free names —
/// a static scan of the source for names would under-count, and this test
/// fails if anyone builds admission on one.
#[tokio::test]
async fn a_registered_function_is_reachable_by_a_built_string_so_free_names_are_not_the_reach_set()
{
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let (f, calls) = run_with(&interp, "eval('allo' + 'wed(7)')", &["allowed"], |_| {
        Ok(Value::from(1))
    })
    .await;
    assert_eq!(
        calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["allowed"]
    );
    assert!(matches!(f.end, End::Complete { .. }), "{:?}", f.end);
}

/// The first wall: files, the environment, the network and processes are
/// not in the interpreter, and what it asks the host for is refused.
#[tokio::test]
async fn a_program_cannot_open_a_file_read_the_environment_or_reach_the_network() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    // Statement-form imports: `__import__` is absent in monty 1.4, so a
    // route through it dies of `NameError` before reaching the host and
    // measures nothing (review of #765). Every route must fail with an
    // exception the host's refusal produces, never `NameError`.
    let routes = [
        "v = open('/etc/passwd').read()",
        "v = open('/tmp/mecha-code-mode-probe', 'w').write('x')",
        "import pathlib\n    v = pathlib.Path('/etc/hostname').read_text()",
        "import pathlib\n    v = list(pathlib.Path('/').iterdir())",
        "import os\n    v = os.getenv('HOME')",
        "import os\n    v = os.environ['PATH']",
        "import socket\n    v = socket",
        "import urllib.request\n    v = urllib",
        "import subprocess\n    v = subprocess",
        "import time\n    v = time.sleep(0.01)",
    ];
    let mut src = String::from("denied = []\n");
    for r in routes {
        src.push_str(&format!(
            "try:\n    {r}\n    print('ALLOWED', {r:?}, repr(v)[:40])\nexcept BaseException as e:\n    denied.append(type(e).__name__)\n"
        ));
    }
    src.push_str("print(denied)\nlen(denied)");
    let (f, calls) = run_with(&interp, &src, &[], |_| Ok(Value::Null)).await;
    assert!(calls.is_empty());
    assert!(!f.printed.contains("ALLOWED"), "{}", f.printed);
    assert!(
        !f.printed.contains("NameError") && !f.printed.contains("SyntaxError"),
        "a route never reached the host: {}",
        f.printed
    );
    assert_eq!(
        f.end,
        End::Complete {
            value: Some(Value::from(routes.len())),
            repr: routes.len().to_string()
        },
        "{}",
        f.printed
    );
    assert!(!std::path::Path::new("/tmp/mecha-code-mode-probe").exists());
}

/// The second wall, measured on the live worker rather than read off an
/// argv: the process running the program has no environment, no network
/// namespace of ours, and a filesystem holding nothing but its binary.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_worker_process_has_no_environment_no_network_and_no_filesystem() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let (mut program, step) = interp
        .start("hold()\n1", &["hold".into()], CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    let launcher = program.worker_pid().expect("a worker pid");
    let tree = process::tree(launcher);
    let worker = *tree
        .iter()
        .find(|p| {
            std::fs::read(format!("/proc/{p}/cmdline"))
                .is_ok_and(|c| c.starts_with(b"/monty\0subprocess"))
        })
        .unwrap_or_else(|| panic!("no /monty subprocess under {launcher}: {tree:?}"));
    let environ =
        std::fs::read(format!("/proc/{worker}/environ")).expect("reading the worker's environment");
    // bwrap sets `PWD` to the `--chdir` it was given, `/`; nothing else may
    // survive. Names only in the message: a value could be a secret.
    let names: Vec<String> = environ
        .split(|b| *b == 0)
        .filter(|kv| !kv.is_empty() && *kv != b"PWD=/")
        .map(|kv| {
            String::from_utf8_lossy(kv.split(|b| *b == b'=').next().unwrap_or_default())
                .into_owned()
        })
        .collect();
    assert!(
        names.is_empty(),
        "the worker inherited an environment: {names:?}"
    );
    let ns = |p: &str, kind: &str| std::fs::read_link(format!("/proc/{p}/ns/{kind}")).unwrap();
    assert_ne!(
        ns(&worker.to_string(), "net"),
        ns("self", "net"),
        "the worker shares our network"
    );
    let root: Vec<String> = std::fs::read_dir(format!("/proc/{worker}/root"))
        .expect("listing the worker's root")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        root,
        vec!["monty".to_string()],
        "the worker sees more than its binary"
    );
    let step = program.resume(Value::Null).await.unwrap();
    assert!(matches!(
        step,
        Step::Done(Finished {
            end: End::Complete { .. },
            ..
        })
    ));
}

/// The second wall on a Mac, measured on the live worker rather than read
/// off an argv — the honest twin of the `/proc` test above, which macOS has
/// no `/proc` for. The pool's child *is* the worker (the launcher's shell,
/// `env -i` and `sandbox-exec` each exec the next), so its pid is asked:
/// what it runs (libproc), what environment the kernel holds for it
/// (`KERN_PROCARGS2`), and what its profile denies (`sandbox_check`, the
/// kernel's own evaluation of the profile on that process). Each denial is
/// checked against an allowance on the same process and against this
/// unsandboxed test, so a check that answered "denied" for everything, or
/// for every process, fails here.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_live_worker_s_profile_denies_files_the_environment_services_and_the_network() {
    use mecha_core::sandbox::seatbelt::live;
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let w = pinned_worker().await.unwrap();
    let (mut program, step) = interp
        .start("hold()\n1", &["hold".into()], CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    let worker = program.worker_pid().expect("a worker pid");
    assert_eq!(
        process::exe(worker).as_deref(),
        Some(w.path()),
        "the pool's child is not the worker itself"
    );
    let names = process::environment_names(worker).expect("reading the worker's environment");
    assert!(
        names.is_empty(),
        "the worker inherited an environment: {names:?}"
    );
    // The control: the same parse of this test's own `KERN_PROCARGS2` finds
    // what is in this test's environment, so an empty answer above is the
    // worker's, not a parser that lost its place (review of #769).
    let mine = process::environment_names(std::process::id()).expect("reading this test's");
    let expected: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .collect();
    assert!(
        !mine.is_empty() && mine.iter().all(|n| expected.contains(n)),
        "the parser misreads this test's own environment: {mine:?}"
    );
    assert!(
        expected.iter().any(|n| n == "PATH") && mine.iter().any(|n| n == "PATH"),
        "the parser did not find PATH in this test's environment: {mine:?}"
    );

    assert_eq!(
        live::is_sandboxed(worker),
        Some(true),
        "not under a profile"
    );
    let me = std::process::id();
    assert_eq!(live::is_sandboxed(me), Some(false), "the check is vacuous");

    let canon = |p: &str| std::path::Path::new(p).canonicalize().unwrap();
    let home = dirs::home_dir().expect("a home").canonicalize().unwrap();
    let tmp = std::env::temp_dir().canonicalize().unwrap();
    // An existing file to read, write and unlink, kept until every answer
    // is in: `sandbox_check` reads a path that does not exist as denied even
    // for this unsandboxed test (macOS 26), which would measure nothing.
    // `process-exec` is asked as `process-exec*`; the bare name is an error
    // to `sandbox_check`.
    let existing = tmp.join(format!("mecha-code-mode-probe-{}", std::process::id()));
    std::fs::write(&existing, b"x").unwrap();
    let denied: Vec<(&str, std::path::PathBuf)> = vec![
        ("file-read-data", canon("/etc/passwd")),
        ("file-read-data", home.clone()),
        ("file-read-data", home.join(".ssh")),
        ("file-read-data", canon("/usr/lib")),
        ("file-read-data", existing.clone()),
        ("file-read-metadata", home.clone()),
        ("file-write-data", existing.clone()),
        ("file-write-data", canon("/dev/null")),
        ("file-write-unlink", existing.clone()),
        ("process-exec*", canon("/bin/sh")),
        ("process-exec*", canon("/usr/bin/sandbox-exec")),
    ];
    let ops = [
        "network-outbound",
        "network-inbound",
        "network-bind",
        "process-fork",
    ];
    let services = [
        "com.apple.cfprefsd.daemon",
        "com.apple.SecurityServer",
        "com.apple.system.opendirectoryd.libinfo",
        "com.apple.system.notification_center",
        "com.apple.logd",
    ];
    // The whole reading first, so one run shows every answer.
    let mut table = Vec::new();
    for (op, path) in &denied {
        table.push(format!(
            "  {op:<20} {:<60} worker {:?}  this test {:?}",
            path.display(),
            live::denies_path(worker, op, path),
            live::denies_path(me, op, path)
        ));
    }
    for op in ops {
        table.push(format!(
            "  {op:<20} {:<60} worker {:?}  this test {:?}",
            "",
            live::denies(worker, op),
            live::denies(me, op)
        ));
    }
    for s in services {
        table.push(format!(
            "  {:<20} {s:<60} worker {:?}  this test {:?}",
            "mach-lookup",
            live::denies_service(worker, s),
            live::denies_service(me, s)
        ));
    }
    for (op, path) in [
        ("file-read-data", std::path::Path::new("/")),
        ("process-exec*", w.path()),
    ] {
        table.push(format!(
            "  {op:<20} {:<60} worker {:?}  (allowed by the profile)",
            path.display(),
            live::denies_path(worker, op, path)
        ));
    }
    let table = table.join("\n");
    println!("the live worker's profile, asked of the kernel:\n{table}");
    for (op, path) in &denied {
        assert_eq!(
            live::denies_path(worker, op, path),
            Some(true),
            "the worker may {op} {}\n{table}",
            path.display()
        );
        assert_eq!(
            live::denies_path(me, op, path),
            Some(false),
            "{op} {} is denied to this test too, so the check measures nothing\n{table}",
            path.display()
        );
    }
    // What the profile does allow reads as allowed on the same process.
    assert_eq!(
        live::denies_path(worker, "file-read-data", std::path::Path::new("/")),
        Some(false),
        "{table}"
    );
    assert_eq!(
        live::denies_path(worker, "process-exec*", w.path()),
        Some(false),
        "{table}"
    );
    std::fs::remove_file(&existing).ok();
    for op in ops {
        assert_eq!(
            live::denies(worker, op),
            Some(true),
            "the worker may {op}\n{table}"
        );
    }
    for service in services {
        assert_eq!(
            live::denies_service(worker, service),
            Some(true),
            "the worker may look up {service}\n{table}"
        );
    }
    let step = program.resume(Value::Null).await.unwrap();
    assert!(matches!(
        step,
        Step::Done(Finished {
            end: End::Complete { .. },
            ..
        })
    ));
}

fn short(wall: u64) -> Backstops {
    Backstops {
        wall_clock: Duration::from_secs(wall),
        ..Backstops::default()
    }
}

/// The wall clock ends a spinning program from outside: `except
/// BaseException` does not catch it, and the result names it.
#[tokio::test]
async fn the_wall_clock_backstop_fires_uncatchably_and_is_named() {
    let Some(interp) = interpreter(short(2)).await else {
        return;
    };
    let t = Instant::now();
    let (f, _) = run_with(
        &interp,
        "print('spinning')\ntry:\n    while True:\n        pass\nexcept BaseException:\n    print('caught')",
        &[],
        |_| Ok(Value::Null),
    )
    .await;
    assert_eq!(
        f.end,
        End::Backstop(Backstop::WallClock(Duration::from_secs(2)))
    );
    assert_eq!(f.printed, "spinning\n");
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
}

/// The clock counts the host's time too: a program parked on a call past
/// its wall clock is over when the host comes back.
#[tokio::test]
async fn host_time_counts_against_the_wall_clock() {
    let Some(interp) = interpreter(short(1)).await else {
        return;
    };
    let (mut program, step) = interp
        .start("slow()\n1", &["slow".into()], CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let Step::Done(f) = program.resume(Value::Null).await.unwrap() else {
        panic!()
    };
    assert_eq!(
        f.end,
        End::Backstop(Backstop::WallClock(Duration::from_secs(1)))
    );
}

/// A worker that stops answering mid-turn — the hang monty's own limits
/// cannot see, since a stopped process runs no clock — is killed at the
/// wall clock. Three deadlines can end it from outside — this module's own,
/// `monty-pool`'s `request_timeout`, and the pool's parent-side enforcement
/// of `max_feed_duration` — and a backend with none of them (the pool's
/// defaults) hangs here until the outer timeout fails the test.
#[tokio::test]
async fn a_worker_that_stops_answering_is_killed_at_the_wall_clock() {
    let Some(interp) = interpreter(short(2)).await else {
        return;
    };
    let (mut program, step) = interp
        .start(
            "pause()\nwhile True:\n    pass",
            &["pause".into()],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    let tree = process::tree(program.worker_pid().unwrap());
    for p in &tree {
        // SAFETY: plain signal syscalls on processes this test's pool spawned.
        unsafe { libc::kill(*p as i32, libc::SIGSTOP) };
    }
    let step = tokio::time::timeout(Duration::from_secs(30), program.resume(Value::Null))
        .await
        .expect("a stopped worker hung the program past every deadline")
        .unwrap();
    let Step::Done(f) = step else { panic!() };
    assert_eq!(
        f.end,
        End::Backstop(Backstop::WallClock(Duration::from_secs(2)))
    );
    // The worker is gone, not left stopped.
    tokio::time::sleep(Duration::from_millis(300)).await;
    for p in tree {
        assert!(
            process::is_gone(p),
            "process {p} outlived its program: {:?}",
            process::state(p)
        );
    }
}

#[tokio::test]
async fn the_memory_backstop_fires_uncatchably_and_is_named() {
    let backstops = Backstops {
        max_memory: 32 * 1024 * 1024,
        ..short(60)
    };
    let Some(interp) = interpreter(backstops).await else {
        return;
    };
    let (f, _) = run_with(
        &interp,
        "x = []\ntry:\n    while True:\n        x.append('a' * 100000)\nexcept BaseException:\n    print('caught')",
        &[],
        |_| Ok(Value::Null),
    )
    .await;
    assert_eq!(
        f.end,
        End::Backstop(Backstop::Memory(32 * 1024 * 1024)),
        "printed: {}",
        f.printed
    );
    assert!(!f.printed.contains("caught"));
}

/// The memory backstop measured against what the worker actually holds,
/// not only against how the program ended (CODE-MODE-DESIGN.md §5). A Mac
/// has no cgroup behind it — `[sandbox] memory_mb` never reaches a Seatbelt
/// worker, and macOS refuses or ignores the rlimits that would stand in
/// (§5) — so monty's own allocator ceiling is the only bound there, and a
/// program that allocated past it before the check fired would hold the
/// machine's memory for as long as the wall clock let it.
///
/// Each program tries to take far more than the limit, gradually or in one
/// operation, while a sampler reads the worker's lifetime peak (`VmHWM`, or
/// macOS's `ri_lifetime_max_phys_footprint`, which counts compressed and
/// swapped pages too). Every one must end without completing, and none may
/// have held more than the limit plus the worker's own baseline and slack.
/// The table prints, so CI's log is the measurement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_memory_backstop_bounds_what_the_worker_holds() {
    const LIMIT: usize = 64 * 1024 * 1024;
    // The worker's own resident baseline (measured: about 2 MiB on macOS,
    // 6 MiB on Linux), monty's 4 MiB allocator headroom, and room for a
    // sample's lag.
    const SLACK: u64 = 64 * 1024 * 1024;
    let backstops = Backstops {
        max_memory: LIMIT,
        ..short(30)
    };
    let Some(interp) = interpreter(backstops).await else {
        return;
    };
    let programs = [
        (
            "a list grown 100 kB at a time",
            "x = []\nwhile True:\n    x.append('a' * 100000)",
        ),
        ("a string doubled", "s = 'ab'\nwhile True:\n    s = s + s"),
        (
            "a dict grown",
            "d = {}\ni = 0\nwhile True:\n    d[i] = i\n    i += 1",
        ),
        ("one 2 GiB string", "x = 'a' * (2 * 1024 ** 3)"),
        ("one 2 GiB list", "x = [0] * (256 * 1024 ** 2)"),
        ("one 2 GiB tuple", "x = (1,) * (256 * 1024 ** 2)"),
        ("one 1 GiB integer", "x = 2 ** (8 * 1024 ** 3)"),
        ("a range listed", "x = list(range(300000000))"),
        (
            "a replace that multiplies",
            "x = ('a' * 1000).replace('a', 'b' * 1000000)",
        ),
        ("a 2 GiB ljust", "x = 'a'.ljust(2 * 1024 ** 3)"),
        (
            "a join of a generator",
            "x = ''.join(['a' * 1000 for i in range(2000000)])",
        ),
    ];
    let mut rows = Vec::new();
    let mut over = Vec::new();
    for (name, body) in programs {
        let (mut program, step) = interp
            .start(
                &format!("hold()\n{body}"),
                &["hold".into()],
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(matches!(step, Step::Call(_)), "{name}: {step:?}");
        let tree = process::tree(program.worker_pid().unwrap());
        let baseline = tree.iter().filter_map(|p| process::peak_bytes(*p)).max();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sampler = {
            let stop = stop.clone();
            let tree = tree.clone();
            tokio::spawn(async move {
                let mut peak = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for p in &tree {
                        peak = peak.max(process::peak_bytes(*p).unwrap_or(0));
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                peak
            })
        };
        let t = Instant::now();
        let step = program.resume(Value::Null).await.unwrap();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let peak = sampler.await.unwrap();
        let Step::Done(f) = step else {
            panic!("{name}: {step:?}")
        };
        let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
        rows.push(format!(
            "  {name:<30} peak {:>7.1} MiB (start {:>5.1})  {:>6.0} ms  {:?}",
            mib(peak),
            mib(baseline.unwrap_or(0)),
            t.elapsed().as_secs_f64() * 1e3,
            f.end
        ));
        if matches!(f.end, End::Complete { .. }) || peak > LIMIT as u64 + SLACK {
            over.push(name);
        }
    }
    println!(
        "memory backstop at {} MiB, {}:\n{}",
        LIMIT / (1024 * 1024),
        std::env::consts::OS,
        rows.join("\n")
    );
    assert!(
        over.is_empty(),
        "these programs completed or held more than {} MiB + {} MiB: {over:?}\n{}",
        LIMIT / (1024 * 1024),
        SLACK / (1024 * 1024),
        rows.join("\n")
    );
}

#[tokio::test]
async fn the_suspension_backstop_fires_uncatchably_and_is_named() {
    let backstops = Backstops {
        max_suspensions: 5,
        ..short(60)
    };
    let Some(interp) = interpreter(backstops).await else {
        return;
    };
    let (f, calls) = run_with(
        &interp,
        "try:\n    for i in range(100):\n        f(i)\nexcept BaseException:\n    print('caught')",
        &["f"],
        |c| Ok(c.args[0].clone()),
    )
    .await;
    assert_eq!(f.end, End::Backstop(Backstop::Suspensions(5)));
    assert_eq!(calls.len(), 5);
    assert!(!f.printed.contains("caught"));
}

/// Monty raises a catchable `RecursionError` at the depth limit, as CPython
/// does — so recursion is bounded but is not a backstop. Uncaught, it ends
/// the program as its own exception.
#[tokio::test]
async fn the_recursion_limit_holds_and_raises_a_catchable_recursion_error() {
    let backstops = Backstops {
        max_recursion_depth: 50,
        ..short(60)
    };
    let Some(interp) = interpreter(backstops).await else {
        return;
    };
    let def = "def r(n):\n    return r(n + 1)\n";
    let (f, _) = run_with(
        &interp,
        &format!("{def}try:\n    r(0)\nexcept RecursionError:\n    print('caught')"),
        &[],
        |_| Ok(Value::Null),
    )
    .await;
    assert_eq!(f.printed, "caught\n");
    let (f, _) = run_with(&interp, &format!("{def}r(0)"), &[], |_| Ok(Value::Null)).await;
    let End::Raised(r) = f.end else {
        panic!("{:?}", f.end)
    };
    assert_eq!(r.exc_type, "RecursionError");
}

/// A program that raises a backstop's own message early is the program's
/// exception, not a backstop: the host checks what it can see for itself.
#[tokio::test]
async fn a_program_cannot_impersonate_a_backstop() {
    let backstops = Backstops {
        max_suspensions: 5,
        ..short(60)
    };
    let Some(interp) = interpreter(backstops).await else {
        return;
    };
    for src in [
        "raise TimeoutError('feed time limit exceeded: 61s > 60s')",
        "raise RuntimeError('suspension limit 5 exceeded')",
        // The budget spent first, so the host's own count is at the limit.
        "for i in range(5):\n    f(i)\nraise RuntimeError('suspension limit 5 exceeded')",
    ] {
        let (f, _) = run_with(&interp, src, &["f"], |_| Ok(Value::Null)).await;
        assert!(matches!(f.end, End::Raised(_)), "{src}: {:?}", f.end);
    }
}

/// Stop ends a spinning program within the grace period, keeping what it
/// printed; a Stop while the host holds a call ends it without resuming.
#[tokio::test]
async fn cancellation_stops_a_program_promptly() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let cancel = CancellationToken::new();
    let (mut program, step) = interp
        .start(
            "print('started')\nwait()\nwhile True:\n    pass",
            &["wait".into()],
            cancel.clone(),
        )
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c.cancel();
    });
    let t = Instant::now();
    let Step::Done(f) = program.resume(Value::Null).await.unwrap() else {
        panic!()
    };
    let took = t.elapsed();
    assert_eq!(f.end, End::Cancelled);
    assert_eq!(f.printed, "started\n");
    let grace = Backstops::default().cancel_grace;
    assert!(
        took < Duration::from_millis(300) + grace + Duration::from_secs(1),
        "{took:?}"
    );

    let cancel = CancellationToken::new();
    let (mut program, step) = interp
        .start("wait()\nprint('resumed')", &["wait".into()], cancel.clone())
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    cancel.cancel();
    let Step::Done(f) = program.resume(Value::Null).await.unwrap() else {
        panic!()
    };
    assert_eq!(f.end, End::Cancelled);
    assert_eq!(f.printed, "");
}

/// "Verified" means at this pool's start: a `VerifiedWorker` whose file
/// changed after `verify` ran is refused by `Monty::new` (review of #765).
#[tokio::test]
async fn a_pool_start_hashes_the_worker_again() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) {
        return;
    }
    let dir = tmpdir("monty-rehash");
    let copy = dir.join("monty");
    std::fs::copy(w.unwrap().path(), &copy).unwrap();
    let verified = worker::verify(&copy).unwrap();
    // Same length, one byte different: only a hash tells them apart.
    let mut bytes = std::fs::read(&copy).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&copy, bytes).unwrap();
    let err = Monty::new(
        &verified,
        &Confinement::bwrap_at("/bin/false"),
        Backstops::default(),
        1,
    )
    .await
    .err()
    .expect("a changed worker started a pool");
    assert!(
        err.to_string().contains("does not hash to the pinned"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The silently-degrading guard: a sandbox that cannot start the worker
/// fails preflight with a refusal, never a fallback to running it bare. On
/// Linux a backend that spawned the worker directly would pass this
/// preflight; on a Mac it is caught too, below.
#[tokio::test]
async fn preflight_refuses_when_the_sandbox_cannot_confine_the_worker() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) {
        return;
    }
    let w = w.unwrap();
    let broken: Vec<Confinement> = if cfg!(target_os = "macos") {
        ["/usr/bin/false", "/nonexistent/sandbox-exec"]
            .map(Confinement::seatbelt_at)
            .into()
    } else {
        ["/bin/false", "/nonexistent/bwrap"]
            .map(Confinement::bwrap_at)
            .into()
    };
    for c in broken {
        let interp = Monty::new(&w, &c, Backstops::default(), 1).await.unwrap();
        let err = interp.preflight().await.unwrap_err().to_string();
        assert!(
            err.contains("does not run inside the sandbox"),
            "{c:?}: {err}"
        );
    }
    if !unavailable(SANDBOX, sandbox_works()) {
        let ok = Monty::new(&w, &confinement(), Backstops::default(), 1)
            .await
            .unwrap();
        ok.preflight()
            .await
            .expect("preflight under the platform's sandbox");
    }
}

/// A `sandbox-exec` that runs the worker but drops its profile — the
/// worker answers, every program would succeed, and nothing confines it.
/// Preflight asks the kernel about the live worker and refuses it; a
/// preflight that only ran a program would pass it.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn preflight_refuses_a_worker_that_runs_unconfined() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) {
        return;
    }
    let dir = tmpdir("monty-bare");
    let fake = dir.join("sandbox-exec");
    // `-p <profile>` dropped, then the worker exec'd bare.
    std::fs::write(&fake, "#!/bin/sh\nshift 2\nexec \"$@\"\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let interp = Monty::new(
        &w.unwrap(),
        &Confinement::seatbelt_at(&fake),
        Backstops::default(),
        1,
    )
    .await
    .unwrap();
    // It does run programs: only the live check tells it apart.
    let (_p, step) = interp
        .start("21 * 2", &[], CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(&step, Step::Done(Finished { end: End::Complete { value: Some(v), .. }, .. }) if *v == 42),
        "{step:?}"
    );
    let err = interp.preflight().await.unwrap_err().to_string();
    assert!(err.contains("not under a Seatbelt profile"), "{err}");
    assert!(err.contains("refusing to run programs unconfined"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A worker killed under the program — what a cgroup memory limit does —
/// ends it as a named backstop, never as the backend failing (review of
/// #765: `Err` is promised to be for nothing the program did).
#[tokio::test]
async fn a_worker_killed_under_a_program_ends_it_as_a_backstop() {
    let Some(interp) = interpreter(Backstops::default()).await else {
        return;
    };
    let (mut program, step) = interp
        .start(
            "hold()\nwhile True:\n    pass",
            &["hold".into()],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    // Killed while it spins, so the death lands mid-turn.
    let tree = process::tree(program.worker_pid().unwrap());
    let killer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for p in tree {
            // SAFETY: plain signal syscalls on processes this test's pool spawned.
            unsafe { libc::kill(p as i32, libc::SIGKILL) };
        }
    });
    let step = program
        .resume(Value::Null)
        .await
        .expect("a killed worker is an ending, not an Err");
    killer.await.unwrap();
    let Step::Done(f) = step else { panic!() };
    assert!(
        matches!(f.end, End::Backstop(Backstop::WorkerKilled { .. })),
        "{:?}",
        f.end
    );
}

/// A Stop while every worker is busy ends the waiting program at once,
/// not after the pool's 60 s checkout timeout (review of #765).
#[tokio::test]
async fn cancellation_reaches_a_program_waiting_for_a_worker() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) || unavailable(SANDBOX, sandbox_works())
    {
        return;
    }
    let conf = confinement();
    let interp = Monty::new(&w.unwrap(), &conf, Backstops::default(), 1)
        .await
        .unwrap();
    // The pool's only worker, held by a program parked on a call.
    let (_held, step) = interp
        .start("hold()", &["hold".into()], CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(step, Step::Call(_)));
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        c.cancel();
    });
    let t = Instant::now();
    let (_p, step) = tokio::time::timeout(Duration::from_secs(30), interp.start("1", &[], cancel))
        .await
        .expect("a Stop waited out the checkout")
        .unwrap();
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    let Step::Done(f) = step else { panic!() };
    assert_eq!(f.end, End::Cancelled);
}

/// A worker goes back to the pool after a clean or a raised ending, and
/// the next program must not see the last one's state.
#[tokio::test]
async fn a_reused_worker_starts_clean() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) || unavailable(SANDBOX, sandbox_works())
    {
        return;
    }
    let conf = confinement();
    // One worker, so every program below runs on the same one.
    let interp = Monty::new(&w.unwrap(), &conf, Backstops::default(), 1)
        .await
        .unwrap();
    let (f, _) = run_with(&interp, "leak = 1\nleak", &[], |_| Ok(Value::Null)).await;
    assert!(matches!(f.end, End::Complete { .. }));
    let (f, _) = run_with(&interp, "leak2 = 2\nraise ValueError('x')", &[], |_| {
        Ok(Value::Null)
    })
    .await;
    assert!(matches!(f.end, End::Raised(_)));
    for name in ["leak", "leak2"] {
        let (f, _) = run_with(&interp, name, &[], |_| Ok(Value::Null)).await;
        let End::Raised(r) = f.end else {
            panic!("`{name}` survived into the next program: {:?}", f.end)
        };
        assert_eq!(r.exc_type, "NameError");
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// M1 (CODE-MODE-DESIGN.md §10): worker start, per-call overhead and a
/// tight loop's throughput, through the shipped path (pinned worker, the
/// platform's sandbox).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn m1_measure() {
    let label = std::env::var("M1_LABEL").unwrap_or_else(|_| "alone".into());
    let backstops = Backstops {
        max_suspensions: 10_000_000,
        ..Backstops::default()
    };
    let w = pinned_worker().await.expect("the pinned worker");
    let conf = confinement();

    // Raw process start, for context: the binary alone, then confined.
    let mut bare = Vec::new();
    let mut boxed = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let ok = std::process::Command::new(w.path())
            .args(["-c", "1"])
            .output()
            .unwrap();
        assert!(ok.status.success());
        bare.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let ok = if cfg!(target_os = "macos") {
            std::process::Command::new("/usr/bin/sandbox-exec")
                .arg("-p")
                .arg(mecha_core::sandbox::seatbelt::worker_profile(w.path()).unwrap())
                .arg(w.path())
                .args(["-c", "1"])
                .env_clear()
                .output()
                .unwrap()
        } else {
            std::process::Command::new("bwrap")
                .args([
                    "--unshare-all",
                    "--unshare-user",
                    "--disable-userns",
                    "--clearenv",
                    "--ro-bind",
                ])
                .arg(w.path())
                .args(["/monty", "--", "/monty", "-c", "1"])
                .output()
                .unwrap()
        };
        assert!(ok.status.success());
        boxed.push(t.elapsed().as_secs_f64() * 1e3);
    }

    // Cold: a fresh pool, so the program pays the spawn, the sandbox and
    // Configure.
    let mut cold = Vec::new();
    for _ in 0..20 {
        let interp = Monty::new(&w, &conf, backstops, 1).await.unwrap();
        let t = Instant::now();
        let (_p, step) = interp
            .start("1", &[], CancellationToken::new())
            .await
            .unwrap();
        cold.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(matches!(step, Step::Done(_)));
    }

    // Warm: one pool, its worker reused (checkout, Configure, feed, Reset).
    let interp = Monty::new(&w, &conf, backstops, 1).await.unwrap();
    interp.preflight().await.unwrap();
    let mut warm = Vec::new();
    for _ in 0..200 {
        let t = Instant::now();
        let (_p, step) = interp
            .start("1", &[], CancellationToken::new())
            .await
            .unwrap();
        warm.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(matches!(step, Step::Done(_)));
    }

    // Per call: a tight loop of host calls answered at once, against the
    // same loop calling a function defined in the program.
    const N: usize = 5000;
    let mut per_call = Vec::new();
    let mut per_local = Vec::new();
    for _ in 0..5 {
        let src = format!("s = 0\nfor i in range({N}):\n    s += f(i)\ns");
        let t = Instant::now();
        let (f, calls) = run_with(&interp, &src, &["f"], |c| Ok(c.args[0].clone())).await;
        per_call.push(t.elapsed().as_secs_f64() * 1e6 / N as f64);
        assert_eq!(calls.len(), N);
        assert!(matches!(f.end, End::Complete { .. }));
        let src = format!("def g(x):\n    return x\ns = 0\nfor i in range({N}):\n    s += g(i)\ns");
        let t = Instant::now();
        let (f, _) = run_with(&interp, &src, &[], |_| Ok(Value::Null)).await;
        per_local.push(t.elapsed().as_secs_f64() * 1e6 / N as f64);
        assert!(matches!(f.end, End::Complete { .. }));
    }
    let call_us = median(&mut per_call);
    println!("M1 [{label}]");
    println!(
        "  process start, bare `monty -c 1`:        median {:.1} ms (p90 {:.1})",
        median(&mut bare),
        pct(&mut bare, 0.9)
    );
    println!(
        "  process start, confined `monty -c 1`:    median {:.1} ms (p90 {:.1})",
        median(&mut boxed),
        pct(&mut boxed, 0.9)
    );
    println!(
        "  cold program (spawn + Configure + feed): median {:.1} ms (p10 {:.1}, p90 {:.1}, n=20)",
        median(&mut cold),
        pct(&mut cold, 0.1),
        pct(&mut cold, 0.9)
    );
    println!(
        "  warm program (reused worker):            median {:.2} ms (p10 {:.2}, p90 {:.2}, n=200)",
        median(&mut warm),
        pct(&mut warm, 0.1),
        pct(&mut warm, 0.9)
    );
    println!(
        "  host call round trip:                    median {:.1} us/call ({:.0} calls/s, {N} calls x 5)",
        call_us,
        1e6 / call_us
    );
    println!(
        "  in-program call, same loop:              median {:.2} us/iteration",
        median(&mut per_local)
    );
}
