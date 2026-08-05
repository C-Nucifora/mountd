//! SMB credential fetch from the login Keychain via /usr/bin/security.
//!
//! `mount_smbfs -N` does NOT consult the Keychain (it only reads ~/.nsmbrc);
//! Keychain-backed mounting is a NetFS/NetAuthAgent feature. So the daemon
//! fetches the password itself and embeds it in the mount URL. The item is
//! a standard "smb " internet password — the same kind Finder (⌘K with
//! "Remember") creates, so existing entries just work. The first read from
//! an item created by another app triggers one macOS ACL prompt; "Always
//! Allow" makes subsequent reads silent.
//!
//! Failures are reported precisely rather than collapsed into "no password":
//! silently mounting without a credential can succeed as guest against a
//! server with anonymous access, leaving the user authenticated as nobody
//! with no signal anywhere.

use std::path::Path;
use std::time::Duration;

/// `security` exits 44 when the requested item is not in the keychain.
const SECURITY_ITEM_NOT_FOUND: i32 = 44;

/// What happened when we asked the Keychain for a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Found(String),
    /// No such item — the usual "you never saved this credential" case.
    Missing,
    /// The prompt was dismissed or never answered within the deadline.
    TimedOut,
    /// ACL denial, a broken keychain, or unreadable output. Carries a short
    /// reason for `last_error`; never carries secret material.
    Failed(String),
}

impl FetchOutcome {
    pub fn password(&self) -> Option<&str> {
        match self {
            FetchOutcome::Found(p) => Some(p),
            _ => None,
        }
    }

    /// A short phrase for status output, e.g. "no Keychain item".
    pub fn reason(&self) -> &str {
        match self {
            FetchOutcome::Found(_) => "credential found",
            FetchOutcome::Missing => "no Keychain item",
            FetchOutcome::TimedOut => "Keychain prompt not answered",
            FetchOutcome::Failed(_) => "Keychain read failed",
        }
    }
}

/// Fetch the SMB password for `server`/`account` from the Keychain.
/// `keychain` overrides the default search list (used by tests).
pub async fn fetch_smb_password(
    server: &str,
    account: &str,
    keychain: Option<&Path>,
    deadline: Duration,
) -> FetchOutcome {
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

    let out = match tokio::time::timeout(deadline, cmd.output()).await {
        Err(_) => return FetchOutcome::TimedOut,
        Ok(Err(e)) => return FetchOutcome::Failed(format!("spawning security: {e}")),
        Ok(Ok(out)) => out,
    };

    if !out.status.success() {
        if out.status.code() == Some(SECURITY_ITEM_NOT_FOUND) {
            return FetchOutcome::Missing;
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("security exited {}", out.status)
        } else {
            stderr
        };
        return FetchOutcome::Failed(detail);
    }

    let pw = match String::from_utf8(out.stdout) {
        Ok(pw) => pw,
        Err(_) => return FetchOutcome::Failed("keychain item is not valid UTF-8".into()),
    };
    // `security -w` appends exactly one newline; a password that genuinely
    // ends in newlines must survive intact.
    let raw = pw.strip_suffix('\n').unwrap_or(&pw).to_string();
    if raw.is_empty() {
        return FetchOutcome::Missing;
    }

    // `-w` prints the password hex-encoded whenever it contains anything
    // outside printable ASCII — an accented letter, an emoji, a tab. Using
    // that hex string as the password fails authentication with no clue why.
    // Only `-g` distinguishes it from a password that merely looks like hex
    // ("deadbeef"), so pay for that second call in the rare ambiguous case.
    if looks_like_hex(&raw) {
        if let Some(decoded) = disambiguate_hex(server, account, keychain, deadline).await {
            return FetchOutcome::Found(decoded);
        }
    }
    FetchOutcome::Found(raw)
}

