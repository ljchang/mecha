//! The macOS confinement: `sandbox-exec` with a Seatbelt profile built for
//! each call (`docs/DOCUMENT-EXTRACTION-DESIGN.md` §Seatbelt).
//!
//! **Never `shell`'s.** It confines the PDF renderer (poppler) and the layout
//! worker — programs that read one input and write one directory — and, with
//! a profile of its own, code mode's interpreter (below). `[sandbox]`
//! refuses it for `shell` and MCP servers: their confinement is what lets a
//! tool declare narrower capabilities, and neither profile has been measured
//! against a shell's reach (`Config::validate`).
//!
//! **Deny by default, then name what the call needs.** The profile starts
//! `(deny default)` over Apple's own base profile (`system.sb`: the dyld
//! cache, libSystem's own reads, the few system services every process
//! touches), and allows:
//!
//! - executing the program, by its path as named and as resolved;
//! - reading the system's libraries and data, the program and every library
//!   it loads — **found by reading the Mach-O's own load commands**, never
//!   guessed — and the Homebrew kegs they live in, with that prefix's font
//!   and poppler data;
//! - reading and writing the call's workspace (the scratch directory holding
//!   the input), and whatever the call names in `readable` / `writable`;
//! - no network: `(deny network*)` unless the call asks for it.
//!
//! The user's home is denied explicitly after the base profile, so nothing
//! `system.sb` allows can reach it; the call's own paths inside the home are
//! carved out of that deny with `require-not`, since a later allow does not
//! reliably override it ([`profile`]).
//!
//! Metadata (`stat`, `readlink`) is readable everywhere: dyld and path
//! resolution need it along every ancestor, and a name or a size is not a
//! file's contents — with no network and no write outside the scratch
//! directory, there is nowhere to send what a stat learns.
//!
//! **A path that cannot be written into a profile is refused**, never
//! mangled: SBPL strings take `\` and `"` escaped, and a control character or
//! a name that is not UTF-8 stops the call.
//!
//! **Code mode's worker has a profile of its own** ([`worker_profile`]),
//! separate from the documents profile above and used by
//! `interpreter::monty` alone. It is much narrower: no `system.sb`, no
//! metadata everywhere, no scratch directory, no write anywhere — exec of the
//! pinned binary, the root directory's listing and one named sysctl, each
//! found by measurement on macOS. `[sandbox]` never names it, so it confines no
//! `shell` and no MCP server ([`live`] reads what a running process's
//! profile allows, which is how the interpreter's preflight proves it holds).

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The system's own read-only trees. The dyld shared cache lives in the
/// cryptex on macOS 13+; fonts are read by poppler's fontconfig.
const SYSTEM_READ: &[&str] = &[
    "/usr/lib",
    "/usr/share",
    "/System",
    "/System/Volumes/Preboot/Cryptexes",
    "/Library/Fonts",
    "/private/var/db/timezone",
];

/// Devices every program opens.
const DEVICES: &[&str] = &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"];

/// Where a program is looked for when its name is bare: the caller's `PATH`,
/// then the package managers' prefixes, since a launchd agent's `PATH` is the
/// system's alone.
const FALLBACK_BIN: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

/// What one call may touch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Policy {
    /// The program as named (absolute), and as resolved, if they differ.
    pub exec: Vec<PathBuf>,
    /// Trees readable whole.
    pub read_trees: Vec<PathBuf>,
    /// Single files readable.
    pub read_files: Vec<PathBuf>,
    /// Trees readable and writable.
    pub write_trees: Vec<PathBuf>,
    /// The user's home: denied after the base profile.
    pub home: Option<PathBuf>,
    pub network: bool,
}

