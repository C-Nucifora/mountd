//! Mount/unmount execution. v0.1 shells out to the system tools:
//!   SMB: `mount_smbfs -N` (credentials come from the login Keychain)
//!   NFS: `mount -t nfs` with a safe soft-mount option set
//! Unmounts escalate: diskutil unmount → umount → umount -f.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::{MountSpec, Protocol};

const MOUNT_TIMEOUT: Duration = Duration::from_secs(30);
const UNMOUNT_STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Expand a leading `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Percent-encode a URL component (RFC 3986 unreserved characters pass).
pub fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the SMB source URL: `//[user@]host/share`.
pub fn smb_source(username: Option<&str>, address: &str, share: &str) -> String {
    match username {
        Some(u) => format!("//{}@{}/{}", url_escape(u), address, url_escape(share)),
        None => format!("//{}/{}", address, url_escape(share)),
    }
}

/// Build the full mount argv for a spec at a chosen endpoint address.
/// `mountpoint` must already be expanded to an absolute path.
pub fn mount_argv(spec: &MountSpec, address: &str, mountpoint: &str) -> Result<Vec<String>> {
    let mut argv: Vec<String> = Vec::new();
    match spec.protocol {
        Protocol::Smb => {
            let share = spec
                .share
                .as_deref()
                .context("smb mount missing `share`")?;
            argv.push("/sbin/mount_smbfs".into());
            // -N: never prompt; credentials come from the Keychain or fail fast
            argv.push("-N".into());
            if let Some(opts) = &spec.options {
                argv.push("-o".into());
                argv.push(opts.clone());
            }
            argv.push(smb_source(spec.username.as_deref(), address, share));
        }
        Protocol::Nfs => {
            let export = spec
                .export
                .as_deref()
                .context("nfs mount missing `export`")?;
            argv.push("/sbin/mount".into());
            argv.push("-t".into());
            argv.push("nfs".into());
            let opts = spec
                .options
                .clone()
                .unwrap_or_else(|| "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp".into());
            argv.push("-o".into());
            argv.push(opts);
            argv.push(format!("{address}:{export}"));
        }
    }
    argv.push(mountpoint.into());
    Ok(argv)
}

async fn run(argv: &[String], deadline: Duration) -> Result<()> {
    let out = tokio::time::timeout(
        deadline,
        tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| format!("`{}` timed out", argv.join(" ")))?
    .with_context(|| format!("failed to spawn `{}`", argv[0]))?;
    if !out.status.success() {
        bail!(
            "`{}` failed ({}): {}",
            argv.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Mount `spec` using `address`, creating the mountpoint directory if needed.
pub async fn mount(spec: &MountSpec, address: &str) -> Result<()> {
    let mp = expand_tilde(&spec.mountpoint);
    tokio::fs::create_dir_all(&mp)
        .await
        .with_context(|| format!("creating mountpoint {}", mp.display()))?;
    let argv = mount_argv(spec, address, &mp.to_string_lossy())?;
    run(&argv, MOUNT_TIMEOUT).await
}

/// Unmount with escalation: diskutil unmount → umount → umount -f.
/// Returns Ok if any rung succeeds.
pub async fn unmount(mountpoint: &str) -> Result<()> {
    let mp = expand_tilde(mountpoint);
    let mp = mp.to_string_lossy().to_string();
    let ladder: [Vec<String>; 3] = [
        vec!["/usr/sbin/diskutil".into(), "unmount".into(), mp.clone()],
        vec!["/sbin/umount".into(), mp.clone()],
        vec!["/sbin/umount".into(), "-f".into(), mp.clone()],
    ];
    let mut last_err = None;
    for argv in &ladder {
        match run(argv, UNMOUNT_STEP_TIMEOUT).await {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Protocol};

    fn specs() -> Config {
        Config::from_toml_str(
            r#"
[[mount]]
name = "smb1"
protocol = "smb"
share = "media library"
username = "chris"
mountpoint = "~/mnt/smb1"

  [[mount.endpoint]]
  address = "192.168.1.5"

[[mount]]
name = "nfs1"
protocol = "nfs"
export = "/mnt/tank/scratch"
mountpoint = "~/mnt/nfs1"

  [[mount.endpoint]]
  address = "100.64.0.11"
"#,
        )
        .unwrap()
    }

    #[test]
    fn smb_argv_uses_keychain_and_escapes_share() {
        let cfg = specs();
        let argv = mount_argv(cfg.mount("smb1").unwrap(), "192.168.1.5", "/Users/u/mnt/smb1")
            .unwrap();
        assert_eq!(
            argv,
            vec![
                "/sbin/mount_smbfs",
                "-N",
                "//chris@192.168.1.5/media%20library",
                "/Users/u/mnt/smb1",
            ]
        );
    }

    #[test]
    fn smb_source_without_username() {
        assert_eq!(smb_source(None, "host", "share"), "//host/share");
    }

    #[test]
    fn nfs_argv_defaults_to_soft_mount_options() {
        let cfg = specs();
        let argv =
            mount_argv(cfg.mount("nfs1").unwrap(), "100.64.0.11", "/Users/u/mnt/nfs1").unwrap();
        assert_eq!(
            argv,
            vec![
                "/sbin/mount",
                "-t",
                "nfs",
                "-o",
                "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp",
                "100.64.0.11:/mnt/tank/scratch",
                "/Users/u/mnt/nfs1",
            ]
        );
    }

    #[test]
    fn explicit_options_pass_through() {
        let mut cfg = specs();
        let m = &mut cfg.mounts[1];
        m.options = Some("soft,vers=4".into());
        assert!(m.protocol == Protocol::Nfs);
        let argv = mount_argv(m, "h", "/mp").unwrap();
        assert!(argv.contains(&"soft,vers=4".to_string()));
    }

    #[test]
    fn tilde_expansion() {
        let p = expand_tilde("~/mnt/x");
        assert!(p.is_absolute());
        assert!(p.ends_with("mnt/x"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn url_escaping() {
        assert_eq!(url_escape("plain-name_1.2~"), "plain-name_1.2~");
        assert_eq!(url_escape("has space/slash"), "has%20space%2Fslash");
    }
}