/// Even-length lowercase hex is how `security -w` renders a non-printable
/// password — and also how a password like "deadbeef" looks.
fn looks_like_hex(s: &str) -> bool {
    s.len() >= 2
        && s.len().is_multiple_of(2)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode_hex(s: &str) -> Option<String> {
    let bytes: Vec<u8> = s
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<_>>()?;
    String::from_utf8(bytes).ok()
}

/// Ask `security -g`, which prints `password: 0x<HEX>  "..."` for encoded
/// values and `password: "..."` for literal ones. Returns the decoded password
/// only when the item really was hex-encoded.
async fn disambiguate_hex(
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
        "-g",
    ]);
    if let Some(kc) = keychain {
        cmd.arg(kc);
    }
    cmd.kill_on_drop(true);

    let out = tokio::time::timeout(deadline, cmd.output())
        .await
        .ok()?
        .ok()?;
    // The password goes to stderr; the item's attributes go to stdout.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("password:"))?;
    let rest = line.split_once("password:")?.1.trim();
    let hex = rest.strip_prefix("0x")?;
    let hex = hex.split_whitespace().next()?;
    decode_hex(&hex.to_ascii_lowercase())
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

    async fn delete_keychain(kc: &Path) {
        let _ = tokio::process::Command::new("/usr/bin/security")
            .args(["delete-keychain", &kc.to_string_lossy()])
            .output()
            .await;
    }

    #[tokio::test]
    async fn fetches_password_from_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "s3cret!pw").await;
        let got = fetch_smb_password(
            "203.0.113.9",
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(got, FetchOutcome::Found("s3cret!pw".into()));
        assert_eq!(got.password(), Some("s3cret!pw"));

        // wrong account -> Missing, distinctly from any other failure
        let miss =
            fetch_smb_password("203.0.113.9", "other", Some(&kc), Duration::from_secs(10)).await;
        assert_eq!(miss, FetchOutcome::Missing);
        assert_eq!(miss.password(), None);

        delete_keychain(&kc).await;
    }

    #[tokio::test]
    async fn missing_item_yields_missing() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "pw").await;
        let got = fetch_smb_password(
            "198.51.100.1", // different server, same keychain
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(got, FetchOutcome::Missing);
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    async fn a_non_ascii_password_is_decoded_not_returned_as_hex() {
        // `security -w` hex-encodes anything outside printable ASCII. Passing
        // that hex string to mount_smbfs fails with "Authentication error" and
        // no hint as to why — a real trap for any password with an accent.
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.11", "accent", "pässwörd").await;
        let got =
            fetch_smb_password("203.0.113.11", "accent", Some(&kc), Duration::from_secs(20)).await;
        assert_eq!(got, FetchOutcome::Found("pässwörd".into()));
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    async fn a_password_that_merely_looks_like_hex_is_left_alone() {
        // The disambiguation must not corrupt a legitimate ASCII password
        // whose characters happen to all be hex digits.
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.12", "hexy", "deadbeef").await;
        let got =
            fetch_smb_password("203.0.113.12", "hexy", Some(&kc), Duration::from_secs(20)).await;
        assert_eq!(got, FetchOutcome::Found("deadbeef".into()));
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    async fn a_password_containing_a_newline_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.10", "nl", "secret\n").await;
        let got =
            fetch_smb_password("203.0.113.10", "nl", Some(&kc), Duration::from_secs(20)).await;
        assert_eq!(got, FetchOutcome::Found("secret\n".into()));
        delete_keychain(&kc).await;
    }

    #[test]
    fn hex_detection_and_decoding() {
        assert!(looks_like_hex("70c3a4"));
        assert!(looks_like_hex("deadbeef"));
        assert!(!looks_like_hex("plain123")); // 'p' is not a hex digit
        assert!(!looks_like_hex("abc")); // odd length
        assert!(!looks_like_hex("")); // empty
        assert!(!looks_like_hex("DEADBEEF")); // security emits lowercase

        assert_eq!(
            decode_hex("70c3a4737377c3b67264").as_deref(),
            Some("pässwörd")
        );
        assert_eq!(decode_hex("6e6c0a").as_deref(), Some("nl\n"));
        assert_eq!(decode_hex("zz"), None);
    }

    #[tokio::test]
    async fn timeout_is_distinguishable_from_missing() {
        // A zero deadline cannot complete, so the outcome must be TimedOut and
        // never silently look like "no credential".
        let got = fetch_smb_password("203.0.113.9", "u", None, Duration::from_millis(0)).await;
        assert_eq!(got, FetchOutcome::TimedOut);
    }

    #[test]
    fn reasons_are_human_readable() {
        assert_eq!(FetchOutcome::Missing.reason(), "no Keychain item");
        assert_eq!(
            FetchOutcome::TimedOut.reason(),
            "Keychain prompt not answered"
        );
        assert_eq!(
            FetchOutcome::Failed("x".into()).reason(),
            "Keychain read failed"
        );
    }
}