/// The profile text for `p`.
pub fn profile(p: &Policy) -> Result<String> {
    let mut out = String::from(
        "(version 1)\n\
         (deny default)\n\
         (import \"system.sb\")\n\
         (allow file-read-metadata)\n\
         (allow sysctl-read)\n\
         (allow signal (target self))\n",
    );
    let list = |kind: &str, paths: &[PathBuf]| -> Result<String> {
        paths
            .iter()
            .map(|p| Ok(format!(" ({kind} {})", quote(p)?)))
            .collect::<Result<String>>()
    };
    if p.exec.is_empty() {
        bail!("a confined call names no program");
    }
    out.push_str(&format!(
        "(allow process-exec{})\n",
        list("literal", &p.exec)?
    ));
    let system: Vec<PathBuf> = SYSTEM_READ.iter().map(PathBuf::from).collect();
    let devices: Vec<PathBuf> = DEVICES.iter().map(PathBuf::from).collect();
    out.push_str(&format!(
        "(allow file-read*{}{})\n",
        list("subpath", &system)?,
        list("literal", &devices)?
    ));
    out.push_str("(allow file-write* (literal \"/dev/null\"))\n");
    // The data volume is a second spelling of every user file — the home
    // under it reads as `/System/Volumes/Data/Users/<name>` — and `/System`
    // above is allowed whole. A call's own paths arrive canonical
    // (`/Users/…`, `/opt/homebrew/…`, `/private/var/…`), so nothing it needs
    // is spelled this way; denied outright (review of #682).
    out.push_str(
        "(deny file-read-data file-read-xattr file-write* (subpath \"/System/Volumes/Data\"))\n",
    );
    // The home's contents, xattrs and writes are denied — not metadata,
    // which path resolution needs along every ancestor of an interpreter
    // that lives in the home (the layout worker's, under `~/.mecha`). The
    // call's own paths inside the home are carved out of the denial itself
    // with `require-not`, never left to rule order: on macOS 26 a later
    // `file-read*` allow did not override this operation-specific deny, and
    // a named path in the home was refused (measured, macOS CI on #682).
    //
    // Two denials, not one: a read is carved out for every path the call
    // names, a write only for the trees it may write. One carve-out for both
    // lifted the write denial on paths the call may only read — the layout
    // worker's environment among them, which a process reading
    // attacker-chosen pixels must not be able to change (review of #682).
    if let Some(home) = &p.home {
        let except = |sets: &[(&str, &Vec<PathBuf>)]| -> Result<String> {
            let mut out = String::new();
            for (kind, paths) in sets {
                for path in paths.iter().filter(|x| x.starts_with(home)) {
                    out.push_str(&format!(" (require-not ({kind} {}))", quote(path)?));
                }
            }
            Ok(out)
        };
        let filter = |except: String| -> Result<String> {
            Ok(if except.is_empty() {
                format!("(subpath {})", quote(home)?)
            } else {
                format!("(require-all (subpath {}){except})", quote(home)?)
            })
        };
        let reads = except(&[
            ("subpath", &p.read_trees),
            ("subpath", &p.write_trees),
            ("literal", &p.read_files),
            ("literal", &p.exec),
        ])?;
        let writes = except(&[("subpath", &p.write_trees)])?;
        out.push_str(&format!(
            "(deny file-read-data file-read-xattr {})\n",
            filter(reads)?
        ));
        out.push_str(&format!("(deny file-write* {})\n", filter(writes)?));
    }
    // Never empty: `exec` was proven non-empty above.
    out.push_str(&format!(
        "(allow file-read*{}{}{})\n",
        list("subpath", &p.read_trees)?,
        list("literal", &p.read_files)?,
        list("literal", &p.exec)?
    ));
    if !p.write_trees.is_empty() {
        out.push_str(&format!(
            "(allow file-read* file-write*{})\n",
            list("subpath", &p.write_trees)?
        ));
    }
    out.push_str(if p.network {
        "(allow network*)\n"
    } else {
        "(deny network*)\n"
    });
    Ok(out)
}

