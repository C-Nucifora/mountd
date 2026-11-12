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

use crate::mountcmd::OperationLease;
use std::path::Path;
use std::time::Duration;

/// `security` exits 44 when the requested item is not in the keychain.
const SECURITY_ITEM_NOT_FOUND: i32 = 44;

/// What happened when we asked the Keychain for a password.
#[derive(Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Found(String),
    /// No such item — the usual "you never saved this credential" case.
    Missing,
    /// The prompt was dismissed or never answered within the deadline.
    TimedOut,
    /// The child still owns its operation lock while cleanup completes.
    CleanupPending,
    /// ACL denial, a broken keychain, or unreadable output. Carries a short
    /// reason for `last_error`; never carries secret material.
    Failed(String),
}

impl std::fmt::Debug for FetchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Found(_) => f.write_str("Found([redacted])"),
            Self::Missing => f.write_str("Missing"),
            Self::TimedOut => f.write_str("TimedOut"),
            Self::CleanupPending => f.write_str("CleanupPending"),
            Self::Failed(_) => f.write_str("Failed([detail omitted])"),
        }
    }
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
            FetchOutcome::CleanupPending => "Keychain cleanup pending",
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
    lease: OperationLease,
) -> FetchOutcome {
    fetch_with_program(
        "/usr/bin/security",
        server,
        account,
        keychain,
        deadline,
        lease,
    )
    .await
}

async fn fetch_with_program(
    program: &str,
    server: &str,
    account: &str,
    keychain: Option<&Path>,
    deadline: Duration,
    lease: OperationLease,
) -> FetchOutcome {
    if deadline.is_zero() {
        return FetchOutcome::TimedOut;
    }
    fetch_inner(
        program,
        server,
        account,
        keychain,
        tokio::time::Instant::now() + deadline,
        lease,
    )
    .await
}

async fn fetch_inner(
    program: &str,
    server: &str,
    account: &str,
    keychain: Option<&Path>,
    deadline: tokio::time::Instant,
    lease: OperationLease,
) -> FetchOutcome {
    let mut cmd = tokio::process::Command::new(program);
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

    let out = match crate::mountcmd::command_output(
        cmd,
        deadline.saturating_duration_since(tokio::time::Instant::now()),
        lease.clone(),
    )
    .await
    {
        Err(e) => return command_failure(e),
        Ok(out) => out,
    };

    if !out.status.success() {
        if out.status.code() == Some(SECURITY_ITEM_NOT_FOUND) {
            return FetchOutcome::Missing;
        }
        return FetchOutcome::Failed(format!("security exited {}", out.status));
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
        match disambiguate_hex(program, server, account, keychain, deadline, lease).await {
            Ok(Some(decoded)) => return FetchOutcome::Found(decoded),
            Ok(None) => {}
            Err(outcome) => return outcome,
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
    program: &str,
    server: &str,
    account: &str,
    keychain: Option<&Path>,
    deadline: tokio::time::Instant,
    lease: OperationLease,
) -> Result<Option<String>, FetchOutcome> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Err(FetchOutcome::TimedOut);
    }
    let mut cmd = tokio::process::Command::new(program);
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

    let out = crate::mountcmd::command_output(cmd, remaining, lease)
        .await
        .map_err(command_failure)?;
    if !out.status.success() {
        return Err(FetchOutcome::Failed(
            "security disambiguation failed".into(),
        ));
    }
    // Never include this output in diagnostics: it contains the password.
    parse_password_metadata(&String::from_utf8_lossy(&out.stderr))
        .map_err(|reason| FetchOutcome::Failed(reason.into()))
}

fn command_failure(error: anyhow::Error) -> FetchOutcome {
    if error.is::<crate::mountcmd::CleanupPending>() {
        FetchOutcome::CleanupPending
    } else if error.is::<crate::mountcmd::CommandTimedOut>() {
        FetchOutcome::TimedOut
    } else {
        FetchOutcome::Failed("security subprocess failed".into())
    }
}

