use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use mountd::{config::Config, engine, ipc, mountcmd, power, watcher};

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

            // Own the singleton before starting any watcher or mount effect.
            let (listener, lock) = ipc::bind_socket(&ipc::socket_path()).await?;
            let _shutdown_lock = lock.try_clone().context("retaining daemon ownership")?;
            let eng = engine::Engine::new(path, cfg);
            let (stop_ipc, shutdown_ipc) = tokio::sync::oneshot::channel();
            let ipc_engine = eng.clone();
            let mut ipc_task = tokio::spawn(async move {
                ipc::serve_bound_until(ipc_engine, listener, lock, async {
                    let _ = shutdown_ipc.await;
                })
                .await
            });
            let watch_task = tokio::spawn(watcher::run(eng.clone()));
            let power_rx = power::watch();
            let mut run_task = tokio::spawn(eng.clone().run(Some(power_rx)));

            let mut ipc_finished = false;
            let mut run_finished = false;
            let mut result = tokio::select! {
                outcome = &mut ipc_task => {
                    ipc_finished = true;
                    match outcome {
                        Ok(Err(error)) => Err(error.context("IPC server exited")),
                        Ok(Ok(())) => Err(anyhow::anyhow!("IPC server stopped unexpectedly")),
                        Err(error) => Err(error.into()),
                    }
                }
                outcome = &mut run_task => {
                    run_finished = true;
                    match outcome {
                        Ok(()) => Err(anyhow::anyhow!("reconciler stopped unexpectedly")),
                        Err(error) => Err(error.into()),
                    }
                }
                _ = shutdown_signal() => {
                    info!("shutting down");
                    Ok(())
                }
            };
            eng.begin_shutdown();
            let _ = stop_ipc.send(());
            if !run_finished {
                run_task.abort();
                let _ = run_task.await;
            }
            watch_task.abort();
            let _ = watch_task.await;
            if !ipc_finished {
                let drained = ipc_task.await.context("joining IPC server").and_then(|r| r);
                if result.is_ok() {
                    result = drained;
                }
            }
            eng.drain().await;
            mountcmd::drain_commands().await;
            // Keep the socket and lock inode. The next locked bind proves the
            // socket stale before replacing it, after all effects have drained.
            result
        }
    }
}
