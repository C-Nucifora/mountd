//! Mount/unmount execution. Shells out to the system tools:
//!   SMB: `mount_smbfs -N -o soft` with a Keychain-fetched credential in the URL
//!   NFS: `mount -t nfs` with a safe soft-mount option set
//! Unmounts escalate politely by default (`diskutil unmount` → `umount`) and
//! only reach for force (`diskutil unmount force` → `umount -f`) when the
//! caller says so — a user asked, or the server is provably gone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::{MountSpec, Protocol};

/// Bound on a single mount attempt. Once the probe gate has passed, SMB
/// negotiation either completes or fails quickly; a longer window mostly
/// serves to keep the credential visible in `ps` (see `smb_source`).
const MOUNT_TIMEOUT: Duration = Duration::from_secs(15);
const UNMOUNT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Creating the mountpoint directory touches the filesystem at that path,
/// which can block if a stale mount is still covering it.
const MKDIR_TIMEOUT: Duration = Duration::from_secs(5);

/// Expand a leading `~` or `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve a configured mountpoint to the path the kernel will report.
///
/// Trailing slashes are trimmed and the *parent* is canonicalized so a
/// symlinked component (`~/mnt` → elsewhere) still compares equal to
/// `f_mntonname`. The mountpoint itself is never canonicalized or stat'd —
/// doing so would block on a dead mount, which is exactly what this daemon
/// exists to avoid.
pub fn normalize_mountpoint(path: &str) -> PathBuf {
    let trimmed = path.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { path } else { trimmed };
    let expanded = expand_tilde(trimmed);
    match (expanded.parent(), expanded.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            match std::fs::canonicalize(parent) {
                Ok(real) => real.join(name),
                Err(_) => expanded,
            }
        }
        _ => expanded,
    }
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

/// Render an endpoint address for use in a URL authority: IPv6 literals need
/// brackets, everything else passes through.
fn url_host(address: &str) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    }
}

/// Build the SMB source URL: `//[user[:password]@]host/share`.
/// The password rides in the mount argv (mount_smbfs offers no other
/// non-interactive channel — `-N` reads ~/.nsmbrc, not the Keychain), so it is
/// visible to local `ps` for the lifetime of the attempt, bounded by
/// `MOUNT_TIMEOUT`. It is never stored.
pub fn smb_source(
    username: Option<&str>,
    password: Option<&str>,
    address: &str,
    share: &str,
) -> String {
    let host = url_host(address);
    match (username, password) {
        (Some(u), Some(p)) => format!(
            "//{}:{}@{}/{}",
            url_escape(u),
            url_escape(p),
            host,
            url_escape(share)
        ),
        (Some(u), None) => format!("//{}@{}/{}", url_escape(u), host, url_escape(share)),
        (None, _) => format!("//{}/{}", host, url_escape(share)),
    }
}

/// Default mount options per protocol. Both are soft: a dead server must make
/// file operations fail, not block Finder forever.
fn default_options(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Smb => "soft",
        Protocol::Nfs => "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp",
    }
}

