//! The daemon engine: periodically (and on demand) gathers a network
//! snapshot, evaluates conditions and deps, and reconciles each configured
//! mount toward its desired state. Events are hints; only a fresh successful
//! TCP probe of a chosen endpoint permits a mount attempt.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{info, warn};

use crate::config::Config;
use crate::reconcile::{self, Decision, EvalCtx};
use crate::{deps, fingerprint, mountcmd, mtab};

/// Default config path: `~/.config/mountd/config.toml`.
pub fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/mountd/config.toml")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MountStatus {
    pub name: String,
    pub state: String,
    pub mountpoint: String,
    #[serde(default)]
    pub mounted_via: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct MountRuntime {
    mounted_via: Option<String>,
    state: String,
    last_error: Option<String>,
    backoff: Option<Duration>,
    next_retry_at: Option<Instant>,
}

pub struct Engine {
    config_path: PathBuf,
    cfg: RwLock<Config>,
    runtime: Mutex<BTreeMap<String, MountRuntime>>,
    /// Poke to trigger an immediate (debounced) reconcile.
    pub hint: Notify,
    last_snapshot: Mutex<Option<fingerprint::NetSnapshot>>,
}

impl Engine {
    pub fn new(config_path: PathBuf, cfg: Config) -> Arc<Self> {
        Arc::new(Engine {
            config_path,
            cfg: RwLock::new(cfg),
            runtime: Mutex::new(BTreeMap::new()),
            hint: Notify::new(),
            last_snapshot: Mutex::new(None),
        })
    }

    pub async fn reload(&self) -> Result<()> {
        let text = tokio::fs::read_to_string(&self.config_path)
            .await
            .with_context(|| format!("reading {}", self.config_path.display()))?;
        let new = Config::from_toml_str(&text)?;
        *self.cfg.write().await = new;
        info!("config reloaded from {}", self.config_path.display());
        self.hint.notify_one();
        Ok(())
    }

    pub async fn status(&self) -> Vec<MountStatus> {
        let cfg = self.cfg.read().await;
        let runtime = self.runtime.lock().await;
        cfg.mounts
            .iter()
            .map(|m| {
                let rt = runtime.get(&m.name).cloned().unwrap_or_default();
                MountStatus {
                    name: m.name.clone(),
                    state: if rt.state.is_empty() {
                        "unknown".into()
                    } else {
                        rt.state
                    },
                    mountpoint: m.mountpoint.clone(),
                    mounted_via: rt.mounted_via,
                    last_error: rt.last_error,
                }
            })
            .collect()
    }

    /// Force an immediate attempt for one mount, ignoring `when` conditions
    /// (but still honoring endpoint eligibility and probe gates).
    pub async fn force_mount(&self, name: &str) -> Result<()> {
        let cfg = self.cfg.read().await;
        let spec = cfg
            .mount(name)
            .with_context(|| format!("no mount named `{name}`"))?
            .clone();
        let defaults = cfg.defaults.clone();
        drop(cfg);

        let ctx = self.eval_ctx().await;
        let cands = reconcile::candidates(&spec, &ctx, defaults.probe_timeout_ms);
        if cands.is_empty() {
            anyhow::bail!("no eligible endpoint (check `only_if`/`requires` conditions)");
        }
        let chosen = crate::probe::select_endpoint(&cands, Duration::from_millis(250))
            .await
            .context("no endpoint passed its reachability probe")?;
        let address = spec.endpoints[chosen].address.clone();
        mountcmd::mount(&spec, &address).await?;
        self.note_mounted(&spec.name, &address).await;
        Ok(())
    }

    pub async fn force_unmount(&self, name: &str) -> Result<()> {
        let cfg = self.cfg.read().await;
        let spec = cfg
            .mount(name)
            .with_context(|| format!("no mount named `{name}`"))?
            .clone();
        drop(cfg);
        mountcmd::unmount(&spec.mountpoint).await?;
        let mut rt = self.runtime.lock().await;
        let entry = rt.entry(name.to_string()).or_default();
        entry.mounted_via = None;
        entry.state = "unmounted".into();
        Ok(())
    }

    async fn note_mounted(&self, name: &str, address: &str) {
        let mut rt = self.runtime.lock().await;
        let entry = rt.entry(name.to_string()).or_default();
        entry.mounted_via = Some(address.to_string());
        entry.state = format!("mounted via {address}");
        entry.last_error = None;
        entry.backoff = None;
        entry.next_retry_at = None;
    }

    async fn eval_ctx(&self) -> EvalCtx {
        let cfg = self.cfg.read().await;
        let snapshot = fingerprint::gather().await;
        let matched_networks = snapshot.matched_networks(&cfg);
        let deps_up = deps::check_all(&cfg).await;
        *self.last_snapshot.lock().await = Some(snapshot);
        EvalCtx {
            matched_networks,
            deps_up,
        }
    }

