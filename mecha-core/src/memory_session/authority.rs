use super::*;
use crate::memory::{KeyFilePolicy, RevisionRef};
use crate::memory_experience_bridge::{bounded_read, open_nofollow, write_once};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::Component,
};
pub(super) const SOURCE_MAX: usize = 4 * 1024 * 1024;
pub(super) fn digest(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn read(p: &Path, max: usize) -> Result<Vec<u8>> {
    Ok(bounded_read(p, max)?)
}
/// A dangling link is still a competing protocol claim, never absence.
pub(super) fn claim_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
pub(super) fn write(p: &Path, b: &[u8]) -> Result<()> {
    write_once(p, b)
}
fn exact_path(p: &Path, missing: bool) -> Result<()> {
    ensure!(
        p.is_absolute()
            && p.components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
        "ordinary path unavailable"
    );
    let parent = p
        .parent()
        .ok_or_else(|| anyhow::anyhow!("ordinary path unavailable"))?;
    open_nofollow(parent, true)?;
    if !missing || p.try_exists()? {
        open_nofollow(p, p.is_dir())?;
    }
    Ok(())
}
pub(super) fn private(p: &Path, directory: bool) -> Result<()> {
    let file = open_nofollow(p, directory)?;
    #[cfg(target_os = "macos")]
    crate::memory::check_macos_private_metadata(&file)?;
    let m = file.metadata()?;
    ensure!(
        m.mode() & 0o077 == 0 && m.uid() == unsafe { libc::geteuid() },
        "ordinary authority must be private"
    );
    if !directory {
        ensure!(
            m.nlink() == 1 && m.mode() & 0o7177 == 0 && m.mode() & 0o400 != 0,
            "ordinary authority aliases or permissions refused"
        );
    }
    Ok(())
}
pub(super) fn exclude_path(p: &Path, workspace: &Path) -> Result<()> {
    exact_path(p, true)?;
    let w = workspace.canonicalize()?;
    let real = p
        .parent()
        .unwrap()
        .canonicalize()?
        .join(p.file_name().unwrap());
    ensure!(
        !real.starts_with(&w),
        "ordinary authority overlaps workspace"
    );
    let wm = fs::metadata(&w)?;
    for a in real.parent().unwrap().ancestors() {
        let m = fs::metadata(a)?;
        ensure!(
            (m.dev(), m.ino()) != (wm.dev(), wm.ino()),
            "ordinary authority aliases workspace"
        );
    }
    if let Ok(m) = fs::symlink_metadata(p) {
        ensure!(
            m.is_dir() || m.nlink() == 1,
            "ordinary file aliases refused"
        );
    }
    Ok(())
}
pub(super) fn exclude_workspace(r: &SessionMemoryRequest, w: &Path) -> Result<()> {
    for p in [&r.db, &r.key_file, &r.authority] {
        exclude_path(p, w)?;
    }
    ensure!(
        !w.canonicalize()?.starts_with(&r.authority)
            && !w.canonicalize()?.starts_with(r.db.parent().unwrap()),
        "ordinary workspace overlaps storage"
    );
    Ok(())
}
pub(super) fn validate_request(r: &SessionMemoryRequest) -> Result<()> {
    ensure!(
        r.protocol == SESSION_MEMORY_PROTOCOL,
        "ordinary protocol unavailable"
    );
    ensure!(
        serde_json::to_vec(r)?.len() <= 65536,
        "ordinary request too large"
    );
    for s in [&r.domain, &r.account] {
        ensure!(
            !s.is_empty() && s.len() <= 256 && s.trim() == s && !s.chars().any(char::is_control),
            "ordinary identity unavailable"
        );
    }
    ensure!(
        r.scopes.len() <= 64
            && r.scopes
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)),
        "ordinary scopes unavailable"
    );
    for p in [&r.db, &r.key_file, &r.authority] {
        exact_path(p, true)?;
    }
    ensure!(
        !r.db.starts_with(&r.authority) && !r.authority.starts_with(&r.db) && r.db != r.key_file,
        "ordinary paths overlap"
    );
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    request: SessionMemoryRequest,
    id: String,
    instance: String,
}
#[derive(Clone)]
pub(super) struct Authority {
    pub root: PathBuf,
    pub identity: String,
    pub request: SessionMemoryRequest,
    pub instance: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Frozen {
    pub request: SessionMemoryRequest,
    pub authority: String,
    pub session_id: String,
    pub source: PathBuf,
    pub workspace: PathBuf,
    pub prompt: String,
    pub goal: String,
    pub prefix: Vec<u8>,
    pub nonce: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Attempt {
    pub input: serde_json::Value,
    pub output: String,
    pub is_error: bool,
    pub external: bool,
    pub references: Vec<RevisionRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Seal {
    pub frozen_digest: String,
    pub source_digest: String,
    pub attempts: Vec<Attempt>,
    pub payload: HostCapture,
}
pub(super) fn initialize(r: &SessionMemoryRequest) -> Result<()> {
    validate_request(r)?;
    ensure!(
        !r.db.try_exists()? && !r.authority.try_exists()? && !r.key_file.try_exists()?,
        "ordinary initialization requires new paths"
    );
    let policy = KeyFilePolicy::new(&[r.db.parent().unwrap().into(), r.authority.clone()])?;
    let storage = StorageAccess::generate_key_file(&r.key_file, &policy)?;
    fs::DirBuilder::new().mode(0o700).create(&r.authority)?;
    let m = Memory::create_supported_situated_with_storage(&r.db, &r.domain, storage)?;
    let identity = Identity {
        request: r.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        instance: m.snapshot()?.instance,
    };
    write(
        &r.authority.join("identity"),
        &serde_json::to_vec(&identity)?,
    )?;
    open_nofollow(r.authority.parent().unwrap(), true)?.sync_all()?;
    Ok(())
}
pub(super) fn open(
    r: &SessionMemoryRequest,
    w: Option<&Path>,
) -> Result<(Authority, StorageAccess)> {
    validate_request(r)?;
    if let Some(w) = w {
        exclude_workspace(r, w)?;
    }
    private(&r.authority, true)?;
    private(&r.authority.join("identity"), false)?;
    let i: Identity = serde_json::from_slice(&read(&r.authority.join("identity"), 65536)?)?;
    ensure!(i.request == *r, "ordinary request does not match authority");
    let mut excluded = vec![r.db.parent().unwrap().into(), r.authority.clone()];
    if let Some(w) = w {
        excluded.push(w.into());
    }
    let policy = KeyFilePolicy::new(&excluded)?;
    let storage = StorageAccess::load_key_file(&r.key_file, &policy)?;
    ensure!(storage.is_encrypted(), "ordinary storage must be encrypted");
    let m = Memory::open_with_storage(&r.db, storage.clone())?;
    m.check_access(&Access {
        domain: r.domain.clone(),
        scopes: r.scopes.clone(),
    })?;
    ensure!(
        m.snapshot()?.instance == i.instance,
        "ordinary database changed"
    );
    Ok((
        Authority {
            root: r.authority.clone(),
            identity: i.id,
            request: r.clone(),
            instance: i.instance,
        },
        storage,
    ))
}
impl Authority {
    pub fn validate(&self) -> Result<()> {
        validate_request(&self.request)?;
        private(&self.root, true)?;
        private(&self.root.join("identity"), false)?;
        let i: Identity = serde_json::from_slice(&read(&self.root.join("identity"), 65536)?)?;
        ensure!(
            i.id == self.identity && i.request == self.request && i.instance == self.instance,
            "ordinary authority changed"
        );
        Ok(())
    }
    pub fn file(&self, id: &str, suffix: &str) -> Result<PathBuf> {
        ensure!(
            id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "ordinary run id unavailable"
        );
        Ok(self.root.join(format!("{id}.{suffix}")))
    }
    pub fn frozen(&self, id: &str) -> Result<Frozen> {
        self.validate()?;
        let p = self.file(id, "begin")?;
        private(&p, false)?;
        let b = read(&p, SOURCE_MAX * 4)?;
        ensure!(digest(&b) == id, "ordinary begin changed");
        let f: Frozen = serde_json::from_slice(&b)?;
        ensure!(
            f.request == self.request && f.authority == self.identity,
            "ordinary begin authority changed"
        );
        ensure!(
            read(
                &self
                    .root
                    .join(format!("{}.session", digest(f.session_id.as_bytes()))),
                128
            )? == id.as_bytes(),
            "ordinary session admission changed"
        );
        Ok(f)
    }
    pub fn check_prefix(&self, f: &Frozen) -> Result<()> {
        exclude_workspace(&self.request, &f.workspace)?;
        exclude_path(&f.source, &f.workspace)?;
        self.check_source_prefix(f)
    }
    fn check_source_prefix(&self, f: &Frozen) -> Result<()> {
        self.validate()?;
        let source = open_nofollow(&f.source, false)?;
        ensure!(
            source.metadata()?.nlink() == 1,
            "ordinary source aliases refused"
        );
        ensure!(
            read(&f.source, SOURCE_MAX)?.starts_with(&f.prefix),
            "ordinary source prefix changed"
        );
        Ok(())
    }
    pub fn attempts(&self, id: &str) -> Result<Vec<Attempt>> {
        ensure!(
            !self.file(id, "overflow")?.try_exists()?,
            "ordinary recall overflow"
        );
        let mut result = vec![];
        let mut ended = false;
        for i in 0..32 {
            let p = self.file(id, &format!("attempt-{i}"))?;
            let o = self.file(id, &format!("result-{i}"))?;
            if !p.try_exists()? {
                ensure!(!o.try_exists()?, "ordinary delivery incomplete");
                ended = true;
                continue;
            }
            ensure!(!ended, "ordinary attempt gap");
            private(&p, false)?;
            private(&o, false)?;
            let input: serde_json::Value = serde_json::from_slice(&read(&p, 8192)?)?;
            let a: Attempt = serde_json::from_slice(&read(&o, 131072)?)?;
            ensure!(a.input == input, "ordinary delivery input changed");
            result.push(a);
        }
        Ok(result)
    }
    pub fn seal(&self, id: &str) -> Result<(Seal, String)> {
        self.validate()?;
        let p = self.file(id, "seal")?;
        private(&p, false)?;
        let bytes = read(&p, SOURCE_MAX)?;
        let revision = digest(&bytes);
        private(&self.file(id, "seal-hash")?, false)?;
        ensure!(
            read(&self.file(id, "seal-hash")?, 128)? == revision.as_bytes(),
            "ordinary seal changed"
        );
        Ok((serde_json::from_slice(&bytes)?, revision))
    }
    pub fn check_seal(&self, id: &str, s: &Seal) -> Result<()> {
        let f = self.frozen(id)?;
        ensure!(
            !self.file(id, "withdrawn")?.try_exists()?,
            "ordinary source withdrawn"
        );
        self.check_source_prefix(&f)?;
        ensure!(
            s.frozen_digest == id
                && s.payload.identity.session_id == f.session_id
                && s.payload.binding.authority == self.identity
                && s.payload.binding.locator == id
                && s.payload.scopes == self.request.scopes,
            "ordinary seal changed"
        );
        ensure!(
            digest(&read(&f.source, SOURCE_MAX)?) == s.source_digest
                && self.attempts(id)? == s.attempts,
            "ordinary finalized source changed"
        );
        Ok(())
    }
}
impl ExternalSourceGuard for Authority {
    fn check(&self, b: &ExternalBinding) -> SourceStatus {
        let checked = (|| -> Result<()> {
            ensure!(b.authority == self.identity, "ordinary authority mismatch");
            if claim_exists(&self.file(&b.locator, "chat-begin")?)? {
                ensure!(
                    !claim_exists(&self.file(&b.locator, "begin")?)?,
                    "ambiguous memory protocol"
                );
                return super::chat::check_binding(self, b);
            }
            let (seal, revision) = self.seal(&b.locator)?;
            ensure!(revision == b.revision, "ordinary seal mismatch");
            self.check_seal(&b.locator, &seal)
        })();
        if checked.is_ok() {
            SourceStatus::Checked
        } else {
            SourceStatus::Unavailable
        }
    }
}

pub(super) fn request_path(path: &Path, workspace: &Path) -> Result<()> {
    exclude_path(path, workspace)?;
    private(path, false)?;
    read(path, 65536)?;
    Ok(())
}