/// The profile code mode's interpreter runs its worker under
/// (`interpreter::monty::Confinement::Seatbelt`). **Deny by default, and
/// allow only what the pinned worker was measured to need.** Everything a
/// program might reach for arrives at the host as an `OsCall` first and is
/// refused there, so this is the second wall, and it has to hold if the
/// first is escaped.
///
/// Measured on macOS 26.6 (arm64, CI): the worker traced under `(allow
/// default (with report))`, each grant then removed in turn and the worker
/// started without it. Three survive:
///
/// - `process-exec` of the worker, by its canonical path: `sandbox-exec`
///   applies the profile and then execs it. Nothing else may be executed,
///   so a worker cannot become a shell. Not `file-read*` of it: the kernel
///   maps the image, and the worker starts with its own bytes unreadable.
/// - `file-read-data` of `/` itself, a literal and not a subpath: the
///   worker opens the root directory at start and aborts (SIGABRT) before
///   `main` without it. It lists the root's entries' names; nothing under
///   them is readable.
/// - `sysctl-read` of `hw.pagesize_compat` alone, by name: Rust's runtime
///   sizes the main thread's guard page from it and panics at start without
///   it ("failed to allocate a guard page"). The other five the trace showed
///   (`kern.bootargs` and `security.mac.lockdown_mode_state` for dyld;
///   `kern.osproductversion`, `kern.iossupportversion` and
///   `kern.osvariant_status` for libSystem's version checks) are denied,
///   and the worker starts without them.
///
/// And nothing else:
///
/// - **No mach-lookup.** Serving programs, the worker reaches for two
///   services — opendirectoryd's `libinfo` (a user lookup) and the
///   notification center — and every interpreter test passes with both
///   denied, so the measured set is empty. A service a later path wants is
///   denied too, which fails the program rather than opening a route.
/// - **No network** (`deny network*`, spelled out though `deny default`
///   already covers it) and **no file write** anywhere — not `/dev/null`:
///   the worker's stdio are pipes and a log the launcher opened before the
///   profile applied.
/// - **No `system.sb`**, whose grants (the dyld cache paths, metadata
///   everywhere, the logging and notification services) the documents
///   profile imports and this worker was measured not to need: the dyld
///   shared cache is mapped by the kernel, not read through the filesystem.
///   Each start logs the denials of what it reaches for and does without.
pub fn worker_profile(worker: &Path) -> Result<String> {
    let w = quote(worker)?;
    Ok(format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process-exec (literal {w}))\n\
         (allow file-read-data (literal \"/\"))\n\
         (allow sysctl-read (sysctl-name \"hw.pagesize_compat\"))\n\
         (deny network*)\n"
    ))
}

/// What a running process's Seatbelt profile allows, asked of the kernel
/// (`sandbox_check`, libSystem's) — the live reading of a confinement,
/// never inferred from the argv that was meant to apply it. `None` when the
/// kernel would not answer, which every caller treats as not confined.
#[cfg(target_os = "macos")]
pub mod live {
    use std::ffi::{c_char, c_int, CString};
    use std::path::Path;

    extern "C" {
        fn sandbox_check(pid: libc::pid_t, operation: *const c_char, filter: c_int, ...) -> c_int;
    }

    /// `SANDBOX_FILTER_NONE`, `SANDBOX_FILTER_PATH` and
    /// `SANDBOX_FILTER_GLOBAL_NAME` from `sandbox.h`.
    const FILTER_NONE: c_int = 0;
    const FILTER_PATH: c_int = 1;
    const FILTER_GLOBAL_NAME: c_int = 2;

