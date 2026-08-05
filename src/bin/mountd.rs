use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use mountd::{config::Config, engine, ipc, power, watcher};

/// launchd stops jobs with SIGTERM, so listening only for SIGINT meant the
/// clean-shutdown path never ran in production and a manual `kill` orphaned
/// the `route monitor` child.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cannot listen for SIGTERM: {e}");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[derive(Parser)]
#[command(
    name = "mountd",
    version,
    about = "Condition-aware network mount daemon"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon in the foreground (launchd runs this).
    Run {
        /// Path to config.toml (default: ~/.config/mountd/config.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Parse and validate the config, then exit.
    Check {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
}

fn load_config(path: &PathBuf) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Config::from_toml_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { config } => {
            let path = config.unwrap_or_else(engine::default_config_path);
            let cfg = load_config(&path)?;
            println!(
                "ok: {} mount(s), {} network(s), {} dep(s)",
                cfg.mounts.len(),
                cfg.networks.len(),
                cfg.deps.len()
            );
            Ok(())
        }
        Cmd::Run { config } => {
            let path = config.unwrap_or_else(engine::default_config_path);

            // A broken config must not exit: launchd would restart us every
            // ten seconds forever. Idle instead with an empty config and pick
            // up a fixed one on the next `mountctl reload`.
            let cfg = match load_config(&path) {
                Ok(cfg) => {
                    info!("mountd starting with {} mount(s)", cfg.mounts.len());
                    cfg
                }
                Err(e) => {
                    tracing::error!(
                        "{e:#}\nstarting with no mounts; fix the config and run \
                         `mountctl reload`"
                    );
                    Config::default()
                }
            };

            let sleep_budget = Duration::from_secs(cfg.defaults.sleep_unmount_budget_secs);
            let eng = engine::Engine::new(path, cfg);

            let ipc_engine = eng.clone();
            tokio::spawn(async move {
                if let Err(e) = ipc::serve(ipc_engine, ipc::socket_path()).await {
                    tracing::error!("ipc server exited: {e:#}");
                }
            });

            let watch_engine = eng.clone();
            tokio::spawn(watcher::run(watch_engine));

            let power_rx = power::watch(sleep_budget);

            let run_engine = eng.clone();
            tokio::select! {
                _ = run_engine.run(Some(power_rx)) => {}
                _ = shutdown_signal() => {
                    info!("shutting down");
                }
            }
            // The socket is replaced on the next start anyway, but leaving a
            // live-looking socket behind makes `mountctl` hang on connect
            // instead of saying the daemon is not running.
            let _ = std::fs::remove_file(ipc::socket_path());
            Ok(())
        }
    }
}
