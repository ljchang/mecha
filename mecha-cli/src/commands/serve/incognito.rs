//! Incognito chat: a web session that leaves no trace once it is closed
//! (`docs/INCOGNITO-DESIGN.md`, rulings R1–R6).
//!
//! What lives here is the part that is not the chat loop: the room an
//! incognito conversation works in, the key that names it, the tools it may
//! reach, and whether this process may open one at all. `chat.rs` holds the
//! conversation itself, as a `Recording::Incognito` — a session with no
//! transcript by type, so every place that records has to say what it does
//! instead.
//!
//! **The room is in RAM.** `$XDG_RUNTIME_DIR/mecha-incognito/<key>/` holds
//! the workspace (the jail: uploads, model-written files) and, beside it, the
//! spill directory. Both are on tmpfs — checked with `statfs`, never assumed
//! — so nothing the conversation touches reaches the SSD, and closing the
//! room is removing a directory rather than hoping every copy was found. The
//! spill directory is a sibling of the workspace, never inside it: the jail's
//! spill exception must point somewhere the model cannot write (#313).
//!
//! **What it may reach is an allowlist.** The block list a run applies
//! (`RunContext::withheld`) is a denylist by design; incognito computes it as
//! the complement of the few tools it allows, against the live registry, so a
//! server or an outbox route added tomorrow is withheld without anyone
//! remembering to add it (review of #307).

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Every incognito session key starts with this. The ordinary chat door
/// refuses it, and the cache-header layer keys `no-store` on it.
pub const KEY_PREFIX: &str = "incognito-";

/// How long an incognito chat may sit idle before it closes itself (R5).
pub const IDLE: Duration = Duration::from_secs(30 * 60);

/// Whether `key` names an incognito chat.
pub fn is_incognito_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

/// A fresh key: the prefix and 22 hex digits of a v4 UUID — 82 random bits,
/// since the version and variant nibbles fall in that span — which fills the
/// 32 characters `chat::valid_key` allows. Never reused, never `main`.
pub fn new_key() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    format!("{KEY_PREFIX}{}", &hex[..32 - KEY_PREFIX.len()])
}

/// Builtins an incognito chat may call. Everything that writes stays in the
/// room; search reaches a search engine, which the page says before the
/// first search (R4). Not here, deliberately: `image_generate`, until the
/// image server's temp copies are deleted per room (design §6.3, step 4) —
/// a promise that cannot be kept is not made.
const ALLOWED_BUILTINS: &[&str] = &[
    "fs_read",
    "fs_list",
    "fs_write",
    "fs_edit",
    "shell",
    "todo",
    "compact",
    "ask_user",
    "web_search",
    "web_open",
    "http_fetch",
];

/// The MCP server whose read-only tools an incognito chat may call (R3:
/// mail and calendar reads). The graph records every read's query text
/// (`query_log`), so its tools stay withheld until it can read without
/// recording (design §5.2).
const READABLE_SERVER: &str = "mail";

/// Tools an incognito run may not dispatch: every registered tool that is
/// not allowed. `tools` is the registry as `(name, read_only)`; `routed` the
/// outbox's routed names, which stage a draft in every permission mode and
/// so are withheld even when read-only.
pub fn withheld<'a>(
    tools: impl IntoIterator<Item = (&'a str, bool)>,
    routed: &[String],
) -> Vec<String> {
    let mail = format!("{READABLE_SERVER}__");
    tools
        .into_iter()
        .filter(|(name, read_only)| {
            let allowed = ALLOWED_BUILTINS.contains(name)
                || (name.starts_with(&mail) && *read_only && !routed.iter().any(|r| r == name));
            !allowed
        })
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Whether this process's chat provider may serve an incognito chat: a
/// local server on this machine, with no fallbacks. A cloud provider keeps
/// the text on someone else's servers; a fallback would re-send the whole
/// conversation there on a transient local error, silently (design §6.1).
pub fn provider_is_local(
    config: &mecha_core::config::Config,
    provider_name: &str,
) -> std::result::Result<(), String> {
    let Some(provider) = config.providers.get(provider_name) else {
        return Err(format!("the provider `{provider_name}` is not configured"));
    };
    if !provider.fallbacks.is_empty() {
        return Err(format!(
            "the provider `{provider_name}` has fallbacks ({}), which could send the \
             conversation elsewhere",
            provider.fallbacks.join(", ")
        ));
    }
    let loopback = provider
        .base_url
        .as_deref()
        .and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches('[').trim_end_matches(']').to_string())
        })
        .is_some_and(|h| match h.parse::<std::net::IpAddr>() {
            Ok(ip) => ip.is_loopback(),
            Err(_) => h.eq_ignore_ascii_case("localhost"),
        });
    if !loopback {
        return Err(format!(
            "the provider `{provider_name}` is not a server on this machine"
        ));
    }
    Ok(())
}

