//! Checksum-verified, atomic installation of the embedded browser.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::{config::Env, error::Error};

/// Pinned browser version; source and executable pins live in `assets/`.
pub const VERSION: &str = "1.0.0";
/// Override the runtime browser cache root (never an executable override).
pub const ENV_CACHE: &str = "WS_CACHE_DIR";
const COMPRESSED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lightpanda.gz"));

/// Extracts the bundled executable on first use and verifies it on every reuse.
///
/// # Errors
/// Unsupported platforms, unsafe cache paths, corrupt assets or filesystem errors.
pub fn install(env: Env<'_>) -> Result<PathBuf, Error> {
    if env!("WS_BROWSER_HASH").is_empty() {
        return Err(Error::Config("Lightpanda is unavailable for this build target; use --backend cloudflare (native bundles support macOS and glibc Linux on x86_64/aarch64)".to_owned()));
    }
    let base = env(ENV_CACHE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env("XDG_CACHE_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            env("HOME").filter(|value| !value.is_empty()).map(|home| {
                PathBuf::from(home).join(if cfg!(target_os = "macos") {
                    "Library/Caches"
                } else {
                    ".cache"
                })
            })
        })
        .ok_or_else(|| Error::Config(format!("cannot locate a browser cache; set {ENV_CACHE}")))?;
    fs::create_dir_all(&base)?;
    let root = base.join("ws");
    private_directory(&root)?;
    let version = root.join(format!(
        "lightpanda-{VERSION}-{}-{}",
        env!("WS_BROWSER_ASSET"),
        env!("WS_BROWSER_HASH")
    ));
    private_directory(&version)?;
    let size = env!("WS_BROWSER_SIZE")
        .parse()
        .map_err(|_| Error::Config("invalid embedded browser size".to_owned()))?;
    unpack(&version, COMPRESSED, size, env!("WS_BROWSER_HASH"))
}

fn private_directory(path: &Path) -> Result<(), Error> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        let _ = builder.mode(0o700);
        builder
    };
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Config(
            "browser cache directory must be a real private directory".to_owned(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Config(
                "browser cache directory must not be accessible to other users (permissions 0700)"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn checked_file(path: &Path, size: u64, hash: &str) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != size {
        return Err(Error::Config(
            "cached Lightpanda has an invalid type/size; remove this browser cache entry and retry"
                .to_owned(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o277 != 0 {
            return Err(Error::Config(
                "cached Lightpanda must be a private, read-only executable".to_owned(),
            ));
        }
    }
    copy_verified(File::open(path)?, &mut std::io::sink(), size, hash)
}

fn copy_verified(
    mut input: impl Read,
    output: &mut impl Write,
    expected: u64,
    hash: &str,
) -> Result<(), Error> {
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(
                u64::try_from(count)
                    .map_err(|_| Error::Config("browser asset is too large".to_owned()))?,
            )
            .ok_or_else(|| Error::Config("browser asset is too large".to_owned()))?;
        if size > expected {
            return Err(Error::Config(
                "browser asset exceeds its pinned size".to_owned(),
            ));
        }
        let bytes = buffer
            .get(..count)
            .ok_or_else(|| Error::Config("invalid browser asset read".to_owned()))?;
        digest.update(bytes);
        output.write_all(bytes)?;
    }
    if size != expected || format!("{:x}", digest.finalize()) != hash {
        return Err(Error::Config(
            "Lightpanda checksum mismatch; refusing to run this browser".to_owned(),
        ));
    }
    Ok(())
}

fn unpack(directory: &Path, compressed: &[u8], size: u64, hash: &str) -> Result<PathBuf, Error> {
    let executable = directory.join("lightpanda");
    if executable.try_exists()? {
        checked_file(&executable, size, hash)?;
        return Ok(executable);
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    copy_verified(GzDecoder::new(compressed), &mut temporary, size, hash)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o500))?;
    }
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(&executable) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            checked_file(&executable, size, hash)?;
        }
        Err(error) => return Err(error.error.into()),
    }
    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn fixture() -> (Vec<u8>, String) {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(b"pinned executable fixture").unwrap();
        (
            encoder.finish().unwrap(),
            format!("{:x}", Sha256::digest(b"pinned executable fixture")),
        )
    }

    #[test]
    fn concurrent_install_and_reuse_publish_only_verified_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let (compressed, hash) = fixture();
        thread_scope(dir.path(), &compressed, &hash);
        assert_eq!(
            fs::read(unpack(dir.path(), &compressed, 25, &hash).unwrap()).unwrap(),
            b"pinned executable fixture"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn thread_scope(path: &Path, compressed: &[u8], hash: &str) {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let _ = scope.spawn(move || {
                    assert!(unpack(path, compressed, 25, hash).is_ok());
                });
            }
        });
    }

    #[test]
    fn checksum_mismatch_and_oversize_are_never_published() {
        let dir = tempfile::tempdir().unwrap();
        let (compressed, hash) = fixture();
        assert!(unpack(dir.path(), &compressed, 24, &hash).is_err());
        assert!(unpack(dir.path(), &compressed, 25, "bad").is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_corrupt_existing_files_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let (compressed, hash) = fixture();
        let target = dir.path().join("external");
        fs::write(&target, b"pinned executable fixture").unwrap();
        symlink(&target, dir.path().join("lightpanda")).unwrap();
        assert!(unpack(dir.path(), &compressed, 25, &hash).is_err());
        fs::remove_file(dir.path().join("lightpanda")).unwrap();
        let executable = unpack(dir.path(), &compressed, 25, &hash).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&executable, b"corrupt executable bytes!").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(unpack(dir.path(), &compressed, 25, &hash).is_err());
        symlink(dir.path(), dir.path().join("linked-directory")).unwrap();
        assert!(private_directory(&dir.path().join("linked-directory")).is_err());
    }
}
