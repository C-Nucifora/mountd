//! SMB credential fetch from the login Keychain via /usr/bin/security.
//!
//! `mount_smbfs -N` does NOT consult the Keychain (it only reads ~/.nsmbrc);
//! Keychain-backed mounting is a NetFS/NetAuthAgent feature. So the daemon
//! fetches the password itself and embeds it in the mount URL. The item is
//! a standard "smb " internet password — the same kind Finder (⌘K with
//! "Remember") creates, so existing entries just work. The first read from
//! an item created by another app triggers one macOS ACL prompt; "Always
//! Allow" makes subsequent reads silent.

use std::path::Path;
use std::time::Duration;

/// Fetch the SMB password for `server`/`account` from the Keychain.
/// `keychain` overrides the default search list (used by tests).
/// Returns None on missing item, denied ACL, or timeout (e.g. a dismissed
/// permission prompt) — the caller falls back to passwordless mounting.
pub async fn fetch_smb_password(
    server: &str,
    account: &str,
    keychain: Option<&Path>,
    deadline: Duration,
) -> Option<String> {
    let mut cmd = tokio::process::Command::new("/usr/bin/security");
    cmd.args([
        "find-internet-password",
        "-s",
        server,
        "-a",
        account,
        "-r",
        "smb ",
        "-w",
    ]);
    if let Some(kc) = keychain {
        cmd.arg(kc);
    }
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(deadline, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let pw = String::from_utf8(out.stdout).ok()?;
    let pw = pw.trim_end_matches('\n');
    if pw.is_empty() {
        None
    } else {
        Some(pw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a throwaway keychain holding one smb internet password.
    async fn temp_keychain(
        dir: &Path,
        server: &str,
        account: &str,
        pw: &str,
    ) -> std::path::PathBuf {
        let kc = dir.join("test.keychain-db");
        let run = |args: Vec<String>| async move {
            let out = tokio::process::Command::new("/usr/bin/security")
                .args(&args)
                .output()
                .await
                .unwrap();
            assert!(
                out.status.success(),
                "security {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(vec![
            "create-keychain".into(),
            "-p".into(),
            "test".into(),
            kc.to_string_lossy().into_owned(),
        ])
        .await;
        run(vec![
            "unlock-keychain".into(),
            "-p".into(),
            "test".into(),
            kc.to_string_lossy().into_owned(),
        ])
        .await;
        run(vec![
            "add-internet-password".into(),
            "-s".into(),
            server.into(),
            "-a".into(),
            account.into(),
            "-r".into(),
            "smb ".into(),
            "-w".into(),
            pw.into(),
            "-T".into(),
            "/usr/bin/security".into(),
            kc.to_string_lossy().into_owned(),
        ])
        .await;
        kc
    }

    #[tokio::test]
    async fn fetches_password_from_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "s3cret!pw").await;
        let pw = fetch_smb_password(
            "203.0.113.9",
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(pw.as_deref(), Some("s3cret!pw"));
        // wrong account -> None
        let miss =
            fetch_smb_password("203.0.113.9", "other", Some(&kc), Duration::from_secs(10)).await;
        assert_eq!(miss, None);
        let _ = tokio::process::Command::new("/usr/bin/security")
            .args(["delete-keychain", &kc.to_string_lossy()])
            .output()
            .await;
    }

    #[tokio::test]
    async fn missing_item_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "pw").await;
        let pw = fetch_smb_password(
            "198.51.100.1", // different server, same keychain
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(pw, None);
        let _ = tokio::process::Command::new("/usr/bin/security")
            .args(["delete-keychain", &kc.to_string_lossy()])
            .output()
            .await;
    }
}
