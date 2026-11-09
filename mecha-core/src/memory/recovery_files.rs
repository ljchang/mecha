//! Exclusive encrypted staging and no-overwrite publication.
use anyhow::{ensure, Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity(u64, u64);
impl FileIdentity {
    pub(super) fn at(path: &Path) -> Result<Self> {
        let m = fs::symlink_metadata(path)?;
        ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "snapshot path is not a regular file"
        );
        Ok(Self(m.dev(), m.ino()))
    }
    pub(super) fn check(self, path: &Path) -> Result<()> {
        ensure!(Self::at(path)? == self, "snapshot file identity changed");
        Ok(())
    }
}
pub(super) struct Stage {
    pub(super) path: PathBuf,
    file: File,
    parent: File,
    identity: FileIdentity,
}
pub(super) trait FileOps {
    fn before_copy(&self, _stage: &Path) -> Result<()> {
        Ok(())
    }
    fn after_copy(&self, _stage: &Path) -> Result<()> {
        Ok(())
    }
    fn after_transform(&self, _stage: &Path) -> Result<()> {
        Ok(())
    }
    fn sync_file(&self, file: &File) -> Result<()> {
        Ok(file.sync_all()?)
    }
    fn publish(&self, stage: &Path, destination: &Path) -> Result<()> {
        Ok(fs::hard_link(stage, destination)?)
    }
    fn sync_directory(&self, directory: &File) -> Result<()> {
        Ok(directory.sync_all()?)
    }
}
pub(super) struct SystemFiles;
impl FileOps for SystemFiles {}
impl Stage {
    pub(super) fn reserve(destination: &Path) -> Result<Self> {
        let parent_path = destination.parent().context("missing snapshot parent")?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent_path)?;
        let path = parent_path.join(format!(".mecha-snapshot-{}.pending", uuid::Uuid::new_v4()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let m = file.metadata()?;
        let identity = FileIdentity(m.dev(), m.ino());
        Ok(Self {
            path,
            file,
            parent,
            identity,
        })
    }
    pub(super) fn publish(&self, destination: &Path, ops: &impl FileOps) -> Result<()> {
        self.identity.check(&self.path)?;
        ops.sync_file(&self.file)
            .context("snapshot file sync failed before publication")?;
        self.identity.check(&self.path)?;
        ops.publish(&self.path, destination)
            .context("snapshot publication failed; destination was not overwritten")?;
        ops.sync_directory(&self.parent)
            .context("snapshot published; directory durability is uncertain")?;
        self.identity.check(&self.path)?;
        fs::remove_file(&self.path).context("snapshot published; staging cleanup failed")?;
        ops.sync_directory(&self.parent)
            .context("snapshot published; staging cleanup durability is uncertain")?;
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if self.identity.check(&self.path).is_ok() {
            let _ = fs::remove_file(&self.path);
            let _ = self.parent.sync_all();
        }
    }
}
