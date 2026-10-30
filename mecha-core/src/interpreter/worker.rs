//! The pinned monty worker: which binary a pool may spawn, where mecha puts
//! it, and the check that a file *is* that binary before anything runs it.
//!
//! **The binary is identified by its own sha256, never by its path.** A
//! fresh mtime is not a fresh build, and a `monty` on `PATH` is whatever
//! somebody installed last; [`verify`] hashes the file it is handed against
//! the pin compiled in for this platform and refuses anything else, so the
//! only way to run a different worker is to change a reviewed constant.
//!
//! **The source is PyPI's `pydantic-monty-runtime` wheel** — monty's own
//! release channel for the worker (the `monty` CLI from the `monty-runtime`
//! crate), at the version `monty-pool` is pinned to in `Cargo.toml`, since
//! the two speak one wire protocol. On Linux the musllinux wheel's binary is
//! statically linked, which is what lets the sandbox bind that one file and
//! nothing else (`super::monty`). Two hashes are pinned per platform: the
//! wheel's, read from PyPI's JSON API when the pin was authored and checked
//! by [`crate::fetch`] before a byte is unpacked, and the binary's, checked
//! after unpacking and again every time a pool starts.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::sidecar::Manifest;

/// The monty release the worker and `monty-pool` are pinned to. Moving it is
/// one PR: this constant, the four pins below, and the two `=` versions in
/// `mecha-core/Cargo.toml`.
pub const MONTY_VERSION: &str = "1.4.0";

/// The sidecar id the manifest records the install under.
pub const SIDECAR: &str = "monty";

/// One platform's worker: the wheel (its path under PyPI's file host, sha256
/// and size) and the binary inside it (sha256 and size).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    pub target: &'static str,
    pub wheel_path: &'static str,
    pub wheel_sha256: &'static str,
    pub wheel_bytes: u64,
    pub binary_sha256: &'static str,
    pub binary_bytes: u64,
}

/// The binary's path inside every wheel of this release.
const WHEEL_MEMBER: &str = "pydantic_monty_runtime-1.4.0.data/scripts/monty";

/// Read from `https://pypi.org/pypi/pydantic-monty-runtime/1.4.0/json` on
/// 2026-10-30; the binary hashes from each wheel's `RECORD`, confirmed by
/// hashing the unpacked file. Only these committed values are trusted.
const PINS: &[Pin] = &[
    Pin {
        target: "aarch64-linux",
        wheel_path: "80/85/317a61daca57f3211d53f868977dbd1d491e0212082f0e79cd6df50137de/pydantic_monty_runtime-1.4.0-py3-none-musllinux_1_1_aarch64.whl",
        wheel_sha256: "ae4e68d623b9038f8a41baa95e06e8d32402ad972120b068d14c336bb924eb03",
        wheel_bytes: 12_315_943,
        binary_sha256: "989eb5bacc6844fce2472820d28f0ab3247846d29ab62d0dbb521004f3224cc7",
        binary_bytes: 25_375_416,
    },
    Pin {
        target: "x86_64-linux",
        wheel_path: "50/c4/fc238425268af5ca9642d5ae997cb1101c95b87e7876ceabf8275b42a1d3/pydantic_monty_runtime-1.4.0-py3-none-musllinux_1_1_x86_64.whl",
        wheel_sha256: "af1f5d41c32207a2ecd9a9c4277290911c6e5f37299588097c7ff83240bb665c",
        wheel_bytes: 13_021_113,
        binary_sha256: "84357dfc96b0480e8fc99e30734775b75b58624c49c1a3ab1bf2fc2ae753439d",
        binary_bytes: 29_730_128,
    },
    Pin {
        target: "aarch64-macos",
        wheel_path: "cb/9b/0eedee19d738a168cb4fbcefb1be38cb7c729c750d0293379483e38fa71a/pydantic_monty_runtime-1.4.0-py3-none-macosx_11_0_arm64.whl",
        wheel_sha256: "317edf6c62ea62c8a0f456ec13be889387cd16a917441fc087953cae042e6d75",
        wheel_bytes: 13_763_884,
        binary_sha256: "6c1d5b14023d213b4a862131739c5431ac980b5f89a3267c75fb6be2c2d39fdd",
        binary_bytes: 27_764_848,
    },
    Pin {
        target: "x86_64-macos",
        wheel_path: "7e/6e/654bf4e3867e48564c7f4dabf03023660c860f53a0065915c4fa1e816825/pydantic_monty_runtime-1.4.0-py3-none-macosx_10_12_x86_64.whl",
        wheel_sha256: "9ba7ad3163cbf601d7d36e5c9b6edb7b1625f27683a910885682d224edb8c05e",
        wheel_bytes: 12_643_236,
        binary_sha256: "7cbbd968b33f45ebcca3822f5303738d19fa2c40e75bb5f9d04f03bd0ed1c556",
        binary_bytes: 27_686_664,
    },
];