    fn answer(r: c_int) -> Option<bool> {
        match r {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// Whether `pid` runs under any Seatbelt profile.
    pub fn is_sandboxed(pid: u32) -> Option<bool> {
        // SAFETY: a null operation with no filter argument is the documented
        // "is this process sandboxed" form.
        answer(unsafe { sandbox_check(pid as libc::pid_t, std::ptr::null(), FILTER_NONE) })
    }

    /// Whether `pid`'s profile denies `operation` outright (`network-outbound`).
    pub fn denies(pid: u32, operation: &str) -> Option<bool> {
        let op = CString::new(operation).ok()?;
        // SAFETY: a NUL-terminated operation name and no filter argument.
        answer(unsafe { sandbox_check(pid as libc::pid_t, op.as_ptr(), FILTER_NONE) })
    }

    /// Whether `pid`'s profile denies `operation` on `path`, which must be
    /// spelled as the kernel resolves it (`/private/etc`, not `/etc`).
    pub fn denies_path(pid: u32, operation: &str, path: &Path) -> Option<bool> {
        let op = CString::new(operation).ok()?;
        let p = CString::new(path.to_str()?).ok()?;
        // SAFETY: two NUL-terminated strings, the path the one variadic
        // argument the path filter takes.
        answer(unsafe { sandbox_check(pid as libc::pid_t, op.as_ptr(), FILTER_PATH, p.as_ptr()) })
    }

    /// Whether `pid`'s profile denies looking up the Mach service `name`.
    pub fn denies_service(pid: u32, name: &str) -> Option<bool> {
        let op = CString::new("mach-lookup").ok()?;
        let n = CString::new(name).ok()?;
        // SAFETY: as `denies_path`, with a service name for the filter.
        answer(unsafe {
            sandbox_check(
                pid as libc::pid_t,
                op.as_ptr(),
                FILTER_GLOBAL_NAME,
                n.as_ptr(),
            )
        })
    }
}

/// A path as an SBPL string literal, or why it cannot be one.
pub fn quote(path: &Path) -> Result<String> {
    let s = path
        .to_str()
        .with_context(|| format!("{} is not UTF-8, so no profile can name it", path.display()))?;
    if !path.is_absolute() {
        bail!("{s} is not absolute, so no profile can name it");
    }
    if s.chars().any(char::is_control) {
        bail!(
            "{} holds a control character, so no profile can name it",
            s.escape_debug()
        );
    }
    Ok(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// A bare program name on `PATH` (then the package managers' prefixes), or
/// an absolute one as given.
pub fn resolve_program(program: &str, path_var: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    let p = Path::new(program);
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    if program.contains('/') {
        bail!("`{program}` is a relative path; a confined program is named absolutely or bare");
    }
    let dirs = path_var
        .map(|v| std::env::split_paths(v).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.into_iter()
        .chain(FALLBACK_BIN.iter().map(PathBuf::from))
        .map(|d| d.join(program))
        .find(|c| c.is_file())
        .with_context(|| {
            format!(
                "`{program}` is not on PATH or in {}",
                FALLBACK_BIN.join(", ")
            )
        })
}

/// Every library `binary` loads, transitively, by its Mach-O load commands —
/// absolute paths as they resolve on disk. Libraries in the shared cache
/// (`/usr/lib`, `/System`) are covered by the system's reads and skipped, as
/// is anything that does not resolve: dyld would fail on it unconfined too.
///
/// Two places this is narrower than dyld, both failing closed — a denial at
/// load, never a read granted — and the first place to look when a Mac
/// reports a dyld failure under the profile: `@rpath` is resolved against
/// the loading file's own `LC_RPATH`s only, where dyld also searches the
/// loader chain's (the main executable's included); the walk stops at 512
/// files; and a fat binary is read only in its 32-bit-offset form
/// (`FAT_MAGIC`), so a `FAT_MAGIC_64` or byte-swapped one yields no
/// libraries. Homebrew ships thin binaries.
pub fn library_closure(binary: &Path) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut queue = vec![binary.to_path_buf()];
    while let Some(file) = queue.pop() {
        if seen.len() > 512 {
            break;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let Some(info) = macho::load_info(&bytes) else {
            continue;
        };
        let dir = file.parent().unwrap_or(Path::new("/"));
        // `@executable_path` is the program's own directory for every library
        // it loads; `@loader_path` is the loading file's.
        let exe_dir = binary.parent().unwrap_or(Path::new("/"));
        let rpaths: Vec<PathBuf> = info
            .rpaths
            .iter()
            .filter_map(|r| expand(r, dir, exe_dir))
            .collect();
        for dylib in &info.dylibs {
            if is_system(dylib) {
                continue;
            }
            let candidates: Vec<PathBuf> = if let Some(rest) = dylib.strip_prefix("@rpath/") {
                rpaths.iter().map(|r| r.join(rest)).collect()
            } else {
                expand(dylib, dir, exe_dir).into_iter().collect()
            };
            let Some(found) = candidates.into_iter().find(|c| c.is_file()) else {
                continue;
            };
            for p in [found.clone(), found.canonicalize().unwrap_or(found)] {
                if !seen.contains(&p) {
                    seen.push(p.clone());
                    queue.push(p);
                }
            }
        }
    }
    seen
}

/// [`library_closure`], walked once per program for the life of the process:
/// it is a function of the binary alone, and one extraction runs poppler
/// dozens of times (review of #682). The profile around it is still built
/// per call — the scratch directory and the call's paths are not cached.
fn closure_once(binary: &Path) -> Vec<PathBuf> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, Vec<PathBuf>>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = seen.lock().ok().and_then(|m| m.get(binary).cloned()) {
        return hit;
    }
    let walked = library_closure(binary);
    if let Ok(mut m) = seen.lock() {
        m.insert(binary.to_path_buf(), walked.clone());
    }
    walked
}

fn expand(path: &str, loader_dir: &Path, exe_dir: &Path) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("@loader_path") {
        Some(loader_dir.join(rest.trim_start_matches('/')))
    } else if let Some(rest) = path.strip_prefix("@executable_path") {
        Some(exe_dir.join(rest.trim_start_matches('/')))
    } else if path.starts_with('/') {
        Some(PathBuf::from(path))
    } else {
        None
    }
}

fn is_system(path: &str) -> bool {
    path.starts_with("/usr/lib/") || path.starts_with("/System/")
}

/// The Homebrew keg a file lives in (`<prefix>/Cellar/<name>/<version>`), and
/// that prefix's data a renderer reads — fontconfig's configuration and
/// cache, poppler's encodings — where they exist. `None` outside a Cellar.
pub fn keg_and_data(file: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let s = file.to_str()?;
    let (prefix, rest) = s.split_once("/Cellar/")?;
    let mut parts = rest.splitn(3, '/');
    let (name, version) = (parts.next()?, parts.next()?);
    let prefix = PathBuf::from(prefix);
    let keg = prefix.join("Cellar").join(name).join(version);
    let data = [
        "etc/fonts",
        "share/poppler",
        "share/fontconfig",
        "var/cache/fontconfig",
    ]
    .iter()
    .map(|d| prefix.join(d))
    .collect();
    Some((keg, data))
}

/// The policy for running `program` with `workspace` as its scratch: the
/// program and its libraries found, their kegs and data, the call's extra
/// paths — all canonical, since the sandbox matches paths as they resolve
/// (`/var` is `/private/var` on a Mac). Returns the program **as named**,
/// which is what runs: the resolved path is for the library walk and the
/// profile only (see [`command`]).
pub fn policy_for(
    program: &str,
    workspace: &Path,
    readable: &[PathBuf],
    writable: &[PathBuf],
    network: bool,
) -> Result<(PathBuf, Policy)> {
    let named = resolve_program(program, std::env::var_os("PATH").as_deref())?;
    let real = named
        .canonicalize()
        .with_context(|| format!("resolving {}", named.display()))?;
    let canon = |p: &Path| {
        p.canonicalize()
            .with_context(|| format!("resolving {}", p.display()))
    };
    let mut policy = Policy {
        exec: vec![real.clone()],
        write_trees: vec![canon(workspace)?],
        // Required: the home's denial is the rule written to hold whatever
        // `system.sb` grants, and a profile without it is otherwise the same
        // text — so no `HOME` refuses the call rather than dropping the rule
        // in silence (review of #682).
        home: Some(
            std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .map(PathBuf::from)
                .context("HOME is unset, so no profile can deny the home — refusing to confine")?
                .canonicalize()
                .context("resolving HOME for the profile's denial of it")?,
        ),
        network,
        ..Policy::default()
    };
    if named != real {
        policy.exec.push(named.clone());
    }
    for w in writable {
        policy.write_trees.push(canon(w)?);
    }
    for r in readable {
        let r = canon(r)?;
        if r.is_dir() {
            policy.read_trees.push(r);
        } else {
            policy.read_files.push(r);
        }
    }
    let mut files = vec![real.clone()];
    files.extend(closure_once(&real));
    for f in &files {
        if let Some((keg, data)) = keg_and_data(f) {
            if !policy.read_trees.contains(&keg) {
                policy.read_trees.push(keg);
            }
            for d in data {
                if let Ok(d) = d.canonicalize() {
                    if !policy.read_trees.contains(&d) {
                        policy.read_trees.push(d);
                    }
                }
            }
        } else if f != &real && !policy.read_files.contains(f) {
            policy.read_files.push(f.clone());
        }
    }
    Ok((named, policy))
}

/// The Mach-O reader: the load commands that name libraries and search
/// paths, from a thin 64-bit image or a fat file's slice for this CPU.
pub mod macho {
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const FAT_MAGIC: u32 = 0xcafe_babe;
    const LC_REQ_DYLD: u32 = 0x8000_0000;
    const LC_LOAD_DYLIB: u32 = 0xc;
    const LC_LOAD_WEAK_DYLIB: u32 = 0x18 | LC_REQ_DYLD;
    const LC_REEXPORT_DYLIB: u32 = 0x1f | LC_REQ_DYLD;
    const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
    const LC_LOAD_UPWARD_DYLIB: u32 = 0x23 | LC_REQ_DYLD;
    const LC_RPATH: u32 = 0x1c | LC_REQ_DYLD;
    /// `CPU_TYPE_ARM64` and `CPU_TYPE_X86_64`.
    const CPU_ARM64: u32 = 0x0100_000c;
    const CPU_X86_64: u32 = 0x0100_0007;

    #[derive(Debug, Default, PartialEq)]
    pub struct LoadInfo {
        pub dylibs: Vec<String>,
        pub rpaths: Vec<String>,
    }

    fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
        let w: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(w)
        } else {
            u32::from_le_bytes(w)
        })
    }

