//! LaunchAgent install/uninstall. mountd must run as the console user (a
//! LaunchAgent, never a LaunchDaemon): root-context mounts produce root-owned
//! volumes and can't reach the login Keychain.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

pub const LABEL: &str = "org.mountd.agent";

pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

/// Render the LaunchAgent plist. RunAtLoad doubles as the on-login trigger;
/// KeepAlive restarts after a crash but not after a clean exit — an
/// unconditional KeepAlive turns `mountctl uninstall`, and any deliberate
/// shutdown, into a respawn loop.
pub fn render_plist(daemon_path: &str, log_path: &str) -> String {
    render_plist_with_args(daemon_path, log_path, &[])
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn render_plist_with_args(daemon_path: &str, log_path: &str, args: &[String]) -> String {
    let daemon_path = xml_escape(daemon_path);
    let log_path = xml_escape(log_path);
    let args = args
        .iter()
        .map(|arg| format!("        <string>{}</string>\n", xml_escape(arg)))
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon_path}</string>
        <string>run</string>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Background</string>
    <key>ExitTimeOut</key>
    <integer>0</integer>
    <key>StandardOutPath</key>
    <string>{log_path}</string>
    <key>StandardErrorPath</key>
    <string>{log_path}</string>
</dict>
</plist>
"#
    )
}

fn log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Logs/mountd.log")
}

async fn command_output(program: &str, args: &[String]) -> Result<std::process::Output> {
    run_command(program, args, Duration::from_secs(10)).await
}

