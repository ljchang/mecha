//! Host-owned connection access for the synthetic memory store.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{ffi, Connection, OpenFlags};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

const KEY_BYTES: usize = 32;
const HEX_BYTES: usize = 2 * KEY_BYTES;
const HEX: &[u8; 16] = b"0123456789abcdef";

struct SecretKey(Zeroizing<[u8; KEY_BYTES]>);

/// Decryption authority supplied by the host, independent of memory scopes.
/// Cloning shares the key. Removing the original file does not revoke handles.
#[derive(Clone)]
pub struct StorageAccess(Option<Arc<SecretKey>>);

impl fmt::Debug for StorageAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StorageAccess([redacted])")
    }
}

/// Host-selected directories that must never contain the key file.
/// Existing components must be real directories. A missing suffix can
/// designate a future task or output directory.
pub struct KeyFilePolicy {
    forbidden_roots: Vec<PathBuf>,
}

impl KeyFilePolicy {
    pub fn new(forbidden_roots: &[PathBuf]) -> Result<Self> {
        ensure!(
            !forbidden_roots.is_empty(),
            "key policy requires an excluded root"
        );
        let forbidden_roots = forbidden_roots
            .iter()
            .map(|root| checked_directory(root, true))
            .collect::<Result<_>>()?;
        Ok(Self { forbidden_roots })
    }

    fn key_path(&self, path: &Path) -> Result<(PathBuf, File)> {
        let path = normalized_absolute(path)?;
        let parent = checked_directory(
            path.parent()
                .context("key file requires a parent directory")?,
            false,
        )?;
        // Path spelling alone misses aliases on case-insensitive filesystems.
        // Inspect every existing ancestor's device/inode as well.
        let ancestors = parent
            .ancestors()
            .map(|directory| fs::metadata(directory).map(|meta| (meta.dev(), meta.ino())))
            .collect::<io::Result<Vec<_>>>()
            .context("check key directory ancestry")?;
        for excluded in &self.forbidden_roots {
            ensure!(
                !path.starts_with(excluded),
                "key file is inside an excluded directory"
            );
            match fs::metadata(excluded) {
                Ok(meta) => ensure!(
                    !ancestors.contains(&(meta.dev(), meta.ino())),
                    "key file is inside an excluded directory"
                ),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("check excluded key directory"),
            }
        }

        // Check the opened directory and the file descriptor independently.
        // Host-owned directory ancestry is required: a same-user adversary
        // swapping ancestors between these path operations is out of scope.
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent)
            .context("open key directory")?;
        let meta = directory.metadata().context("check key directory")?;
        ensure!(
            meta.is_dir() && meta.uid() == effective_uid() && meta.mode() & 0o077 == 0,
            "key directory must be private and owner-controlled"
        );
        #[cfg(target_os = "macos")]
        check_macos_private_metadata(&directory).context("check key directory privacy")?;
        Ok((path, directory))
    }
}

impl StorageAccess {
    /// Explicit plaintext access for synthetic fixture and legacy commands.
    pub fn plaintext_fixture() -> Self {
        Self(None)
    }

    pub fn is_encrypted(&self) -> bool {
        self.0.is_some()
    }