    /// One full reconcile pass over every configured mount.
    pub async fn reconcile_once(&self) {
        let cfg = self.cfg.read().await.clone();
        let ctx = self.eval_ctx().await;
        let entries = mtab::list();

        for spec in &cfg.mounts {
            let mp = mountcmd::expand_tilde(&spec.mountpoint);
            let mounted =
                mtab::find_by_mountpoint(&entries, &mp.to_string_lossy()).is_some();

            let decision = reconcile::decide(spec, &ctx, mounted);
            match decision {
                Decision::Keep => {
                    let mut rt = self.runtime.lock().await;
                    let entry = rt.entry(spec.name.clone()).or_default();
                    if entry.mounted_via.is_none() {
                        // mounted outside our control (e.g. Finder); adopt it
                        entry.state = "mounted (adopted)".into();
                    }
                }
                Decision::Idle => {
                    let mut rt = self.runtime.lock().await;
                    let entry = rt.entry(spec.name.clone()).or_default();
                    entry.mounted_via = None;
                    entry.state = "idle (conditions not met)".into();
                }
                Decision::Waiting => {
                    let mut rt = self.runtime.lock().await;
                    let entry = rt.entry(spec.name.clone()).or_default();
                    entry.state = "waiting (no eligible endpoint)".into();
                }
                Decision::Unmount => {
                    info!(mount = %spec.name, "conditions gone, unmounting");
                    match mountcmd::unmount(&spec.mountpoint).await {
                        Ok(()) => {
                            let mut rt = self.runtime.lock().await;
                            let entry = rt.entry(spec.name.clone()).or_default();
                            entry.mounted_via = None;
                            entry.state = "unmounted (conditions gone)".into();
                            entry.last_error = None;
                        }
                        Err(e) => {
                            warn!(mount = %spec.name, error = %e, "unmount failed");
                            let mut rt = self.runtime.lock().await;
                            let entry = rt.entry(spec.name.clone()).or_default();
                            entry.last_error = Some(e.to_string());
                        }
                    }
                }
                Decision::Attempt => {
                    // honor retry backoff
                    {
                        let rt = self.runtime.lock().await;
                        if let Some(entry) = rt.get(&spec.name) {
                            if let Some(at) = entry.next_retry_at {
                                if Instant::now() < at {
                                    continue;
                                }
                            }
                        }
                    }
                    let cands =
                        reconcile::candidates(spec, &ctx, cfg.defaults.probe_timeout_ms);
                    let chosen =
                        crate::probe::select_endpoint(&cands, Duration::from_millis(250)).await;
                    match chosen {
                        None => {
                            // gate held: no endpoint reachable; not an error
                            let mut rt = self.runtime.lock().await;
                            let entry = rt.entry(spec.name.clone()).or_default();
                            entry.state = "waiting (no endpoint reachable)".into();
                        }
                        Some(idx) => {
                            let address = spec.endpoints[idx].address.clone();
                            info!(mount = %spec.name, %address, "probe passed, mounting");
                            match mountcmd::mount(spec, &address).await {
                                Ok(()) => {
                                    info!(mount = %spec.name, %address, "mounted");
                                    self.note_mounted(&spec.name, &address).await;
                                }
                                Err(e) => {
                                    warn!(mount = %spec.name, error = %e, "mount failed");
                                    let mut rt = self.runtime.lock().await;
                                    let entry = rt.entry(spec.name.clone()).or_default();
                                    let backoff =
                                        reconcile::next_backoff(entry.backoff, &spec.policy.retry);
                                    entry.backoff = Some(backoff);
                                    entry.next_retry_at = Some(Instant::now() + backoff);
                                    entry.last_error = Some(e.to_string());
                                    entry.state =
                                        format!("error (retry in {}s)", backoff.as_secs());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Main loop: reconcile on a poll interval, and sooner when hinted
    /// (debounced so event storms coalesce into one pass).
    pub async fn run(self: Arc<Self>) {
        loop {
            self.reconcile_once().await;
            let (poll, debounce) = {
                let cfg = self.cfg.read().await;
                (
                    Duration::from_secs(cfg.defaults.poll_interval_secs.max(1)),
                    Duration::from_secs(cfg.defaults.debounce_secs),
                )
            };
            tokio::select! {
                _ = tokio::time::sleep(poll) => {}
                _ = self.hint.notified() => {
                    // debounce: absorb the burst, then reconcile once
                    tokio::time::sleep(debounce).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with(toml: &str) -> Arc<Engine> {
        let cfg = Config::from_toml_str(toml).unwrap();
        Engine::new(PathBuf::from("/nonexistent/config.toml"), cfg)
    }

    #[tokio::test]
    async fn status_reports_all_configured_mounts() {
        let eng = engine_with(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/a"
  [[mount.endpoint]]
  address = "203.0.113.9"

[[mount]]
name = "b"
protocol = "nfs"
export = "/e"
mountpoint = "~/mnt/b"
  [[mount.endpoint]]
  address = "203.0.113.10"
"#,
        );
        let st = eng.status().await;
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].name, "a");
        assert_eq!(st[0].state, "unknown"); // no reconcile yet
    }

    #[tokio::test]
    async fn reconcile_marks_unreachable_mount_waiting() {
        // TEST-NET-1 endpoint is unreachable -> probe gate holds -> waiting,
        // and crucially no mount was attempted (state, not error).
        let eng = engine_with(
            r#"
[defaults]
probe_timeout_ms = 100

[[mount]]
name = "x"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/mountd-test-nonexistent"
  [[mount.endpoint]]
  address = "192.0.2.1"
"#,
        );
        eng.reconcile_once().await;
        let st = eng.status().await;
        assert_eq!(st[0].state, "waiting (no endpoint reachable)");
        assert!(st[0].last_error.is_none());
    }

    #[tokio::test]
    async fn reconcile_leaves_condition_gated_mount_idle() {
        let eng = engine_with(
            r#"
[networks.nowhere]
subnet = "203.0.113.0/24"

[[mount]]
name = "x"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/mountd-test-nonexistent"
  [mount.when]
  all_of = ["network:nowhere"]
  [[mount.endpoint]]
  address = "192.0.2.1"
"#,
        );
        eng.reconcile_once().await;
        let st = eng.status().await;
        assert_eq!(st[0].state, "idle (conditions not met)");
    }

    #[tokio::test]
    async fn force_mount_unknown_name_errors() {
        let eng = engine_with(
            r#"
[[mount]]
name = "x"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/x"
  [[mount.endpoint]]
  address = "192.0.2.1"
"#,
        );
        assert!(eng.force_mount("nope").await.is_err());
    }
}