async fn run_command(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output> {
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    crate::mountcmd::command_output(command, timeout, std::sync::Arc::new(())).await
}

async fn launchctl(args: &[String]) -> Result<()> {
    let out = command_output("/bin/launchctl", args)
        .await
        .map_err(UnsettledMutation)?;
    if out.status.code().is_none() {
        return Err(
            UnsettledMutation(anyhow::anyhow!("launchctl client terminated by signal")).into(),
        );
    }
    anyhow::ensure!(
        out.status.success(),
        "launchctl {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

async fn is_loaded() -> Result<bool> {
    let target = format!("{}/{}", gui_domain(), LABEL);
    let out = command_output("/bin/launchctl", &["print".into(), target.clone()]).await?;
    if !out.status.success() {
        let error = String::from_utf8_lossy(&out.stderr);
        // Only absence is harmless. Permission and domain errors must not be
        // hidden while replacing files beneath a still-running service.
        anyhow::ensure!(
            error.contains("Could not find service") && error.contains(LABEL),
            "cannot inspect {target}: {}",
            error.trim()
        );
        return Ok(false);
    }
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
#[error("launchd mutation outcome is uncertain; settle the service before retrying: {0}")]
struct UnsettledMutation(anyhow::Error);

async fn verify_mutation(
    result: Result<()>,
    expected_loaded: bool,
    inspect: impl std::future::Future<Output = Result<bool>>,
) -> Result<()> {
    // A snapshot cannot order after a request whose client timed out or still exists.
    if result
        .as_ref()
        .is_err_and(|error| error.is::<UnsettledMutation>())
    {
        return result;
    }
    let loaded = inspect.await.map_err(UnsettledMutation)?;
    if loaded == expected_loaded {
        return Ok(());
    }
    result?;
    Err(UnsettledMutation(anyhow::anyhow!(
        "service has not reached the requested state"
    ))
    .into())
}

async fn bootout() -> Result<bool> {
    if !is_loaded().await? {
        return Ok(false);
    }
    let result = launchctl(&["bootout".into(), format!("{}/{}", gui_domain(), LABEL)]).await;
    verify_mutation(result, false, is_loaded()).await?;
    Ok(true)
}

async fn bootstrap(path: &std::path::Path) -> Result<()> {
    let result = launchctl(&[
        "bootstrap".into(),
        gui_domain(),
        path.to_string_lossy().into_owned(),
    ])
    .await;
    verify_mutation(result, true, is_loaded()).await
}

// Save recovery bytes before any service or plist mutation can occur.
fn backup_plist(path: &std::path::Path, previous: Option<&[u8]>) -> Result<Option<PathBuf>> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let Some(previous) = previous else {
        return Ok(None);
    };
    let backup = path.with_extension(format!("plist.recovery.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&backup)
        .context("saving previous plist before installation")?;
    file.write_all(previous)?;
    Ok(Some(backup))
}

fn preserved_args(args: Vec<String>) -> Result<Vec<String>> {
    anyhow::ensure!(
        args.get(1).map(String::as_str) == Some("run"),
        "existing LaunchAgent does not run mountd; refusing to overwrite it"
    );
    Ok(args.into_iter().skip(2).collect())
}

fn gui_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

/// The daemon binary the LaunchAgent should run: the `mountd` that sits next
/// to the calling executable (mountctl and mountd install side by side).
pub fn daemon_path(current_exe: &std::path::Path) -> Result<PathBuf> {
    let dir = current_exe
        .parent()
        .context("executable has no parent directory")?;
    let daemon = dir.join("mountd");
    anyhow::ensure!(
        daemon.is_file(),
        "mountd binary not found next to {} — install both binaries together \
         (e.g. `cargo install --path .`)",
        current_exe.display()
    );
    Ok(daemon)
}

/// Write the plist for the sibling mountd binary and bootstrap it.
pub async fn install() -> Result<()> {
    let exe = std::env::current_exe().context("locating current binary")?;
    let daemon = daemon_path(&exe)?;
    let path = plist_path();
    let args = if path.try_exists()? {
        let out = command_output(
            "/usr/bin/plutil",
            &[
                "-extract".into(),
                "ProgramArguments".into(),
                "json".into(),
                "-o".into(),
                "-".into(),
                path.to_string_lossy().into_owned(),
            ],
        )
        .await?;
        anyhow::ensure!(
            out.status.success(),
            "cannot read existing LaunchAgent; leaving it unchanged"
        );
        preserved_args(
            serde_json::from_slice(&out.stdout).context("reading existing ProgramArguments")?,
        )?
    } else {
        Vec::new()
    };
    let plist = render_plist_with_args(
        &daemon.to_string_lossy(),
        &log_path().to_string_lossy(),
        &args,
    );
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    if let Some(dir) = log_path().parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let previous = if path.try_exists()? {
        Some(tokio::fs::read(&path).await?)
    } else {
        None
    };
    let backup = backup_plist(&path, previous.as_deref())?;
    let mut restored = false;
    let result = async {
        let was_loaded = bootout().await?;
        let apply = async {
            tokio::fs::write(&path, plist)
                .await
                .with_context(|| format!("writing {}", path.display()))?;
            bootstrap(&path).await
        }
        .await;
        if let Err(error) = apply {
            if error.is::<UnsettledMutation>() {
                return Err(error.context("attempted plist retained; automatic rollback skipped"));
            }
            bootout()
                .await
                .context("cannot settle registration for rollback; attempted plist retained")?;
            if let Some(previous) = previous {
                tokio::fs::write(&path, previous)
                    .await
                    .context("restoring previous LaunchAgent")?;
                if was_loaded {
                    bootstrap(&path)
                        .await
                        .context("restarting previous LaunchAgent")?;
                }
            } else if path.try_exists()? {
                tokio::fs::remove_file(&path).await?;
            }
            restored = true;
            return Err(error);
        }
        Ok(())
    }
    .await;
    if let Some(backup) = backup {
        if result.is_ok() || restored {
            tokio::fs::remove_file(&backup)
                .await
                .context("removing completed recovery backup")?;
        } else {
            return result
                .with_context(|| format!("previous plist retained at {}", backup.display()));
        }
    }
    result
}

pub async fn uninstall() -> Result<()> {
    bootout().await?;
    let path = plist_path();
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_or_timed_out_mutations_cannot_be_promoted_by_a_snapshot() {
        for cause in [
            anyhow::Error::from(crate::mountcmd::CleanupPending),
            anyhow::Error::from(crate::mountcmd::CommandTimedOut),
        ] {
            let inspected = std::sync::atomic::AtomicBool::new(false);
            let result = verify_mutation(Err(UnsettledMutation(cause).into()), true, async {
                inspected.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(true)
            })
            .await;
            assert!(result.unwrap_err().is::<UnsettledMutation>());
            assert!(!inspected.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(verify_mutation(Ok(()), true, async { Ok(false) })
            .await
            .unwrap_err()
            .is::<UnsettledMutation>());
    }

    #[test]
    fn recovery_backup_precedes_mutation_and_cannot_overwrite_another_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.plist");
        std::fs::write(&path, b"previous").unwrap();
        let backup = backup_plist(&path, Some(b"previous")).unwrap().unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), b"previous");
        assert_eq!(
            std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(backup_plist(&path, Some(b"replacement")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read(&backup).unwrap(), b"previous");
    }

    #[test]
    fn plist_escapes_paths_and_preserves_custom_config() {
        let args = preserved_args(vec![
            "/old/mountd".into(),
            "run".into(),
            "--config".into(),
            "/tmp/a&b.toml".into(),
        ])
        .unwrap();
        let plist = render_plist_with_args("/tmp/a<b/mountd", "/tmp/a&b.log", &args);
        assert!(plist.contains("/tmp/a&lt;b/mountd"));
        assert!(plist.contains("/tmp/a&amp;b.log"));
        assert!(plist.contains("<string>--config</string>"));
        assert!(plist.contains("/tmp/a&amp;b.toml"));
        assert!(preserved_args(vec!["mountd".into(), "check".into()]).is_err());
    }

    #[tokio::test]
    async fn command_failure_is_reported() {
        assert!(command_output("/no/such/synthetic-command", &[])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn command_deadline_stops_an_owned_synthetic_child() {
        let start = std::time::Instant::now();
        let result = run_command(
            "/bin/sh",
            &["-c".into(), "exec sleep 5".into()],
            Duration::from_millis(30),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn plist_contains_program_and_keepalive() {
        let p = render_plist("/usr/local/bin/mountd", "/tmp/mountd.log");
        assert!(p.contains("<string>/usr/local/bin/mountd</string>"));
        assert!(p.contains("<string>run</string>"));
        assert!(p.contains("<key>KeepAlive</key>"));
        assert!(p.contains("<key>RunAtLoad</key>"));
        assert!(p.contains(LABEL));
    }

    #[test]
    fn keepalive_does_not_respawn_after_a_clean_exit() {
        // An unconditional <true/> makes `mountctl uninstall` and any
        // deliberate shutdown into a restart loop.
        let p = render_plist("/usr/local/bin/mountd", "/tmp/mountd.log");
        assert!(p.contains("<key>SuccessfulExit</key>"), "{p}");
        assert!(!p.contains("<key>KeepAlive</key>\n    <true/>"), "{p}");
        assert!(
            p.contains("<key>ExitTimeOut</key>\n    <integer>0</integer>"),
            "{p}"
        );
    }

    #[test]
    fn daemon_path_resolves_sibling_mountd_not_caller() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = dir.path().join("mountctl");
        let daemon = dir.path().join("mountd");
        std::fs::write(&ctl, b"").unwrap();
        std::fs::write(&daemon, b"").unwrap();
        // called from mountctl, the plist must still point at mountd
        assert_eq!(daemon_path(&ctl).unwrap(), daemon);
        assert_eq!(daemon_path(&daemon).unwrap(), daemon);
    }

    #[test]
    fn daemon_path_errors_when_mountd_missing() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = dir.path().join("mountctl");
        std::fs::write(&ctl, b"").unwrap();
        let err = daemon_path(&ctl).unwrap_err().to_string();
        assert!(err.contains("mountd binary not found"), "{err}");
    }

    #[test]
    fn plist_path_is_in_launch_agents() {
        assert!(plist_path()
            .to_string_lossy()
            .contains("Library/LaunchAgents/org.mountd.agent.plist"));
    }
}