/// Where incognito rooms live: a directory in the per-user runtime
/// directory, which must be tmpfs, and within it one per mecha home — so a
/// second `serve` against another home (a test server, a trial) sweeps its
/// own leftovers at start and never the rooms of the one serving the owner.
pub fn rooms_root() -> Result<PathBuf> {
    let home = mecha_core::work::mecha_home()?;
    let home = home.canonicalize().unwrap_or(home);
    let scope: String = home
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(runtime_rooms()?.join(scope))
}

/// `$XDG_RUNTIME_DIR/mecha-incognito`, refused unless it is tmpfs.
fn runtime_rooms() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow!("XDG_RUNTIME_DIR is not set, so there is no RAM-backed place for a room")
        })?;
    if !is_tmpfs(&runtime)? {
        bail!(
            "{} is not tmpfs, so an incognito room there could reach the disk",
            runtime.display()
        );
    }
    Ok(runtime.join("mecha-incognito"))
}

#[cfg(target_os = "linux")]
fn is_tmpfs(path: &Path) -> Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    const TMPFS_MAGIC: i64 = 0x0102_1994;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a zeroed out-parameter of the right
    // type; statfs writes only into `stat`.
    if unsafe { libc::statfs(c.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error()).context(format!("statfs {}", path.display()));
    }
    Ok(stat.f_type as i64 == TMPFS_MAGIC)
}

#[cfg(not(target_os = "linux"))]
fn is_tmpfs(_: &Path) -> Result<bool> {
    // No portable way to know here; refusing is the honest answer.
    Ok(false)
}

/// An incognito chat's room: where it works, where its oversized output
/// spills, and when it was last used.
#[derive(Debug)]
pub struct Room {
    /// The session key, which is also this room's directory name.
    pub key: String,
    /// The room itself — removed whole when the chat closes.
    pub root: PathBuf,
    /// The jail: `<root>/<key>`, so its directory name is the session key.
    pub workspace: PathBuf,
    /// Beside the jail, never in it.
    pub spill: PathBuf,
    last_active: std::sync::Mutex<Instant>,
}