fn parse_password_metadata(stderr: &str) -> Result<Option<String>, &'static str> {
    let mut records = stderr
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("password:"));
    let rest = records
        .next()
        .ok_or("security password metadata missing")?
        .trim();
    if records.next().is_some() {
        return Err("ambiguous password metadata");
    }
    if let Some(hex) = rest.strip_prefix("0x") {
        let hex = hex
            .split_whitespace()
            .next()
            .ok_or("invalid password hex")?;
        if hex.len() % 2 != 0 {
            return Err("invalid password hex");
        }
        decode_hex(hex).map(Some).ok_or("invalid password hex")
    } else if rest
        .strip_prefix('"')
        .and_then(|literal| literal.strip_suffix('"'))
        .is_some()
    {
        Ok(None)
    } else {
        Err("invalid password metadata")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_fail_closed_and_debug_hides_secrets() {
        assert_eq!(
            parse_password_metadata("password: 0x70C3A4 \"...\""),
            Ok(Some("pä".into()))
        );
        assert_eq!(parse_password_metadata("password: \"deadbeef\""), Ok(None));
        assert!(parse_password_metadata("permission denied").is_err());
        assert!(parse_password_metadata("password: 0xabc").is_err());
        assert!(!format!("{:?}", FetchOutcome::Found("secret".into())).contains("secret"));
    }

    #[test]
    fn metadata_requires_distinct_literal_quotes() {
        for metadata in [
            "password: \"",
            "password: \"deadbeef",
            "password: deadbeef\"",
        ] {
            assert_eq!(
                parse_password_metadata(metadata),
                Err("invalid password metadata")
            );
        }
        for metadata in [
            "password: \"\"",
            "password: \"deadbeef\"",
            "keychain: \"fixture\"\n\tpassword: \"deadbeef\" \r\nclass: \"inet\"\n",
        ] {
            assert_eq!(parse_password_metadata(metadata), Ok(None));
        }
        assert_eq!(
            parse_password_metadata("class: \"inet\"\n  password: 0x70C3A4 \"...\"\n"),
            Ok(Some("pä".into()))
        );
        assert_eq!(
            parse_password_metadata("password: 0x6e6c0a"),
            Ok(Some("nl\n".into()))
        );
    }

    #[test]
    fn metadata_requires_exactly_one_password_record() {
        for (first, second) in [
            ("\"deadbeef\"", "0x70C3A4 \"...\""),
            ("0x70C3A4 \"...\"", "\"deadbeef\""),
            ("\"deadbeef\"", "\"deadbeef\""),
            ("0x70C3A4", "0x70C3A4"),
            ("\"", "\"deadbeef\""),
            ("\"deadbeef\"", "\""),
            ("", "\"deadbeef\""),
            ("\"deadbeef\"", ""),
        ] {
            assert_eq!(
                parse_password_metadata(&format!("password: {first}\n\tpassword: {second}\n")),
                Err("ambiguous password metadata")
            );
        }
    }

    #[test]
    fn subprocess_timeouts_keep_the_timeout_outcome() {
        assert_eq!(
            command_failure(crate::mountcmd::CommandTimedOut.into()),
            FetchOutcome::TimedOut
        );
        assert_eq!(
            command_failure(crate::mountcmd::CleanupPending.into()),
            FetchOutcome::CleanupPending
        );
        assert!(matches!(
            command_failure(anyhow::anyhow!("synthetic failure")),
            FetchOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn cancelling_either_security_call_reaps_before_releasing_lease() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{
            atomic::{AtomicI32, Ordering},
            Arc,
        };
        struct Lease(AtomicI32);
        impl Drop for Lease {
            fn drop(&mut self) {
                let pid = self.0.load(Ordering::SeqCst);
                assert!(pid > 0);
                assert_eq!(
                    unsafe { libc::kill(pid, 0) },
                    -1,
                    "credential lease released before reap"
                );
            }
        }
        for second_call in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let program = dir.path().join("security");
            let pidfile = dir.path().join("pid");
            let first = if second_call {
                "case \"$*\" in *-w*) echo deadbeef; exit 0;; esac\n"
            } else {
                ""
            };
            std::fs::write(&program, format!("#!/bin/sh\n{first}for arg; do :; done\necho $$ > \"$arg\"\nexec /bin/sleep 30\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let lease = Arc::new(Lease(AtomicI32::new(0)));
            let worker = {
                let lease = lease.clone();
                let pidfile = pidfile.clone();
                tokio::spawn(async move {
                    fetch_with_program(
                        program.to_str().unwrap(),
                        "fake",
                        "fake",
                        Some(&pidfile),
                        Duration::from_secs(30),
                        lease,
                    )
                    .await
                })
            };
            let pid = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Ok(text) = std::fs::read_to_string(&pidfile) {
                        if let Ok(pid) = text.trim().parse::<i32>() {
                            break pid;
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            lease.0.store(pid, Ordering::SeqCst);
            let retained = Arc::downgrade(&lease);
            drop(lease);
            worker.abort();
            let _ = worker.await;
            tokio::time::timeout(Duration::from_secs(2), crate::mountcmd::drain_commands())
                .await
                .unwrap();
            assert!(retained.upgrade().is_none());
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        }
    }

    #[tokio::test]
    async fn fake_security_failure_does_not_echo_secret() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("security");
        std::fs::write(
            &program,
            "#!/bin/sh\necho password=synthetic-secret >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let got = fetch_with_program(
            program.to_str().unwrap(),
            "fake",
            "fake",
            None,
            Duration::from_secs(1),
            std::sync::Arc::new(()),
        )
        .await;
        let FetchOutcome::Failed(detail) = got else {
            panic!("expected failure")
        };
        assert!(!detail.contains("synthetic-secret"));
    }

    #[tokio::test]
    async fn disambiguation_failure_cannot_return_hex_as_password() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("security");
        std::fs::write(
            &program,
            "#!/bin/sh\ncase \"$*\" in *-w*) echo deadbeef;; *) exit 1;; esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            fetch_with_program(
                program.to_str().unwrap(),
                "fake",
                "fake",
                None,
                Duration::from_secs(1),
                std::sync::Arc::new(())
            )
            .await,
            FetchOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn both_security_calls_share_one_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("security");
        std::fs::write(&program, "#!/bin/sh\ncase \"$*\" in *-w*) /bin/sleep 0.1; echo deadbeef;; *) exec /bin/sleep 30;; esac\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            fetch_with_program(
                program.to_str().unwrap(),
                "fake",
                "fake",
                None,
                Duration::from_millis(150),
                std::sync::Arc::new(())
            )
            .await,
            FetchOutcome::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

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
    #[ignore = "requires explicit macOS disposable Keychain integration run"]
    async fn fetches_password_from_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "s3cret!pw").await;
        let got = fetch_smb_password(
            "203.0.113.9",
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
            std::sync::Arc::new(()),
        )
        .await;
        assert_eq!(got, FetchOutcome::Found("s3cret!pw".into()));
        assert_eq!(got.password(), Some("s3cret!pw"));

        // wrong account -> Missing, distinctly from any other failure
        let miss = fetch_smb_password(
            "203.0.113.9",
            "other",
            Some(&kc),
            Duration::from_secs(10),
            std::sync::Arc::new(()),
        )
        .await;
        assert_eq!(miss, FetchOutcome::Missing);
        assert_eq!(miss.password(), None);

        delete_keychain(&kc).await;
    }

    #[tokio::test]
    #[ignore = "requires explicit macOS disposable Keychain integration run"]
    async fn missing_item_yields_missing() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.9", "testuser", "pw").await;
        let got = fetch_smb_password(
            "198.51.100.1", // different server, same keychain
            "testuser",
            Some(&kc),
            Duration::from_secs(10),
            std::sync::Arc::new(()),
        )
        .await;
        assert_eq!(got, FetchOutcome::Missing);
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    #[ignore = "requires explicit macOS disposable Keychain integration run"]
    async fn a_non_ascii_password_is_decoded_not_returned_as_hex() {
        // `security -w` hex-encodes anything outside printable ASCII. Passing
        // that hex string to mount_smbfs fails with "Authentication error" and
        // no hint as to why — a real trap for any password with an accent.
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.11", "accent", "pässwörd").await;
        let got = fetch_smb_password(
            "203.0.113.11",
            "accent",
            Some(&kc),
            Duration::from_secs(20),
            std::sync::Arc::new(()),
        )
        .await;
        assert_eq!(got, FetchOutcome::Found("pässwörd".into()));
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    #[ignore = "requires explicit macOS disposable Keychain integration run"]
    async fn a_password_that_merely_looks_like_hex_is_left_alone() {
        // The disambiguation must not corrupt a legitimate ASCII password
        // whose characters happen to all be hex digits.
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.12", "hexy", "deadbeef").await;
        let got = fetch_smb_password(
            "203.0.113.12",
            "hexy",
            Some(&kc),
            Duration::from_secs(20),
            std::sync::Arc::new(()),
        )
        .await;
        assert_eq!(got, FetchOutcome::Found("deadbeef".into()));
        delete_keychain(&kc).await;
    }

    #[tokio::test]
    #[ignore = "requires explicit macOS disposable Keychain integration run"]
    async fn a_password_containing_a_newline_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let kc = temp_keychain(dir.path(), "203.0.113.10", "nl", "secret\n").await;
        let got = fetch_smb_password(
            "203.0.113.10",
            "nl",
            Some(&kc),
            Duration::from_secs(20),
            std::sync::Arc::new(()),
        )
        .await;
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
        let got = fetch_smb_password(
            "203.0.113.9",
            "u",
            None,
            Duration::from_millis(0),
            std::sync::Arc::new(()),
        )
        .await;
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