    pub fn load_key_file(path: &Path, policy: &KeyFilePolicy) -> Result<Self> {
        let (path, _directory) = policy.key_path(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .context("open key file")?;
        check_key_file(&file).context("check key file privacy")?;

        let mut input = Zeroizing::new([0; HEX_BYTES]);
        file.read_exact(input.as_mut())
            .context("read 64 hexadecimal key bytes")?;
        let mut extra = Zeroizing::new([0]);
        if file.read(extra.as_mut()).context("check key file length")? != 0 {
            ensure!(
                extra[0] == b'\n'
                    && file.read(extra.as_mut()).context("check key file length")? == 0,
                "key file must contain 64 hexadecimal bytes and an optional newline"
            );
        }

        let mut secret = Arc::new(SecretKey(Zeroizing::new([0; KEY_BYTES])));
        let key = Arc::get_mut(&mut secret).expect("new secret has one owner");
        for (byte, pair) in key.0.iter_mut().zip(input.as_chunks::<2>().0) {
            *byte = hex_digit(pair[0])? << 4 | hex_digit(pair[1])?;
        }
        Ok(Self(Some(secret)))
    }

    pub fn generate_key_file(path: &Path, policy: &KeyFilePolicy) -> Result<Self> {
        let (path, directory) = policy.key_path(path)?;
        let mut secret = Arc::new(SecretKey(Zeroizing::new([0; KEY_BYTES])));
        let key = Arc::get_mut(&mut secret).expect("new secret has one owner");
        getrandom::fill(key.0.as_mut())
            .map_err(|_| anyhow::anyhow!("system key generation failed"))?;
        let mut encoded = Zeroizing::new([0; HEX_BYTES]);
        encode_hex(&key.0, encoded.as_mut());
        write_new_key(
            &path,
            &directory,
            encoded.as_ref(),
            check_key_file,
            |file, bytes| {
                file.write_all(bytes)?;
                file.sync_all()
            },
            File::sync_all,
        )?;
        Ok(Self(Some(secret)))
    }
}

fn normalized_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path).context("resolve key policy path")?;
    ensure!(
        !absolute
            .components()
            .any(|part| part == Component::ParentDir),
        "key policy paths cannot contain parent traversal"
    );
    Ok(absolute.components().collect())
}

fn checked_directory(path: &Path, allow_missing: bool) -> Result<PathBuf> {
    let path = normalized_absolute(path)?;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) => ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "key policy directory contains a symlink or non-directory"
            ),
            Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error).context("check key policy directory"),
        }
    }
    Ok(path)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid is a process query and does not take pointers.
    unsafe { libc::geteuid() }
}

fn check_key_file(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != effective_uid()
        || meta.nlink() != 1
        || meta.mode() & 0o7177 != 0
        || meta.mode() & 0o400 == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "key file must be a private owner-readable regular file with one link",
        ));
    }
    #[cfg(target_os = "macos")]
    check_macos_private_metadata(file)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn check_macos_private_metadata(file: &File) -> io::Result<()> {
    use std::{ffi::c_void, mem::MaybeUninit, os::fd::AsRawFd};

    // Darwin evaluates extended ACLs before BSD mode bits. Its libc exposes
    // descriptor-based ACL inspection, but the Rust libc crate omits these APIs.
    unsafe extern "C" {
        fn acl_get_fd(fd: libc::c_int) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
    }

    let fd = file.as_raw_fd();
    let mut mount = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: the borrowed File keeps fd open, and mount is valid output storage.
    if unsafe { libc::fstatfs(fd, mount.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful fstatfs initialized the output struct.
    let mount = unsafe { mount.assume_init() };
    if mount.f_flags & libc::MNT_IGNORE_OWNERSHIP as u32 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "key storage volume must enforce ownership",
        ));
    }

    // SAFETY: the borrowed File keeps fd open for this descriptor query.
    let acl = unsafe { acl_get_fd(fd) };
    if !acl.is_null() {
        // SAFETY: this is the live ACL allocation returned by Darwin libc.
        unsafe { acl_free(acl) };
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "key file and directory must have no extended ACL",
        ));
    }
    let error = io::Error::last_os_error();
    // Darwin's FILESEC_ACL query reports ENOENT when this open file has no ACL.
    match error.raw_os_error() {
        Some(libc::ENOENT) => Ok(()),
        _ => Err(error),
    }
}

fn hex_digit(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => bail!("key file contains a nonhexadecimal byte"),
    }
}

fn encode_hex(bytes: &[u8; KEY_BYTES], output: &mut [u8]) {
    for (byte, pair) in bytes.iter().copied().zip(output.as_chunks_mut::<2>().0) {
        pair[0] = HEX[usize::from(byte >> 4)];
        pair[1] = HEX[usize::from(byte & 0x0f)];
    }
}

