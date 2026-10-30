//! Code mode's interpreter, executed rather than described: a pinned monty
//! worker inside bwrap, driven through the `Interpreter` trait.
//!
//! Needs the pinned worker (`mecha_core::interpreter::worker`) and a bwrap
//! that can create namespaces. The worker is found at `MECHA_MONTY_WORKER`,
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

/// bwrap that can actually build the worker's namespaces here.
fn bwrap_works() -> bool {
    std::process::Command::new("bwrap")
        .args([
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--ro-bind",
            "/",
            "/",
            "true",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn bwrap_sandbox() -> Sandbox {
    Sandbox::new(SandboxConfig {
        kind: Backend::Bwrap,
        ..Default::default()
    })
}

/// A confined interpreter with these backstops, or `None` (a skip).
async fn interpreter(backstops: Backstops) -> Option<Monty> {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some())
        || unavailable("a working bwrap", bwrap_works())
    {
        return None;
    }
    let confinement = Confinement::for_sandbox(&bwrap_sandbox()).expect("bwrap confinement");
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

/// Every process under `pid`, itself included.
fn process_tree(pid: u32) -> Vec<u32> {
    let mut out = vec![pid];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{p}/task")) {
            for t in tasks.flatten() {
                let kids = std::fs::read_to_string(t.path().join("children")).unwrap_or_default();
                out.extend(
                    kids.split_whitespace()
                        .filter_map(|k| k.parse::<u32>().ok()),
                );
            }
        }
        i += 1;
    }
    out
}

/// The second wall, measured on the live worker rather than read off an
/// argv: the process running the program has no environment, no network
/// namespace of ours, and a filesystem holding nothing but its binary.
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
    let tree = process_tree(launcher);
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
    let tree = process_tree(program.worker_pid().unwrap());
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
        let state = std::fs::read_to_string(format!("/proc/{p}/stat")).unwrap_or_default();
        assert!(
            state.is_empty() || state.contains(") Z "),
            "process {p} outlived its program: {state}"
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
/// fails preflight with a refusal, never a fallback to running it bare. A
/// backend that spawned the worker directly would pass this preflight.
#[tokio::test]
async fn preflight_refuses_when_the_sandbox_cannot_confine_the_worker() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some()) {
        return;
    }
    let w = w.unwrap();
    for broken in ["/bin/false", "/nonexistent/bwrap"] {
        let interp = Monty::new(&w, &Confinement::bwrap_at(broken), Backstops::default(), 1)
            .await
            .unwrap();
        let err = interp.preflight().await.unwrap_err().to_string();
        assert!(
            err.contains("does not run inside the sandbox"),
            "{broken}: {err}"
        );
    }
    if !unavailable("a working bwrap", bwrap_works()) {
        let ok = Monty::new(
            &w,
            &Confinement::for_sandbox(&bwrap_sandbox()).unwrap(),
            Backstops::default(),
            1,
        )
        .await
        .unwrap();
        ok.preflight()
            .await
            .expect("preflight under a working bwrap");
    }
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
    let tree = process_tree(program.worker_pid().unwrap());
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

/// A worker goes back to the pool after a clean or a raised ending, and
/// the next program must not see the last one's state.
#[tokio::test]
async fn a_reused_worker_starts_clean() {
    let w = pinned_worker().await;
    if unavailable("the pinned monty worker", w.is_some())
        || unavailable("a working bwrap", bwrap_works())
    {
        return;
    }
    let conf = Confinement::for_sandbox(&bwrap_sandbox()).unwrap();
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
/// tight loop's throughput, through the shipped path (pinned worker, bwrap).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn m1_measure() {
    let label = std::env::var("M1_LABEL").unwrap_or_else(|_| "alone".into());
    let backstops = Backstops {
        max_suspensions: 10_000_000,
        ..Backstops::default()
    };
    let w = pinned_worker().await.expect("the pinned worker");
    let conf = Confinement::for_sandbox(&bwrap_sandbox()).unwrap();

    // Raw process start, for context: the binary alone, then under bwrap.
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
        let ok = std::process::Command::new("bwrap")
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
            .unwrap();
        assert!(ok.status.success());
        boxed.push(t.elapsed().as_secs_f64() * 1e3);
    }

    // Cold: a fresh pool, so the program pays the spawn, bwrap and Configure.
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
        "  process start, bwrap `monty -c 1`:       median {:.1} ms (p90 {:.1})",
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