/// Build the full mount argv for a spec at a chosen endpoint address.
/// `mountpoint` must already be expanded to an absolute path.
pub fn mount_argv(
    spec: &MountSpec,
    address: &str,
    mountpoint: &str,
    password: Option<&str>,
) -> Result<Vec<String>> {
    let options = spec
        .options
        .clone()
        .unwrap_or_else(|| default_options(spec.protocol).to_string());
    let mut argv: Vec<String> = Vec::new();
    match spec.protocol {
        Protocol::Smb => {
            let share = spec.share.as_deref().context("smb mount missing `share`")?;
            argv.push("/sbin/mount_smbfs".into());
            // -N: never prompt; the credential comes from the Keychain or the
            // attempt fails fast rather than hanging on an invisible dialog.
            argv.push("-N".into());
            if !options.is_empty() {
                argv.push("-o".into());
                argv.push(options);
            }
            argv.push(smb_source(
                spec.username.as_deref(),
                password,
                address,
                share,
            ));
        }
        Protocol::Nfs => {
            let export = spec
                .export
                .as_deref()
                .context("nfs mount missing `export`")?;
            argv.push("/sbin/mount".into());
            argv.push("-t".into());
            argv.push("nfs".into());
            if !options.is_empty() {
                argv.push("-o".into());
                argv.push(options);
            }
            argv.push(format!("{}:{}", url_host(address), export));
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

/// Redact the password in an error message that echoes the mount argv.
/// Both the percent-encoded form (what we put on the command line) and the raw
/// form (in case a tool echoes a decoded URL) are replaced.
fn redact(msg: &str, password: Option<&str>) -> String {
    match password {
        Some(p) if !p.is_empty() => msg
            .replace(&format!(":{}@", url_escape(p)), ":****@")
            .replace(&format!(":{p}@"), ":****@"),
        _ => msg.to_string(),
    }
}

/// Mount `spec` using `address`, creating the mountpoint directory if needed.
///
/// Refuses to mount over a path that already carries a mount: stacking a second
/// mount on one mountpoint takes two unmounts to clear and confuses every
/// subsequent reconcile.
pub async fn mount(spec: &MountSpec, address: &str, password: Option<&str>) -> Result<()> {
    let mp = normalize_mountpoint(&spec.mountpoint);
    let mp_str = mp.to_string_lossy().to_string();

    if crate::mtab::find_by_mountpoint(&crate::mtab::list(), &mp_str).is_some() {
        bail!("already mounted at {mp_str}");
    }

    ensure_mountpoint(&mp).await?;

    let argv = mount_argv(spec, address, &mp_str, password)?;
    run(&argv, MOUNT_TIMEOUT)
        .await
        .map_err(|e| anyhow::anyhow!("{}", redact(&format!("{e:#}"), password)))
}

/// Create the mountpoint directory, bounded in time. `create_dir_all` stats the
/// path when it already exists, which blocks if a stale mount covers it.
async fn ensure_mountpoint(mp: &Path) -> Result<()> {
    let owned = mp.to_path_buf();
    match tokio::time::timeout(MKDIR_TIMEOUT, tokio::fs::create_dir_all(&owned)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e).with_context(|| format!("creating mountpoint {}", mp.display())),
        Err(_) => bail!("mountpoint {} is unresponsive (stale mount?)", mp.display()),
    }
}

/// Unmount with escalation. Polite rungs always run first; `force` adds the
/// rungs that can yank a volume out from under an open file, so it is reserved
/// for explicit user requests and provably dead servers.
/// Returns Ok if any rung succeeds.
pub async fn unmount(mountpoint: &str, force: bool) -> Result<()> {
    let mp = normalize_mountpoint(mountpoint);
    let mp = mp.to_string_lossy().to_string();
    let mut ladder = unmount_ladder(&mp, force);

    let mut last_err = None;
    for argv in ladder.drain(..) {
        match run(&argv, UNMOUNT_STEP_TIMEOUT).await {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no unmount rung ran for {mp}")))
}

/// The escalation ladder for a mountpoint. Split out so the policy — polite
/// rungs always, forceful rungs only on request — is directly testable.
fn unmount_ladder(mp: &str, force: bool) -> Vec<Vec<String>> {
    let mut ladder: Vec<Vec<String>> = vec![
        vec!["/usr/sbin/diskutil".into(), "unmount".into(), mp.into()],
        vec!["/sbin/umount".into(), mp.into()],
    ];
    if force {
        ladder.push(vec![
            "/usr/sbin/diskutil".into(),
            "unmount".into(),
            "force".into(),
            mp.into(),
        ]);
        ladder.push(vec!["/sbin/umount".into(), "-f".into(), mp.into()]);
    }
    ladder
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
    fn smb_argv_defaults_to_a_soft_mount() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("smb1").unwrap(),
            "192.168.1.5",
            "/Users/u/mnt/smb1",
            None,
        )
        .unwrap();
        assert_eq!(
            argv,
            vec![
                "/sbin/mount_smbfs",
                "-N",
                "-o",
                "soft",
                "//chris@192.168.1.5/media%20library",
                "/Users/u/mnt/smb1",
            ]
        );
    }

    #[test]
    fn smb_argv_embeds_escaped_password() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("smb1").unwrap(),
            "192.168.1.5",
            "/Users/u/mnt/smb1",
            Some("p@ss:w/rd"),
        )
        .unwrap();
        assert!(argv.contains(&"//chris:p%40ss%3Aw%2Frd@192.168.1.5/media%20library".to_string()));
    }

    #[test]
    fn smb_source_without_username() {
        assert_eq!(smb_source(None, None, "host", "share"), "//host/share");
        // password without username is ignored
        assert_eq!(
            smb_source(None, Some("pw"), "host", "share"),
            "//host/share"
        );
    }

    #[test]
    fn ipv6_literals_are_bracketed() {
        assert_eq!(
            smb_source(None, None, "fd7a::1", "share"),
            "//[fd7a::1]/share"
        );
        let cfg = specs();
        let argv = mount_argv(cfg.mount("nfs1").unwrap(), "fd7a::1", "/mp", None).unwrap();
        assert!(argv.contains(&"[fd7a::1]:/mnt/tank/scratch".to_string()));
    }

    #[test]
    fn redact_strips_password_from_errors() {
        let msg = "`/sbin/mount_smbfs -N //u:p%40w@h/s /mp` failed";
        assert_eq!(
            redact(msg, Some("p@w")),
            "`/sbin/mount_smbfs -N //u:****@h/s /mp` failed"
        );
        assert_eq!(redact(msg, None), msg);
    }

    #[test]
    fn redact_also_strips_the_decoded_form() {
        // a tool echoing a decoded URL must not defeat redaction
        let msg = "mount error: //u:p@w@h/s";
        assert_eq!(redact(msg, Some("p@w")), "mount error: //u:****@h/s");
    }

    #[test]
    fn nfs_argv_defaults_to_soft_mount_options() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("nfs1").unwrap(),
            "100.64.0.11",
            "/Users/u/mnt/nfs1",
            None,
        )
        .unwrap();
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
        let argv = mount_argv(m, "h", "/mp", None).unwrap();
        assert!(argv.contains(&"soft,vers=4".to_string()));
        assert!(!argv.contains(&"soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp".to_string()));
    }

    #[test]
    fn explicit_empty_options_suppress_the_flag() {
        let mut cfg = specs();
        let m = &mut cfg.mounts[0];
        m.options = Some(String::new());
        let argv = mount_argv(m, "h", "/mp", None).unwrap();
        assert!(!argv.contains(&"-o".to_string()));
    }

    #[test]
    fn tilde_expansion() {
        let p = expand_tilde("~/mnt/x");
        assert!(p.is_absolute());
        assert!(p.ends_with("mnt/x"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert!(expand_tilde("~").is_absolute());
    }

    #[test]
    fn normalize_trims_trailing_slashes() {
        let a = normalize_mountpoint("/tmp/mountd-test-x");
        let b = normalize_mountpoint("/tmp/mountd-test-x/");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_resolves_a_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let via_link = normalize_mountpoint(&format!("{}/share", link.display()));
        let via_real = normalize_mountpoint(&format!("{}/share", real.display()));
        assert_eq!(via_link, via_real);
        // the mountpoint component itself need not exist
        assert!(via_link.ends_with("share"));
    }

    #[test]
    fn normalize_leaves_unknown_parents_alone() {
        let p = normalize_mountpoint("/nonexistent-parent-xyz/share");
        assert_eq!(p, PathBuf::from("/nonexistent-parent-xyz/share"));
    }

    #[test]
    fn polite_ladder_never_forces() {
        let polite = unmount_ladder("/mp", false);
        assert_eq!(polite.len(), 2);
        assert!(!polite.iter().any(|a| a.contains(&"-f".to_string())));
        assert!(!polite.iter().any(|a| a.contains(&"force".to_string())));
    }

    #[test]
    fn forced_ladder_adds_the_forceful_rungs_last() {
        let forced = unmount_ladder("/mp", true);
        assert_eq!(forced.len(), 4);
        // polite rungs still come first
        assert_eq!(forced[0][1], "unmount");
        assert!(!forced[0].contains(&"force".to_string()));
        assert!(forced[2].contains(&"force".to_string()));
        assert!(forced[3].contains(&"-f".to_string()));
    }

    #[tokio::test]
    async fn unmount_of_an_unmounted_path_reports_the_last_rung() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-mounted");
        std::fs::create_dir(&path).unwrap();
        let err = unmount(&path.to_string_lossy(), false)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("umount"), "{err}");
    }

    #[tokio::test]
    async fn mount_refuses_to_stack_on_an_existing_mount() {
        // "/" is always mounted; a spec pointing there must be refused before
        // any mount command runs.
        let cfg = Config::from_toml_str(
            r#"
[[mount]]
name = "root"
protocol = "smb"
share = "s"
mountpoint = "/"
  [[mount.endpoint]]
  address = "192.0.2.1"
"#,
        )
        .unwrap();
        let err = mount(cfg.mount("root").unwrap(), "192.0.2.1", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("already mounted"), "{err}");
    }

    #[test]
    fn url_escaping() {
        assert_eq!(url_escape("plain-name_1.2~"), "plain-name_1.2~");
        assert_eq!(url_escape("has space/slash"), "has%20space%2Fslash");
    }
}
