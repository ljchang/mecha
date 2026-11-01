//! What the kernel says about a process the tests spawned: its descendants,
//! whether it is stopped, gone, and how much memory it has held. Linux reads
//! `/proc`; macOS asks libproc, which has no `/proc` to read. Shared by the
//! interpreter's integration tests and its unit tests (`#[path]`).

#![allow(dead_code)]

/// How a process is, as far as the tests care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Zombie,
    /// Torn down past being a zombie (Linux's `X`): gone in all but name.
    Dead,
    Other,
}

/// `pid`'s state, or `None` once it is gone.
pub fn state(pid: u32) -> Option<State> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (_, rest) = stat.rsplit_once(") ")?;
        Some(match rest.chars().next()? {
            'T' | 't' => State::Stopped,
            'Z' => State::Zombie,
            'X' | 'x' => State::Dead,
            _ => State::Other,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let info = bsdinfo(pid)?;
        Some(match info.pbi_status {
            libc::SSTOP => State::Stopped,
            libc::SZOMB => State::Zombie,
            _ => State::Other,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Gone, a zombie waiting to be reaped, or dead: either way, no longer
/// running.
pub fn is_gone(pid: u32) -> bool {
    matches!(state(pid), None | Some(State::Zombie | State::Dead))
}

/// Whether `pid` has a `SIGKILL` delivered and not yet acted on. A process
/// in uninterruptible sleep (exec paging in a binary on a loaded runner, say)
/// holds one until it leaves it, so "killed" is this or [`is_gone`]: the
/// signal reached it, whatever the scheduler does next. Linux reads
/// `/proc/<pid>/status`; elsewhere it answers `false`, and only gone counts.
pub fn sigkill_pending(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return false;
        };
        // SIGKILL is signal 9: bit 8 of the pending masks.
        status
            .lines()
            .filter(|l| l.starts_with("SigPnd:") || l.starts_with("ShdPnd:"))
            .filter_map(|l| u64::from_str_radix(l.split_whitespace().nth(1)?, 16).ok())
            .any(|mask| mask & (1 << 8) != 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// Killed: gone, or with a `SIGKILL` on its way ([`sigkill_pending`]).
pub fn is_killed(pid: u32) -> bool {
    is_gone(pid) || sigkill_pending(pid)
}

/// `/proc/<pid>/stat` as it reads now, or empty: for a failure message.
pub fn stat_line(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(target_os = "macos")]
fn bsdinfo(pid: u32) -> Option<libc::proc_bsdinfo> {
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: a zeroed plain-data struct the kernel fills, of the size passed.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (n == size).then_some(info)
}

/// Every process under `pid`, itself included.
pub fn tree(pid: u32) -> Vec<u32> {
    let mut out = vec![pid];
    let mut i = 0;
    while i < out.len() {
        out.extend(children(out[i]));
        i += 1;
    }
    out
}

fn children(pid: u32) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let mut kids = Vec::new();
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
            for t in tasks.flatten() {
                let list = std::fs::read_to_string(t.path().join("children")).unwrap_or_default();
                kids.extend(
                    list.split_whitespace()
                        .filter_map(|k| k.parse::<u32>().ok()),
                );
            }
        }
        kids
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0 as libc::pid_t; 1024];
        // SAFETY: the buffer's byte length is what is passed; the call
        // returns how many pids it wrote.
        let n = unsafe {
            libc::proc_listchildpids(
                pid as libc::pid_t,
                buf.as_mut_ptr().cast(),
                (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
            )
        };
        buf.truncate(n.max(0) as usize);
        buf.into_iter()
            .filter(|p| *p > 0)
            .map(|p| p as u32)
            .collect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Vec::new()
    }
}

/// The most memory `pid` has held over its life, in bytes: `VmHWM` (peak
/// resident set) on Linux, `ri_lifetime_max_phys_footprint` on macOS (what
/// the kernel charges the process, compressed and swapped pages included).
/// Both only grow, so one reading after a peak finds it.
pub fn peak_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let kb = status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(kb * 1024)
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a zeroed plain-data struct of the flavor named.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::proc_pid_rusage(
                pid as libc::c_int,
                libc::RUSAGE_INFO_V4,
                (&mut info as *mut libc::rusage_info_v4).cast(),
            )
        };
        (r == 0).then_some(info.ri_lifetime_max_phys_footprint)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// The path of the executable `pid` runs: `/proc/<pid>/exe` on Linux (as
/// the process itself names it, inside its own mount namespace), libproc's
/// `proc_pidpath` on macOS.
pub fn exe(pid: u32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }
    #[cfg(target_os = "macos")]
    {
        // `PROC_PIDPATHINFO_MAXSIZE`.
        let mut buf = vec![0u8; 4096];
        // SAFETY: the buffer's length is what is passed.
        let n = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(std::path::PathBuf::from(String::from_utf8(buf).ok()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// The names in `pid`'s environment as the kernel holds it — never the
/// values, which could be secrets. `/proc/<pid>/environ` on Linux;
/// `KERN_PROCARGS2` on macOS, the block `exec` copied onto the new stack:
/// argc, the exec path, argv, then the environment.
pub fn environment_names(pid: u32) -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    let block: Vec<u8> = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    #[cfg(target_os = "macos")]
    let block: Vec<u8> = {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
        let mut size: libc::size_t = 0;
        // SAFETY: a size query, then a read into a buffer of that size.
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        if !ok || size < 4 {
            return None;
        }
        let mut buf = vec![0u8; size];
        let ok = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        if !ok {
            return None;
        }
        buf.truncate(size);
        let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?) as usize;
        let mut at = 4;
        // The exec path, then the NULs padding it.
        while at < buf.len() && buf[at] != 0 {
            at += 1;
        }
        while at < buf.len() && buf[at] == 0 {
            at += 1;
        }
        // argv: argc strings, one NUL each.
        for _ in 0..argc {
            while at < buf.len() && buf[at] != 0 {
                at += 1;
            }
            at += 1;
        }
        // The environment runs to the first empty string.
        let rest = buf.get(at..).unwrap_or_default();
        let end = rest
            .windows(2)
            .position(|w| w[0] == 0 && w[1] == 0)
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let mut env = rest[..end].to_vec();
        if env.first() == Some(&0) {
            env.clear();
        }
        env
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let block: Vec<u8> = {
        let _ = pid;
        return None;
    };
    Some(
        block
            .split(|b| *b == 0)
            .filter(|kv| !kv.is_empty())
            .map(|kv| {
                String::from_utf8_lossy(kv.split(|b| *b == b'=').next().unwrap_or_default())
                    .into_owned()
            })
            .collect(),
    )
}
