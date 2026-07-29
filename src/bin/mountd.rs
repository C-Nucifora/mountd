use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use mountd::{config::Config, engine, ipc};

#[derive(Parser)]
#[command(name = "mountd", version, about = "Condition-aware network mount daemon")]
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
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    Config::from_toml_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
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
            let cfg = load_config(&path)?;
            info!("mountd starting with {} mount(s)", cfg.mounts.len());
            let eng = engine::Engine::new(path, cfg);

            let ipc_engine = eng.clone();
            tokio::spawn(async move {
                if let Err(e) = ipc::serve(ipc_engine, ipc::socket_path()).await {
                    tracing::error!("ipc server exited: {e:#}");
                }
            });

            let run_engine = eng.clone();
            tokio::select! {
                _ = run_engine.run() => {}
                _ = tokio::signal::ctrl_c() => {
                    info!("shutting down");
                }
            }
            Ok(())
        }
    }
}
