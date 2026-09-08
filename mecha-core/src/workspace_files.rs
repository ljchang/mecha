//! Directory-relative attachment I/O. The workspace is trusted; its contents
//! are not. Hold directory descriptors across creation/open so a replaced
//! `inbox` or intermediate path cannot redirect an operation through a symlink.

use crate::tool::ToolCtx;
use std::ffi::{CString, OsStr};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

pub struct WorkspaceFiles {
    root: PathBuf,
    directory: File,
}

fn component(name: &OsStr) -> io::Result<CString> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one filename",
        ));
    }
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in filename"))
}

fn open_at(parent: &File, name: &OsStr, flags: i32) -> io::Result<File> {
    let name = component(name)?;
    // SAFETY: parent is a live directory descriptor; name is NUL terminated.
    // A successful descriptor is transferred exactly once to File.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

impl WorkspaceFiles {
    pub fn open(workspace: &Path) -> io::Result<Self> {
        let root = workspace.canonicalize()?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root)?;
        Ok(Self { root, directory })
    }

    /// Atomically reserve a free inbox filename. Existing files, including
    /// dangling symlinks, are collisions; they are never opened or overwritten.
    pub fn upload(&self, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        component(OsStr::new(name))?;
        // SAFETY: live directory fd and a static NUL-terminated component.
        if unsafe { libc::mkdirat(self.directory.as_raw_fd(), c"inbox".as_ptr(), 0o700) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        let inbox = open_at(
            &self.directory,
            OsStr::new("inbox"),
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        // A bounded collision search prevents a populated directory wedging
        // a request. Each candidate is reserved by the kernel, not exists().
        for n in 1..=1000 {
            let candidate = if n == 1 {
                name.to_owned()
            } else {
                match name.rsplit_once('.') {
                    Some((stem, ext)) => format!("{stem}-{n}.{ext}"),
                    None => format!("{name}-{n}"),
                }
            };
            match open_at(
                &inbox,
                OsStr::new(&candidate),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            ) {
                Ok(mut file) => {
                    file.write_all(bytes)?;
                    return Ok(Path::new("inbox").join(candidate));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many files with this name; rename the upload",
        ))
    }

    /// Resolve through the normal jail, then open each canonical component
    /// relative to a held descriptor without following replacement symlinks.
    /// Return a regular file handle, never a FIFO/device that can block a read.
    pub fn read(&self, path: &str) -> io::Result<(File, PathBuf)> {
        let ctx = ToolCtx {
            workspace: self.root.clone(),
            spill_dir: None,
            ..ToolCtx::default()
        };
        let target = ctx
            .resolve(path)
            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e.to_string()))?;
        let relative = target.strip_prefix(&self.root).map_err(io::Error::other)?;
        let mut parts = relative.components().peekable();
        let mut parent = self.directory.try_clone()?;
        while let Some(part) = parts.next() {
            let Component::Normal(name) = part else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "invalid path component",
                ));
            };
            if parts.peek().is_some() {
                parent = open_at(&parent, name, libc::O_RDONLY | libc::O_DIRECTORY)?;
            } else {
                let file = open_at(&parent, name, libc::O_RDONLY | libc::O_NONBLOCK)?;
                if !file.metadata()?.is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "not a regular file",
                    ));
                }
                return Ok((file, target));
            }
        }
        Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_uploads_reserve_distinct_files() {
        let root = std::env::temp_dir().join(format!("mecha-upload-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let files = WorkspaceFiles::open(&root).unwrap();
        let names = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..12)
                .map(|n| {
                    let files = &files;
                    scope.spawn(move || (files.upload("photo.jpg", &[n]).unwrap(), n))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        for (name, n) in names {
            assert_eq!(std::fs::read(root.join(name)).unwrap(), [n]);
        }
        for invalid in ["../escape", "/tmp/escape", "..", "", "sub/file"] {
            assert!(files.upload(invalid, b"x").is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn downloads_allow_internal_links_and_refuse_external_links_and_directories() {
        let root = std::env::temp_dir().join(format!("mecha-download-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), "inside").unwrap();
        std::os::unix::fs::symlink("file", root.join("link")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("outside")).unwrap();
        let files = WorkspaceFiles::open(&root).unwrap();
        assert!(files.read("link").is_ok());
        assert!(files.read("outside").is_err());
        assert!(files.read(".").is_err());
        assert!(files.read("../elsewhere").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