fn write_new_key(
    path: &Path,
    directory: &File,
    bytes: &[u8],
    check_file: impl FnOnce(&File) -> io::Result<()>,
    persist: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    sync_directory: impl FnOnce(&File) -> io::Result<()>,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .context("key generation requires a new file")?;
    // An unusually restrictive umask must not leave a complete key that the
    // owner cannot reopen. This changes only the newly reserved key file.
    let result = file
        .set_permissions(fs::Permissions::from_mode(0o600))
        .and_then(|()| check_file(&file))
        .context("validate new key file privacy")
        .and_then(|()| persist(&mut file, bytes).context("write or sync key file"));
    if let Err(error) = result {
        drop(file);
        fs::remove_file(path).context("remove incomplete key file")?;
        return Err(error);
    }
    drop(file);
    sync_directory(directory).context("complete key file exists but directory sync failed")?;
    Ok(())
}

fn supported_cipher_version(version: &str) -> bool {
    let mut tokens = version.split_ascii_whitespace();
    let Some(version) = tokens.next() else {
        return false;
    };
    if !matches!(
        tokens.next(),
        None | Some("community" | "commercial" | "enterprise" | "fips")
    ) || tokens.next().is_some()
    {
        return false;
    }
    let mut components = version.split('.');
    let (Some("4"), Some(minor), Some(patch), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    minor.bytes().all(|byte| byte.is_ascii_digit())
        && minor.parse::<u32>().is_ok_and(|minor| minor >= 19)
        && patch.bytes().all(|byte| byte.is_ascii_digit())
        && patch.parse::<u32>().is_ok()
}

pub(super) fn connect_existing(
    path: &Path,
    access: &StorageAccess,
    flags: OpenFlags,
) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    if let Some(secret) = &access.0 {
        let version: String = conn
            .query_row("PRAGMA cipher_version", [], |row| row.get(0))
            .map_err(|_| anyhow::anyhow!("supported SQLCipher runtime unavailable"))?;
        ensure!(
            supported_cipher_version(&version),
            "unsupported SQLCipher runtime version"
        );

        // SQLCipher recognizes x'64hex' as a raw 32-byte key. No SQL statement
        // contains the token, and SQLCipher copies it during this call.
        let mut token = Zeroizing::new([0; HEX_BYTES + 3]);
        token[..2].copy_from_slice(b"x'");
        encode_hex(&secret.0, &mut token[2..2 + HEX_BYTES]);
        token[HEX_BYTES + 2] = b'\'';
        // SAFETY: conn is live, local and unused by any other caller. The
        // token is a valid byte buffer for the full explicit length.
        let code =
            unsafe { ffi::sqlite3_key(conn.handle(), token.as_ptr().cast(), (HEX_BYTES + 3) as _) };
        drop(token);
        ensure!(code == ffi::SQLITE_OK, "SQLCipher could not apply the key");
        ensure!(
            conn.query_row("PRAGMA cipher_use_hmac", [], |row| row.get::<_, String>(0))
                .is_ok_and(|value| value == "1")
                && conn
                    .query_row("PRAGMA cipher_plaintext_header_size", [], |row| row
                        .get::<_, String>(0))
                    .is_ok_and(|value| value == "0"),
            "SQLCipher runtime does not use the required page protection"
        );
        conn.query_row("SELECT count(*) FROM main.sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|_| {
            anyhow::anyhow!("encrypted memory schema unavailable with the selected key")
        })?;
    }
    Ok(conn)
}