/// This machine's pin, if mecha has one for it.
pub fn pin() -> Option<&'static Pin> {
    let target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    PINS.iter().find(|p| p.target == target)
}

/// Where [`ensure`] puts the worker: `~/.mecha/sidecars/monty/<version>/monty`.
pub fn installed_path(mecha_home: &Path) -> PathBuf {
    mecha_home
        .join("sidecars")
        .join(SIDECAR)
        .join(MONTY_VERSION)
        .join("monty")
}

/// A file proven, by its sha256, to be this platform's pinned worker.
/// The only way to make one is [`verify`], so a pool cannot be handed a path
/// nobody checked.
#[derive(Debug, Clone)]
pub struct VerifiedWorker {
    path: PathBuf,
}

impl VerifiedWorker {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Hash `path` against this platform's pin. Absolute and canonical on
/// success, because the sandbox binds it by that name.
pub fn verify(path: &Path) -> Result<VerifiedWorker> {
    let pin = pin().with_context(|| {
        format!(
            "mecha pins no monty worker for {}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    let path = path
        .canonicalize()
        .with_context(|| format!("no monty worker at {}", path.display()))?;
    let len = std::fs::metadata(&path)?.len();
    if len != pin.binary_bytes {
        bail!(
            "{} is {len} bytes, not the pinned monty {MONTY_VERSION} worker ({} bytes)",
            path.display(),
            pin.binary_bytes
        );
    }
    let sha = crate::fetch::sha256_file(&path)?;
    if sha != pin.binary_sha256 {
        bail!(
            "{} does not hash to the pinned monty {MONTY_VERSION} worker (sha256 {sha}, pinned {})",
            path.display(),
            pin.binary_sha256
        );
    }
    Ok(VerifiedWorker { path })
}

/// The installed worker, verified; an error naming the fix when it is
/// absent or does not match.
pub fn installed(mecha_home: &Path) -> Result<VerifiedWorker> {
    let p = installed_path(mecha_home);
    if !p.exists() {
        bail!(
            "the monty {MONTY_VERSION} worker is not installed at {}",
            p.display()
        );
    }
    verify(&p)
}

/// Fetch, unpack and verify the pinned worker into
/// [`installed_path`]; a verified install already there is returned as is.
/// The manifest records the directory before anything is written, as every
/// sidecar install does, so an interrupted one is resumed, not trusted.
pub async fn ensure(mecha_home: &Path, progress: &mut dyn FnMut(u64)) -> Result<VerifiedWorker> {
    let dest = installed_path(mecha_home);
    if let Ok(w) = verify(&dest) {
        return Ok(w);
    }
    let pin = pin().with_context(|| {
        format!(
            "mecha pins no monty worker for {}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    let dir = dest.parent().context("an install path with no parent")?;
    Manifest::begin(mecha_home, SIDECAR)?;
    Manifest::record(mecha_home, SIDECAR, dir)?;
    let download = dir.join("download");
    let wheel = crate::fetch::fetch_pypi_file(
        pin.wheel_path,
        pin.wheel_sha256,
        pin.wheel_bytes,
        &download,
        progress,
    )
    .await?;
    let bytes = std::fs::read(&wheel)?;
    let binary = zip_member(&bytes, WHEEL_MEMBER, pin.binary_bytes)?;
    let part = dir.join("monty.part");
    std::fs::write(&part, &binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = verify(&part) {
        let _ = std::fs::remove_file(&part);
        return Err(e.context("the unpacked worker does not match its pin"));
    }
    std::fs::rename(&part, &dest)?;
    let _ = std::fs::remove_dir_all(&download);
    Manifest::finish(mecha_home, SIDECAR)?;
    verify(&dest)
}

/// One member of a zip archive, by name, through its central directory —
/// stored or deflated, the two methods a wheel uses. `limit` bounds the
/// inflated size, so a corrupt entry cannot inflate without end; the caller
/// hashes the result anyway.
pub(crate) fn zip_member(zip: &[u8], name: &str, limit: u64) -> Result<Vec<u8>> {
    let u16_at = |at: usize| -> Result<u16> {
        zip.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .context("a truncated zip")
    };
    let u32_at = |at: usize| -> Result<u32> {
        zip.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .context("a truncated zip")
    };
    // The end-of-central-directory record: 22 bytes plus a comment of at
    // most 65,535, so it is searched for backwards over that window.
    let floor = zip.len().saturating_sub(22 + 65_535);
    let eocd = (floor..=zip.len().saturating_sub(22))
        .rev()
        .find(|&i| u32_at(i).ok() == Some(0x0605_4b50))
        .context("not a zip archive: no end of central directory")?;
    let entries = u16_at(eocd + 10)? as usize;
    let mut at = u32_at(eocd + 16)? as usize;
    for _ in 0..entries {
        if u32_at(at)? != 0x0201_4b50 {
            bail!("a corrupt zip central directory");
        }
        let method = u16_at(at + 10)?;
        let compressed = u32_at(at + 20)? as usize;
        let size = u32_at(at + 24)? as u64;
        let name_len = u16_at(at + 28)? as usize;
        let extra_len = u16_at(at + 30)? as usize;
        let comment_len = u16_at(at + 32)? as usize;
        let local = u32_at(at + 42)? as usize;
        let entry_name = zip
            .get(at + 46..at + 46 + name_len)
            .context("a truncated zip")?;
        at += 46 + name_len + extra_len + comment_len;
        if entry_name != name.as_bytes() {
            continue;
        }
        if size > limit {
            bail!("{name} is {size} bytes in the archive, more than the {limit} expected");
        }
        if u32_at(local)? != 0x0403_4b50 {
            bail!("a corrupt zip local header for {name}");
        }
        let start = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let data = zip
            .get(start..start + compressed)
            .context("a truncated zip entry")?;
        return match method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(size as usize);
                flate2::read::DeflateDecoder::new(data)
                    .take(limit + 1)
                    .read_to_end(&mut out)
                    .with_context(|| format!("inflating {name}"))?;
                if out.len() as u64 != size {
                    bail!("{name} inflated to {} bytes, not {size}", out.len());
                }
                Ok(out)
            }
            m => bail!("{name} uses zip compression method {m}, which a wheel does not"),
        };
    }
    bail!("the archive holds no {name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A one- or two-entry zip, written by hand the way the format says.
    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in entries {
            let body = if *deflate {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0; 8]); // time, date, crc (unchecked here)
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&body);
            central.extend(0x0201_4b50u32.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0; 8]);
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]); // extra, comment, disk, attrs
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        let cd_len = central.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend(cd_len.to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn a_member_is_read_stored_or_deflated_and_by_its_exact_name() {
        let payload = b"not really an ELF".repeat(50);
        let zip = zip_of(&[
            ("a/monty.txt", b"decoy", false),
            ("a/monty", &payload, true),
        ]);
        assert_eq!(zip_member(&zip, "a/monty", 10_000).unwrap(), payload);
        assert_eq!(zip_member(&zip, "a/monty.txt", 10).unwrap(), b"decoy");
        assert!(zip_member(&zip, "monty", 10_000).is_err());
    }

    #[test]
    fn a_member_larger_than_the_pin_expects_is_refused_before_inflating() {
        let zip = zip_of(&[("m", &[7u8; 4096], true)]);
        let err = zip_member(&zip, "m", 100).unwrap_err().to_string();
        assert!(err.contains("more than the 100 expected"), "{err}");
        assert!(zip_member(b"plainly not a zip", "m", 100).is_err());
    }

    #[test]
    fn every_pin_is_complete_and_names_this_release() {
        for p in PINS {
            assert_eq!(p.wheel_sha256.len(), 64);
            assert_eq!(p.binary_sha256.len(), 64);
            assert!(p
                .wheel_path
                .contains(&format!("pydantic_monty_runtime-{MONTY_VERSION}-")));
            assert!(
                WHEEL_MEMBER.starts_with(&format!("pydantic_monty_runtime-{MONTY_VERSION}.data/"))
            );
        }
    }

    /// The path a pool binds is checked by content: a file of the right
    /// length and the wrong bytes is refused, and so is a missing one.
    #[test]
    fn verify_refuses_a_file_that_is_not_the_pinned_binary() {
        let Some(pin) = pin() else { return };
        let dir = std::env::temp_dir().join(format!("mecha-monty-pin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("monty");
        // Sparse: the right length, all zeros, no 25 MB written.
        std::fs::File::create(&fake)
            .unwrap()
            .set_len(pin.binary_bytes)
            .unwrap();
        let err = verify(&fake).unwrap_err().to_string();
        assert!(err.contains("does not hash to the pinned"), "{err}");
        std::fs::write(&fake, b"short").unwrap();
        assert!(verify(&fake).is_err());
        assert!(verify(&dir.join("absent")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The version string, the wheel member and the Cargo pins move together.
    #[test]
    fn the_worker_pin_and_the_pool_crate_are_the_same_release() {
        let manifest = include_str!("../../Cargo.toml");
        for krate in ["monty-pool", "monty-types"] {
            let line = manifest
                .lines()
                .find(|l| l.starts_with(&format!("{krate} =")))
                .unwrap_or_else(|| panic!("{krate} is not in mecha-core/Cargo.toml"));
            assert!(
                line.contains(&format!("\"={MONTY_VERSION}\"")),
                "{krate} must be pinned to ={MONTY_VERSION}: {line}"
            );
        }
    }
}