    /// The libraries and rpaths a Mach-O names, or `None` for anything else.
    pub fn load_info(bytes: &[u8]) -> Option<LoadInfo> {
        match u32_at(bytes, 0, true)? {
            FAT_MAGIC => {
                let want = if cfg!(target_arch = "x86_64") {
                    CPU_X86_64
                } else {
                    CPU_ARM64
                };
                let n = u32_at(bytes, 4, true)? as usize;
                (0..n.min(16)).find_map(|i| {
                    let at = 8 + i * 20;
                    if u32_at(bytes, at, true)? != want {
                        return None;
                    }
                    let off = u32_at(bytes, at + 8, true)? as usize;
                    let size = u32_at(bytes, at + 12, true)? as usize;
                    thin(bytes.get(off..off.checked_add(size)?)?)
                })
            }
            _ => thin(bytes),
        }
    }

    fn thin(b: &[u8]) -> Option<LoadInfo> {
        if u32_at(b, 0, false)? != MH_MAGIC_64 {
            return None;
        }
        let ncmds = u32_at(b, 16, false)? as usize;
        let mut at = 32; // sizeof(mach_header_64)
        let mut info = LoadInfo::default();
        for _ in 0..ncmds.min(4096) {
            let cmd = u32_at(b, at, false)?;
            let size = u32_at(b, at + 4, false)? as usize;
            if size < 8 {
                return None;
            }
            let string = |field: usize| -> Option<String> {
                let off = u32_at(b, at + field, false)? as usize;
                let raw = b.get(at + off..at + size)?;
                let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                String::from_utf8(raw[..end].to_vec()).ok()
            };
            match cmd {
                LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LAZY_LOAD_DYLIB
                | LC_LOAD_UPWARD_DYLIB => info.dylibs.extend(string(8)),
                LC_RPATH => info.rpaths.extend(string(8)),
                _ => {}
            }
            at = at.checked_add(size)?;
        }
        Some(info)
    }

