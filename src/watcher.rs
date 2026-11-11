//! Event-driven reconcile hints from the routing socket, via a supervised
//! `/sbin/route -n monitor` child process. Events are only hints — the
//! engine debounces them and re-evaluates the world itself — so a missed
//! event costs one poll interval, never correctness.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, warn};

use crate::engine::Engine;

/// Routing-socket message types that indicate the network actually changed:
/// interface state (IFINFO), address add/remove (NEWADDR/DELADDR), and route
/// table changes (ADD/DELETE/CHANGE — the default route moving shows up
/// here). RTM_MISS fires on every failed lookup and RTM_GET on queries;
/// both are noise.
const HINT_EVENTS: [&str; 6] = [
    "RTM_IFINFO",
    "RTM_NEWADDR",
    "RTM_DELADDR",
    "RTM_ADD",
    "RTM_DELETE",
    "RTM_CHANGE",
];

/// True if this `route monitor` output line is a reconcile-worthy event.
pub fn is_hint_line(line: &str) -> bool {
    let Some(event) = line.split(':').next() else {
        return false;
    };
    HINT_EVENTS.contains(&event.trim())
}

/// Run forever: spawn `route -n monitor`, forward hint lines to the engine,
/// restart the child (with a delay) if it exits.
pub async fn run(engine: Arc<Engine>) {
    loop {
        match tokio::process::Command::new("/sbin/route")
            .args(["-n", "monitor"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => {
                let mut monitor = Monitor {
                    child: Some(child),
                    activity: Some(crate::mountcmd::CommandActivity::begin()),
                };
                let child = monitor.child.as_mut().unwrap();
                if let Some(stdout) = child.stdout.take() {
                    let mut lines = BufReader::new(stdout).lines();
                    loop {
                        match lines.next_line().await {
                            Ok(Some(line)) => {
                                if is_hint_line(&line) {
                                    debug!(event = %line.trim(), "network event");
                                    engine.hint.notify_one();
                                }
                            }
                            Ok(None) => break, // the child closed stdout
                            // A single undecodable line is not a reason to
                            // tear down the monitor and lose every subsequent
                            // event until the restart delay elapses.
                            Err(e) => {
                                warn!("route monitor read error: {e}");
                                break;
                            }
                        }
                    }
                }
                // EOF or a read failure does not mean the process has exited.
                stop_monitor(child).await;
                warn!("route monitor exited; restarting in 5s");
            }
            Err(e) => {
                warn!("failed to spawn route monitor: {e}; retrying in 5s");
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

struct Monitor {
    child: Option<tokio::process::Child>,
    activity: Option<crate::mountcmd::CommandActivity>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let mut child = self.child.take().unwrap();
        let activity = self.activity.take();
        let _ = child.start_kill();
        tokio::spawn(async move {
            stop_monitor(&mut child).await;
            drop(activity);
        });
    }
}

async fn stop_monitor(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    while child.wait().await.is_err() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_stdout_does_not_prevent_restart() {
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", "exec 1>&-; exec /bin/sleep 30"])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        assert!(tokio::time::timeout(
            Duration::from_secs(2),
            BufReader::new(stdout).lines().next_line()
        )
        .await
        .unwrap()
        .unwrap()
        .is_none());
        tokio::time::timeout(Duration::from_secs(2), stop_monitor(&mut child))
            .await
            .unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn interface_and_address_events_are_hints() {
        assert!(is_hint_line(
            "RTM_IFINFO: iface status change: len 112, if# 12, name: en0, what: <LINK>"
        ));
        assert!(is_hint_line(
            "RTM_NEWADDR: address being added to iface: len 116, metric 0, flags:<UP>"
        ));
        assert!(is_hint_line(
            "RTM_DELADDR: address being removed from iface: len 116"
        ));
        assert!(is_hint_line("RTM_ADD: Add Route: len 176, pid: 0"));
        assert!(is_hint_line("RTM_DELETE: Delete Route: len 176"));
        assert!(is_hint_line("RTM_CHANGE: Change Metrics or flags: len 176"));
    }

    #[test]
    fn lookup_noise_is_not_a_hint() {
        assert!(!is_hint_line(
            "RTM_MISS: Lookup failed on this address: len 120, pid: 0, seq 0"
        ));
        assert!(!is_hint_line("RTM_GET: Report Metrics: len 176"));
        assert!(!is_hint_line(
            "got message of size 120 on Wed Jul 29 20:35:27 2026"
        ));
        assert!(!is_hint_line("sockaddrs: <DST>"));
        assert!(!is_hint_line(" 2001:4860:4860::8888"));
        assert!(!is_hint_line(""));
    }
}
