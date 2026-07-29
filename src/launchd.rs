//! LaunchAgent install/uninstall. mountd must run as the console user (a
//! LaunchAgent, never a LaunchDaemon): root-context mounts produce root-owned
//! volumes and can't reach the login Keychain.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub const LABEL: &str = "org.mountd.agent";

pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

/// Render the LaunchAgent plist. RunAtLoad doubles as the on-login trigger;
/// KeepAlive gives crash-restart for free.
pub fn render_plist(daemon_path: &str, log_path: &str) -> String {
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
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
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

async fn launchctl(args: &[String]) -> Result<()> {
    let out = tokio::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .await
        .context("running launchctl")?;
    if !out.status.success() {
        anyhow::bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn gui_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

/// Write the plist for the current executable and bootstrap it.
pub async fn install() -> Result<()> {
    let exe = std::env::current_exe().context("locating mountd binary")?;
    let plist = render_plist(&exe.to_string_lossy(), &log_path().to_string_lossy());
    let path = plist_path();
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(&path, plist)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    // re-bootstrap if already loaded
    let _ = launchctl(&["bootout".into(), format!("{}/{}", gui_domain(), LABEL)]).await;
    launchctl(&[
        "bootstrap".into(),
        gui_domain(),
        path.to_string_lossy().into_owned(),
    ])
    .await?;
    Ok(())
}

pub async fn uninstall() -> Result<()> {
    let _ = launchctl(&["bootout".into(), format!("{}/{}", gui_domain(), LABEL)]).await;
    let path = plist_path();
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn plist_path_is_in_launch_agents() {
        assert!(plist_path()
            .to_string_lossy()
            .contains("Library/LaunchAgents/org.mountd.agent.plist"));
    }
}