    #[cfg(test)]
    pub(crate) mod fixture {
        /// A thin arm64 Mach-O with the given load commands: `(cmd, string)`.
        pub fn thin(cmds: &[(u32, &str)]) -> Vec<u8> {
            let mut body = Vec::new();
            for (cmd, s) in cmds {
                let mut name = s.as_bytes().to_vec();
                name.push(0);
                // dylib_command is 24 bytes before the name; rpath_command 12.
                let head = if *cmd == super::LC_RPATH { 12 } else { 24 };
                let mut size = head + name.len();
                size = size.div_ceil(8) * 8;
                let mut c = Vec::new();
                c.extend_from_slice(&cmd.to_le_bytes());
                c.extend_from_slice(&(size as u32).to_le_bytes());
                c.extend_from_slice(&(head as u32).to_le_bytes());
                c.resize(head, 0);
                c.extend_from_slice(&name);
                c.resize(size, 0);
                body.extend(c);
            }
            let mut out = Vec::new();
            out.extend_from_slice(&super::MH_MAGIC_64.to_le_bytes());
            out.extend_from_slice(&super::CPU_ARM64.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
            out.extend_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
            out.extend_from_slice(&(cmds.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // flags
            out.extend_from_slice(&0u32.to_le_bytes()); // reserved
            out.extend(body);
            out
        }

        pub const LOAD: u32 = super::LC_LOAD_DYLIB;
        pub const WEAK: u32 = super::LC_LOAD_WEAK_DYLIB;
        pub const RPATH: u32 = super::LC_RPATH;
    }
}

/// The command for `program` under the policy: `sandbox-exec -p <profile>`,
/// the environment cleared to what a renderer needs, `HOME` and `TMPDIR` the
/// scratch directory, and the caller's named variables passed through.
pub fn command(
    program: &str,
    args: &[String],
    workspace: &Path,
    cwd: &Path,
    cfg: &super::SandboxConfig,
) -> Result<tokio::process::Command> {
    // The program runs as named, as every other backend runs it: a venv's
    // `bin/python` is a link to its interpreter, and CPython finds the venv
    // (`pyvenv.cfg`) beside the path it was started by — run by its target,
    // the layout worker would lose the venv's packages (review of #682).
    // Both spellings are in the profile's `process-exec`.
    let (named, policy) = policy_for(
        program,
        workspace,
        &cfg.readable,
        &cfg.writable,
        cfg.network,
    )?;
    let text = profile(&policy)?;
    let scratch = &policy.write_trees[0];
    let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
    c.arg("-p").arg(text).arg(&named).args(args);
    c.current_dir(
        cwd.canonicalize()
            .with_context(|| format!("resolving {}", cwd.display()))?,
    );
    c.env_clear();
    let mut path = String::from("/usr/bin:/bin:/usr/sbin:/sbin");
    if let Some(dir) = named.parent().and_then(Path::to_str) {
        path = format!("{dir}:{path}");
    }
    c.env("PATH", path);
    c.env("HOME", scratch);
    c.env("TMPDIR", scratch);
    for name in &cfg.env {
        if let Ok(value) = std::env::var(name) {
            c.env(name, value);
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_quoted_or_refused() {
        assert_eq!(quote(Path::new("/a b/c")).unwrap(), "\"/a b/c\"");
        assert_eq!(quote(Path::new("/a\"b\\c")).unwrap(), "\"/a\\\"b\\\\c\"");
        assert!(quote(Path::new("/a\nb")).is_err(), "a newline is refused");
        assert!(
            quote(Path::new("rel/p")).is_err(),
            "a relative path is refused"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = std::ffi::OsStr::from_bytes(b"/a\xffb");
            assert!(quote(Path::new(bad)).is_err(), "non-UTF-8 is refused");
        }
    }

    /// The profile denies by default and the network unless asked, lets the
    /// program run, writes only its scratch, and puts the home's denial
    /// before the call's own paths, so a path inside the home that the call
    /// names still wins.
    #[test]
    fn the_profile_denies_by_default_and_names_what_the_call_needs() {
        let p = Policy {
            exec: vec!["/opt/homebrew/Cellar/poppler/26.1/bin/pdftoppm".into()],
            read_trees: vec!["/opt/homebrew/Cellar/poppler/26.1".into()],
            read_files: vec!["/Users/a/.mecha/layout/model.onnx".into()],
            write_trees: vec!["/private/var/folders/x/T/mecha-doc-1".into()],
            home: Some("/Users/a".into()),
            network: false,
        };
        let t = profile(&p).unwrap();
        assert!(t.starts_with("(version 1)\n(deny default)\n"), "{t}");
        assert!(t.contains("(deny network*)"), "{t}");
        assert!(!t.contains("(allow network*)"), "{t}");
        assert!(t.contains(
            "(allow process-exec (literal \"/opt/homebrew/Cellar/poppler/26.1/bin/pdftoppm\"))"
        ));
        assert!(t.contains(
            "(allow file-read* file-write* (subpath \"/private/var/folders/x/T/mecha-doc-1\"))"
        ));
        // The named path in the home is carved out of the denial itself,
        // not left to rule order.
        assert!(
            t.contains(
                "(deny file-read-data file-read-xattr (require-all (subpath \"/Users/a\") \
                 (require-not (literal \"/Users/a/.mecha/layout/model.onnx\"))))"
            ),
            "{t}"
        );
        // A path the call may only read stays unwritable: the write denial
        // carves out nothing it may not write (review of #682).
        assert!(
            t.contains("(deny file-write* (subpath \"/Users/a\"))"),
            "{t}"
        );
        assert!(
            t.contains(
                "(deny file-read-data file-read-xattr file-write* (subpath \
                 \"/System/Volumes/Data\"))"
            ),
            "the data volume's second spelling of the home is denied:\n{t}"
        );
        let bare = profile(&Policy {
            read_files: vec![],
            ..p.clone()
        })
        .unwrap();
        assert!(
            bare.contains("(deny file-read-data file-read-xattr (subpath \"/Users/a\"))")
                && bare.contains("(deny file-write* (subpath \"/Users/a\"))"),
            "{bare}"
        );
        assert!(
            !t.contains("(allow file-write* (subpath \"/Users/a\""),
            "the home is never writable:\n{t}"
        );
        let open = profile(&Policy {
            network: true,
            ..p.clone()
        })
        .unwrap();
        assert!(open.contains("(allow network*)") && !open.contains("(deny network*)"));
        assert!(
            profile(&Policy::default()).is_err(),
            "no program, no profile"
        );
    }

    /// The worker's profile allows exactly its measured list: exec of its own
    /// binary, the root's listing and one sysctl — no read of anything else,
    /// no write, no network, no service, no other program, and none of the
    /// documents profile's grants. The list is reviewed here, not by reading
    /// the format string.
    #[test]
    fn the_worker_profile_allows_its_exec_the_root_listing_and_one_sysctl_only() {
        let t = worker_profile(Path::new("/Users/a/.mecha/sidecars/monty/1.4.0/monty")).unwrap();
        assert!(t.starts_with("(version 1)\n(deny default)\n"), "{t}");
        let allows: Vec<&str> = t.lines().filter(|l| l.starts_with("(allow")).collect();
        assert_eq!(
            allows,
            vec![
                "(allow process-exec (literal \"/Users/a/.mecha/sidecars/monty/1.4.0/monty\"))",
                "(allow file-read-data (literal \"/\"))",
                "(allow sysctl-read (sysctl-name \"hw.pagesize_compat\"))",
            ]
        );
        assert!(t.contains("(deny network*)"), "{t}");
        for absent in [
            "system.sb",
            "file-write",
            "network*)\n(allow",
            "mach-lookup",
            "subpath",
            "file-read-metadata",
            "file-read*",
        ] {
            assert!(
                !t.contains(absent),
                "`{absent}` in the worker's profile:\n{t}"
            );
        }
        assert!(
            worker_profile(Path::new("relative/monty")).is_err(),
            "a path no profile can name refuses"
        );
    }

    #[test]
    fn load_commands_name_the_libraries_and_rpaths() {
        use macho::fixture::*;
        let bin = thin(&[
            (LOAD, "/opt/homebrew/opt/libpng/lib/libpng16.16.dylib"),
            (WEAK, "@rpath/libfoo.dylib"),
            (RPATH, "@loader_path/../lib"),
            (LOAD, "/usr/lib/libSystem.B.dylib"),
        ]);
        let info = macho::load_info(&bin).unwrap();
        assert_eq!(
            info.dylibs,
            [
                "/opt/homebrew/opt/libpng/lib/libpng16.16.dylib",
                "@rpath/libfoo.dylib",
                "/usr/lib/libSystem.B.dylib"
            ]
        );
        assert_eq!(info.rpaths, ["@loader_path/../lib"]);
        assert_eq!(macho::load_info(b"\x7fELF...."), None, "not a Mach-O");
        assert_eq!(
            macho::load_info(&bin[..40]),
            None,
            "a truncated one is unread"
        );
    }

    /// The closure follows `@rpath` through `@loader_path`, takes each
    /// library as found and as resolved, and leaves the shared cache's
    /// libraries to the system's reads.
    #[test]
    fn the_closure_resolves_rpaths_and_skips_the_system() {
        use macho::fixture::*;
        let root = std::env::temp_dir().join(format!("mecha-sb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(
            root.join("lib/libfoo.dylib"),
            thin(&[(LOAD, "/usr/lib/libSystem.B.dylib")]),
        )
        .unwrap();
        std::fs::write(
            root.join("bin/tool"),
            thin(&[
                (RPATH, "@loader_path/../lib"),
                (LOAD, "@rpath/libfoo.dylib"),
                (LOAD, "/usr/lib/libz.1.dylib"),
                (LOAD, "/nowhere/libgone.dylib"),
            ]),
        )
        .unwrap();
        let closure = library_closure(&root.join("bin/tool"));
        let lib = root.join("bin/../lib/libfoo.dylib");
        assert!(closure.contains(&lib), "{closure:?}");
        assert!(
            closure.contains(&lib.canonicalize().unwrap()),
            "{closure:?}"
        );
        assert!(
            !closure
                .iter()
                .any(|p| p.starts_with("/usr/lib") || p.starts_with("/nowhere")),
            "{closure:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cellar_file_names_its_keg_and_its_prefix_s_data() {
        let (keg, data) = keg_and_data(Path::new(
            "/opt/homebrew/Cellar/poppler/26.1.0/bin/pdftoppm",
        ))
        .unwrap();
        assert_eq!(keg, Path::new("/opt/homebrew/Cellar/poppler/26.1.0"));
        assert!(data.contains(&PathBuf::from("/opt/homebrew/etc/fonts")));
        assert!(data.contains(&PathBuf::from("/opt/homebrew/share/poppler")));
        assert_eq!(keg_and_data(Path::new("/usr/local/bin/pdftoppm")), None);
    }

    #[test]
    fn a_bare_name_is_found_on_path_and_a_relative_one_refused() {
        let dir = std::env::temp_dir().join(format!("mecha-sb-p-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tool"), b"").unwrap();
        let found = resolve_program("tool", Some(dir.as_os_str())).unwrap();
        assert_eq!(found, dir.join("tool"));
        assert!(resolve_program("bin/tool", Some(dir.as_os_str())).is_err());
        assert!(resolve_program("no-such-tool-here", Some(dir.as_os_str())).is_err());
        assert_eq!(
            resolve_program("/abs/tool", None).unwrap(),
            Path::new("/abs/tool")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
