use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use mountd::{ipc, launchd};

#[derive(Parser)]
#[command(name = "mountctl", version, about = "Control a running mountd daemon")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the state of every configured mount.
    Status {
        /// Emit raw JSON (for SwiftBar plugins etc.)
        #[arg(long)]
        json: bool,
    },
    /// Explain why each mount is or isn't mounted (conditions, endpoint
    /// eligibility, live probe results).
    Explain {
        /// Limit to one mount.
        name: Option<String>,
        /// Emit raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Force an immediate mount attempt (ignores `when`, honors probes).
    Mount { name: String },
    /// Unmount now.
    Unmount { name: String },
    /// Re-read the config file.
    Reload,
    /// Trigger an immediate reconcile pass.
    Poke,
    /// Install and start the LaunchAgent for the mountd binary next to this one.
    Install,
    /// Stop and remove the LaunchAgent.
    Uninstall,
}

async fn send(req: ipc::Request) -> Result<ipc::Response> {
    let resp = ipc::client_request(&ipc::socket_path(), &req).await?;
    if !resp.ok {
        anyhow::bail!(
            "{}",
            resp.error.as_deref().unwrap_or("unknown daemon error")
        );
    }
    Ok(resp)
}

#[tokio::main]
async fn main() -> Result<()> {
    let result = run().await;
    mountd::mountcmd::drain_commands().await;
    result
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Status { json } => {
            let resp = send(ipc::Request::Status).await?;
            let status = resp.status.context("daemon response missing status")?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else if status.is_empty() {
                println!("no mounts configured");
            } else {
                let name_w = status
                    .iter()
                    .map(|s| s.name.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);
                println!("{:name_w$}  {:32}  MOUNTPOINT", "NAME", "STATE");
                for s in &status {
                    println!("{:name_w$}  {:32}  {}", s.name, s.state, s.mountpoint);
                    if let Some(err) = &s.last_error {
                        println!("{:name_w$}  last error: {err}", "");
                    }
                }
            }
        }
        Cmd::Explain { name, json } => {
            let resp = send(ipc::Request::Explain { name }).await?;
            let explanations = resp
                .explanations
                .context("daemon response missing explanations")?;
            if json {
                println!("{}", serde_json::to_string_pretty(&explanations)?);
            } else {
                for ex in &explanations {
                    let mounted = if ex.mounted { "mounted" } else { "not mounted" };
                    println!("{}: {} — decision: {}", ex.name, mounted, ex.decision);
                    for (label, conds) in [("any_of", &ex.when_any_of), ("all_of", &ex.when_all_of)]
                    {
                        if !conds.is_empty() {
                            println!("  when {label}:");
                            for c in conds {
                                println!("    {} {}", if c.holds { "✓" } else { "✗" }, c.cond);
                            }
                        }
                    }
                    if !ex.state.is_empty() {
                        println!("  state: {}", ex.state);
                    }
                    println!("  endpoints (priority order):");
                    for ep in &ex.endpoints {
                        let state = match (&ep.blocked_by, ep.reachable) {
                            (Some(b), _) => format!("skipped ({b} not met)"),
                            (None, Some(true)) => "eligible, probe ok".into(),
                            (None, Some(false)) => "eligible, probe FAILED".into(),
                            (None, None) => "eligible, not probed".into(),
                        };
                        let marker = if ep.current { " <- in use" } else { "" };
                        println!("    {}  {}{}", ep.address, state, marker);
                    }
                }
            }
        }
        Cmd::Mount { name } => {
            send(ipc::Request::Mount { name: name.clone() }).await?;
            println!("mounted {name}");
        }
        Cmd::Unmount { name } => {
            send(ipc::Request::Unmount { name: name.clone() }).await?;
            println!("unmounted {name}");
        }
        Cmd::Reload => {
            send(ipc::Request::Reload).await?;
            println!("config reloaded");
        }
        Cmd::Poke => {
            send(ipc::Request::Poke).await?;
            println!("reconcile triggered");
        }
        Cmd::Install => {
            launchd::install().await?;
            println!("LaunchAgent installed and started ({})", launchd::LABEL);
        }
        Cmd::Uninstall => {
            launchd::uninstall().await?;
            println!("LaunchAgent removed");
        }
    }
    Ok(())
}