/// Only a fresh private export connection may call this; no public handle or
/// trace callback is exposed. Bound key bytes never enter a SQL string.
pub(super) fn attach_snapshot_source(
    conn: &Connection,
    path: &Path,
    access: &StorageAccess,
) -> Result<()> {
    let secret = access
        .0
        .as_ref()
        .context("snapshot source must be encrypted")?;
    use std::os::unix::ffi::OsStrExt;
    ensure!(path.is_absolute(), "snapshot source path must be absolute");
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            use std::fmt::Write;
            write!(&mut uri, "%{byte:02X}").expect("String formatting");
        }
    }
    uri.push_str("?mode=ro");
    let mut token = Zeroizing::new([0; HEX_BYTES + 3]);
    token[..2].copy_from_slice(b"x'");
    encode_hex(&secret.0, &mut token[2..2 + HEX_BYTES]);
    token[HEX_BYTES + 2] = b'\'';
    let token = std::str::from_utf8(token.as_ref()).expect("ASCII key encoding");
    conn.execute(
        "ATTACH DATABASE ?1 AS snapshot_source KEY ?2",
        [uri.as_str(), token],
    )
    .map_err(|_| anyhow::anyhow!("encrypted snapshot source could not be attached"))?;
    Ok(())
}

pub(super) fn connect_new_reserved(path: &Path, access: &StorageAccess) -> Result<Connection> {
    connect_existing(path, access, OpenFlags::SQLITE_OPEN_READ_WRITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_stable_cipher_versions_at_or_above_the_qualified_release() {
        for version in ["4.19.0 community", "4.20.2 community", "4.19.1"] {
            assert!(supported_cipher_version(version), "{version}");
        }
        for version in [
            "",
            "4.14.0 community",
            "4.18.99 community",
            "4.19",
            "4.19.0-beta community",
            "4.19.0 beta community",
            "5.0.0 community",
            "4.19.0+unknown",
            "4.19.0 unknown",
            "not-a-version",
        ] {
            assert!(!supported_cipher_version(version), "{version}");
        }
    }

    #[test]
    fn failed_key_write_or_file_sync_never_leaves_a_loadable_key() -> Result<()> {
        let scratch = super::super::storage_tests::KeyScratch::new()?;
        let (path, directory) = scratch
            .policy
            .key_path(&scratch.keys.join("generated.key"))?;
        for bytes_written in [32, HEX_BYTES] {
            let error = write_new_key(
                &path,
                &directory,
                &[b'a'; HEX_BYTES],
                check_key_file,
                |file, bytes| {
                    file.write_all(&bytes[..bytes_written])?;
                    Err(io::Error::other("authored write or file sync failure"))
                },
                File::sync_all,
            )
            .unwrap_err();
            assert!(error.to_string().contains("write or sync"));
            assert!(!path.exists());
            assert!(StorageAccess::load_key_file(&path, &scratch.policy).is_err());
        }
        Ok(())
    }

    #[test]
    fn refused_empty_key_descriptor_is_removed_before_writing() -> Result<()> {
        let scratch = super::super::storage_tests::KeyScratch::new()?;
        let (path, directory) = scratch
            .policy
            .key_path(&scratch.keys.join("generated.key"))?;
        let error = write_new_key(
            &path,
            &directory,
            &[b'a'; HEX_BYTES],
            |file| {
                assert_eq!(file.metadata()?.len(), 0);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "authored descriptor refusal",
                ))
            },
            |_, _| panic!("descriptor refusal must precede writing any key bytes"),
            |_| panic!("descriptor refusal must precede directory sync"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("validate new key file privacy"));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn failed_directory_sync_leaves_a_complete_key_with_explicit_status() -> Result<()> {
        let scratch = super::super::storage_tests::KeyScratch::new()?;
        let (path, directory) = scratch
            .policy
            .key_path(&scratch.keys.join("generated.key"))?;
        let error = write_new_key(
            &path,
            &directory,
            &[b'a'; HEX_BYTES],
            check_key_file,
            |file, bytes| {
                file.write_all(bytes)?;
                file.sync_all()
            },
            |_| Err(io::Error::other("authored directory sync failure")),
        )
        .unwrap_err();
        assert!(error.to_string().contains("complete key file exists"));
        assert!(StorageAccess::load_key_file(&path, &scratch.policy)?.is_encrypted());
        assert!(StorageAccess::generate_key_file(&path, &scratch.policy).is_err());
        Ok(())
    }
}
