//! Provision a verified, compressed browser for Cargo's target, never the host.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Command;

use flate2::{Compression, GzBuilder, read::GzDecoder};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=assets/lightpanda-pins.txt");
    for key in ["WS_LIGHTPANDA_ASSET_DIR", "WS_LIGHTPANDA_OFFLINE"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let target = env::var("TARGET")?;
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let embedded = output.join("lightpanda.gz");
    let asset = match target.as_str() {
        "aarch64-apple-darwin" => Some("lightpanda-aarch64-macos"),
        "x86_64-apple-darwin" => Some("lightpanda-x86_64-macos"),
        "aarch64-unknown-linux-gnu" => Some("lightpanda-aarch64-linux"),
        "x86_64-unknown-linux-gnu" => Some("lightpanda-x86_64-linux"),
        _ => None,
    };
    let Some(asset) = asset else {
        fs::write(&embedded, [])?;
        println!("cargo:rustc-env=WS_BROWSER_HASH=");
        println!("cargo:rustc-env=WS_BROWSER_SIZE=0");
        println!("cargo:rustc-env=WS_BROWSER_ASSET=unsupported");
        return Ok(());
    };
    let (size, hash) = pin(asset)?;
    let cache = env::var_os("WS_LIGHTPANDA_ASSET_DIR").map_or_else(
        || {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
                .join("target/lightpanda-assets")
        },
        PathBuf::from,
    );
    fs::create_dir_all(&cache)?;
    let compressed = cache.join(format!("{asset}-{hash}.gz"));
    if compressed.exists() {
        verify(GzDecoder::new(File::open(&compressed)?), size, hash)?;
    } else {
        let raw = cache.join(asset);
        if !raw.exists() {
            if env::var_os("WS_LIGHTPANDA_OFFLINE").is_some() {
                return Err(format!("missing pinned {asset}; place the upstream executable in WS_LIGHTPANDA_ASSET_DIR, or build once online to populate the verified cache").into());
            }
            let download = cache.join(format!(".{asset}-{}.download", std::process::id()));
            let url =
                format!("https://github.com/lightpanda-io/browser/releases/download/1.0.0/{asset}");
            let status = Command::new("curl")
                .env_clear()
                .args([
                    "--disable",
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--max-time",
                    "300",
                    "--output",
                ])
                .arg(&download)
                .arg(url)
                .status()?;
            if !status.success() {
                let _ = fs::remove_file(&download);
                return Err(format!("could not provision {asset}; use WS_LIGHTPANDA_ASSET_DIR for cached/offline builds").into());
            }
            verify(File::open(&download)?, size, hash)?;
            fs::rename(download, &raw)?;
        }
        verify(File::open(&raw)?, size, hash)?;
        let staging = cache.join(format!(".{asset}-{}.gz", std::process::id()));
        let mut encoder = GzBuilder::new()
            .mtime(0)
            .write(File::create(&staging)?, Compression::best());
        let _ = io::copy(&mut File::open(&raw)?, &mut encoder)?;
        encoder.finish()?.sync_all()?;
        verify(GzDecoder::new(File::open(&staging)?), size, hash)?;
        fs::rename(staging, &compressed)?;
    }
    let _ = fs::copy(&compressed, &embedded)?;
    println!("cargo:rustc-env=WS_BROWSER_HASH={hash}");
    println!("cargo:rustc-env=WS_BROWSER_SIZE={size}");
    println!("cargo:rustc-env=WS_BROWSER_ASSET={asset}");
    Ok(())
}

fn pin(asset: &str) -> Result<(u64, &'static str), Box<dyn Error>> {
    include_str!("assets/lightpanda-pins.txt")
        .lines()
        .find_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            match fields.as_slice() {
                [name, size, hash] if *name == asset => {
                    Some(size.parse().map(|size| (size, *hash)))
                }
                _ => None,
            }
        })
        .ok_or("missing asset pin")?
        .map_err(Into::into)
}

fn verify(
    mut reader: impl Read,
    expected_size: u64,
    expected_hash: &str,
) -> Result<(), Box<dyn Error>> {
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(count)?)
            .ok_or("asset size overflow")?;
        if size > expected_size {
            return Err("browser asset exceeds pinned size".into());
        }
        hash.write_all(buffer.get(..count).ok_or("invalid read length")?)?;
    }
    if size != expected_size || format!("{:x}", hash.finalize()) != expected_hash {
        return Err("Lightpanda asset checksum/size mismatch; refusing to embed it".into());
    }
    Ok(())
}
