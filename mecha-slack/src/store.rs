//! Owner-only JSON files, written the way every other store under
//! `~/.mecha` writes them.
//!
//! Small on purpose. It holds no Slack knowledge and no agent knowledge — it
//! is here rather than in either caller so that the binding, the thread store,
//! and anything added later cannot drift apart on the two properties that
//! matter: **0600 inside a 0700 directory**, and **temp sibling then rename**,
//! so a crash halfway through a write leaves the previous file rather than
//! half of the next one. A store that fails to parse is a remote control that
//! silently stops answering, which is the failure mode this is guarding.

use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn set_owner_only(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

/// Read, treating absence as `None` rather than as an error — "nothing is
/// bound yet" and "the disk is broken" are different answers and the caller
/// needs to tell them apart.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_private_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let body = serde_json::to_string_pretty(value).map_err(|e| io::Error::other(e.to_string()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "record".into());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, body.as_bytes())?;
    set_owner_only(&tmp)?;
    std::fs::rename(&tmp, path)
}

pub fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-slack-store-{name}-{}", uuid::Uuid::new_v4()));
        create_private_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn a_round_trip_survives_and_absence_is_none() {
        let dir = scratch("roundtrip");
        let path = dir.join("thing.json");
        assert!(read_json::<serde_json::Value>(&path).unwrap().is_none());

        write_private_json(&path, &serde_json::json!({"a": 1})).unwrap();
        let back: serde_json::Value = read_json(&path).unwrap().unwrap();
        assert_eq!(back["a"], 1);

        remove_if_present(&path).unwrap();
        remove_if_present(&path).unwrap();
        assert!(read_json::<serde_json::Value>(&path).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_file_is_an_error_and_not_an_absence() {
        // The distinction that matters: silently treating unreadable state as
        // "nothing here" is how a store quietly forgets what it was holding.
        let dir = scratch("corrupt");
        let path = dir.join("thing.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_json::<serde_json::Value>(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = scratch("tmp");
        let path = dir.join("thing.json");
        write_private_json(&path, &serde_json::json!({"a": 1})).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn what_is_written_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perm");
        let path = dir.join("secret.json");
        write_private_json(&path, &serde_json::json!({"token": "xoxb"})).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(&dir).ok();
    }
}