impl Room {
    /// Open a room for `key` under `rooms`, owner-only.
    pub fn open(rooms: &Path, key: &str) -> Result<Room> {
        let root = rooms.join(key);
        // The jail is named for the key: `present::WebAsker` routes an
        // `ask_user` card by the jail's directory name, which for every web
        // session is its key. A jail called `workspace` sent an incognito
        // question nowhere — or to an ordinary chat of that name, carrying
        // this conversation's words with it (found on review of #321).
        let workspace = root.join(key);
        let spill = root.join("spill");
        for dir in [rooms, root.as_path(), workspace.as_path(), spill.as_path()] {
            mecha_core::create_private_dir(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(Room {
            key: key.to_string(),
            root,
            workspace,
            spill,
            last_active: std::sync::Mutex::new(Instant::now()),
        })
    }

    /// Mark the room used now: a turn started or finished.
    pub fn touch(&self) {
        *self.last_active.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Pretend the last use was `ago` earlier than it was.
    #[cfg(test)]
    pub fn backdate(&self, ago: Duration) {
        let mut at = self.last_active.lock().unwrap_or_else(|e| e.into_inner());
        *at = at.checked_sub(ago).expect("a monotonic clock that old");
    }

    /// Whether it has been idle longer than `idle`.
    pub fn idle_for(&self, idle: Duration) -> bool {
        self.last_active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
            >= idle
    }

    /// Remove the room and everything in it.
    pub fn remove(&self) -> Result<()> {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", self.root.display())),
        }
    }
}

/// Remove every room under `rooms`: run at start, before the door opens, so
/// a `serve` that died with incognito chats open leaves nothing for the next
/// one. Returns what it removed.
pub fn sweep(rooms: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(rooms) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
        let gone = if is_dir {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match gone {
            Ok(()) => removed.push(path),
            Err(e) => {
                tracing::warn!(path = %path.display(), "cannot remove a leftover incognito room: {e}")
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_key_is_a_valid_incognito_key_and_never_repeats() {
        let a = new_key();
        let b = new_key();
        assert!(is_incognito_key(&a));
        assert!(super::super::chat::valid_key(&a), "{a}");
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(!is_incognito_key("main"));
    }

    #[test]
    fn only_the_allowlist_and_mail_reads_are_reachable() {
        let tools = [
            ("fs_read", true),
            ("shell", false),
            ("web_search", true),
            ("mail__mail_search", true),
            ("mail__mail_send", false),
            ("mail__calendar_create_event", true),
            ("kg_search", true),
            ("docs__docs_read", true),
            ("image_generate", true),
            ("research", false),
            ("a_tool_added_tomorrow", true),
        ];
        let routed = vec!["mail__calendar_create_event".to_string()];
        let mut out = withheld(tools, &routed);
        out.sort();
        assert_eq!(
            out,
            [
                "a_tool_added_tomorrow",
                "docs__docs_read",
                "image_generate",
                "kg_search",
                "mail__calendar_create_event",
                "mail__mail_send",
                "research",
            ]
        );
    }

    #[test]
    fn only_a_local_provider_without_fallbacks_serves_incognito() {
        use mecha_core::config::{Config, ProviderConfig};
        let mut config = Config::default();
        let local = ProviderConfig {
            base_url: Some("http://127.0.0.1:8080".into()),
            ..ProviderConfig::default()
        };
        config.providers.insert("local".into(), local.clone());
        assert!(provider_is_local(&config, "local").is_ok());

        let mut failing_over = local.clone();
        failing_over.fallbacks = vec!["anthropic".into()];
        config.providers.insert("fo".into(), failing_over);
        assert!(provider_is_local(&config, "fo")
            .unwrap_err()
            .contains("fallbacks"));

        let cloud = ProviderConfig {
            base_url: Some("https://api.anthropic.com".into()),
            ..ProviderConfig::default()
        };
        config.providers.insert("cloud".into(), cloud);
        assert!(provider_is_local(&config, "cloud").is_err());
        let unset = ProviderConfig::default();
        config.providers.insert("unset".into(), unset);
        assert!(
            provider_is_local(&config, "unset").is_err(),
            "no base_url is not local"
        );
        assert!(provider_is_local(&config, "missing").is_err());
    }

    #[test]
    fn a_room_is_owner_only_and_removed_whole_and_a_sweep_clears_leftovers() {
        let rooms = std::env::temp_dir().join(format!("mecha-rooms-{}", uuid::Uuid::new_v4()));
        let key = new_key();
        let room = Room::open(&rooms, &key).unwrap();
        assert!(room.workspace.is_dir() && room.spill.is_dir());
        // `WebAsker` routes by the jail's directory name; it must be the key.
        assert_eq!(
            room.workspace.file_name().unwrap().to_str(),
            Some(key.as_str())
        );
        assert!(
            !room.spill.starts_with(&room.workspace),
            "the spill is beside the jail"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&room.root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        std::fs::write(room.workspace.join("upload.png"), "x").unwrap();
        room.remove().unwrap();
        assert!(!room.root.exists());
        room.remove().unwrap(); // closing twice is fine

        let left = Room::open(&rooms, &new_key()).unwrap();
        std::fs::write(left.spill.join("shell-1.txt"), "x").unwrap();
        assert_eq!(sweep(&rooms), vec![left.root.clone()]);
        assert!(!left.root.exists());
        std::fs::remove_dir_all(&rooms).ok();
    }

    #[test]
    fn idleness_is_measured_from_the_last_touch() {
        let rooms = std::env::temp_dir().join(format!("mecha-rooms-{}", uuid::Uuid::new_v4()));
        let room = Room::open(&rooms, &new_key()).unwrap();
        assert!(!room.idle_for(Duration::from_secs(60)));
        assert!(room.idle_for(Duration::ZERO));
        room.touch();
        assert!(!room.idle_for(Duration::from_secs(60)));
        std::fs::remove_dir_all(&rooms).ok();
    }
}
