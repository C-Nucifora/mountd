//! The daemon engine: periodically (and on demand) gathers a network
//! snapshot, evaluates conditions and deps, and reconciles each configured
//! mount toward its desired state. Events are hints; only a fresh successful
//! TCP probe of a chosen endpoint permits a mount attempt.
//!
//! Two properties matter more than anything else here, both learned the hard
//! way from v0.3 running in production:
//!
//! 1. **Destroying a mount requires sustained evidence.** A single bad
//!    condition sample — a slow `tailscale status`, an ARP cache that has not
//!    repopulated 150 ms after wake — must never cost a healthy mount. Every
//!    unmount is gated on a grace period, and no policy unmount ever forces.
//! 2. **No mount may block another.** Each mount reconciles in its own task,
//!    so a share whose server is wedged cannot stall its peers.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Notify, OwnedMutexGuard, RwLock};
use tracing::{info, warn};

use crate::config::{Config, Defaults, MountSpec, Protocol};
use crate::cred::FetchOutcome;
use crate::mountcmd::OperationLease;
use crate::mtab::MountEntry;
use crate::ops::{Ops, SystemOps};
use crate::reconcile::{self, Decision, EvalCtx, MountView};
use crate::state::{MountRuntime, MountState};
use crate::{fingerprint, mountcmd, power};

/// Head start given to a higher-priority endpoint before the next one is
/// probed, so the preferred path wins ties without waiting on the slowest.
const PROBE_STAGGER: Duration = Duration::from_millis(250);
/// Generous: covers the one-time macOS ACL prompt when the Keychain item was
/// created by another app.
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(90);

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
    /// Machine-readable discriminant (`mounted`, `suspect`, `error`, …).
    /// Additive: older clients keep matching on `state`'s prefix.
    #[serde(default)]
    pub state_kind: String,
}

pub struct Engine {
    lifecycle: Arc<RwLock<()>>,
    sleeping: AtomicBool,
    stopping: AtomicBool,
    config_path: PathBuf,
    cfg: RwLock<Config>,
    runtime: Mutex<BTreeMap<String, MountRuntime>>,
    /// Poke to trigger an immediate (debounced) reconcile.
    pub hint: Notify,
    ops: Arc<dyn Ops>,
    /// Until this instant, condition readings are treated as untrustworthy
    /// because the system just woke and the network is still coming up.
    settle_until: Mutex<Option<Instant>>,
    last_snapshot: Mutex<Option<fingerprint::NetSnapshot>>,
}

impl Engine {
    pub fn new(config_path: PathBuf, cfg: Config) -> Arc<Self> {
        Engine::with_ops(config_path, cfg, Arc::new(SystemOps))
    }

    pub fn with_ops(config_path: PathBuf, cfg: Config, ops: Arc<dyn Ops>) -> Arc<Self> {
        Arc::new(Engine {
            lifecycle: Arc::new(RwLock::new(())),
            sleeping: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            config_path,
            cfg: RwLock::new(cfg),
            runtime: Mutex::new(BTreeMap::new()),
            hint: Notify::new(),
            ops,
            settle_until: Mutex::new(None),
            last_snapshot: Mutex::new(None),
        })
    }

    /// Prevent new effects before the accept loop and reconciliation are stopped.
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.sleeping.store(true, Ordering::SeqCst);
        self.hint.notify_one();
    }

    /// Keep the daemon singleton held until owned processes have been reaped.
    pub async fn drain(&self) {
        let _lifecycle = self.lifecycle.write().await;
    }

    pub async fn reload(&self) -> Result<()> {
        let _lifecycle = self.lifecycle.write().await;
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "daemon is stopping");
        let text = tokio::fs::read_to_string(&self.config_path)
            .await
            .with_context(|| format!("reading {}", self.config_path.display()))?;
        let new = Config::from_toml_str(&text)?;
        let mut cfg = self.cfg.write().await;
        let mut runtime = self.runtime.lock().await;
        *cfg = new;
        // Publish config and reset its state together, without a cancellation point.
        runtime.clear();
        drop(runtime);
        drop(cfg);

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
                    state: rt.state.to_string(),
                    state_kind: rt.state.kind().to_string(),
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
        let lifecycle = self.lifecycle.clone().read_owned().await;
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "daemon is stopping");
        anyhow::ensure!(!self.sleeping.load(Ordering::SeqCst), "system is sleeping");
        let cfg = self.cfg.read().await;
        let spec = cfg
            .mount(name)
            .with_context(|| format!("no mount named `{name}`"))?
            .clone();
        let defaults = cfg.defaults.clone();
        drop(cfg);
        let claim = self
            .claim(name)
            .await
            .context("mount operation already in flight")?;
        let lease: OperationLease = Arc::new((claim, lifecycle));

        // Accepting a new mount request ends the earlier manual unmount intent.
        self.runtime
            .lock()
            .await
            .entry(name.to_string())
            .or_default()
            .user_suppressed = false;

        let mountpoint = self.mountpoint_of(&spec);
        if crate::mtab::find_by_mountpoint(&self.ops.list_mounts()?, &mountpoint).is_some() {
            // Nothing to do, and attempting anyway would stack a second mount
            // or fail confusingly after a pointless Keychain read.
            return Ok(());
        }

        self.force_mount_inner(&spec, &defaults, &lease).await
    }

    async fn force_mount_inner(
        &self,
        spec: &MountSpec,
        defaults: &Defaults,
        lease: &OperationLease,
    ) -> Result<()> {
        let ctx = self.eval_ctx().await;
        let cands = reconcile::candidates(spec, &ctx, defaults.probe_timeout_ms);
        if cands.is_empty() {
            anyhow::bail!("no eligible endpoint (check `only_if`/`requires` conditions)");
        }
        let picked = crate::probe::select_endpoint(&cands, PROBE_STAGGER, self.probe_fn())
            .await
            .context("no endpoint passed its reachability probe")?;
        let address = spec.endpoints[picked].address.clone();
        self.attempt_mount(spec, &address, defaults, true, lease)
            .await?;

        let mut rt = self.runtime.lock().await;
        rt.entry(spec.name.clone())
            .or_default()
            .note_mounted(&address);
        Ok(())
    }

    pub async fn force_unmount(&self, name: &str) -> Result<()> {
        let lifecycle = self.lifecycle.clone().read_owned().await;
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "daemon is stopping");
        let cfg = self.cfg.read().await;
        let spec = cfg
            .mount(name)
            .with_context(|| format!("no mount named `{name}`"))?
            .clone();
        drop(cfg);
        let claim = self
            .claim(name)
            .await
            .context("mount operation already in flight")?;
        let lease: OperationLease = Arc::new((claim, lifecycle));

        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "daemon is stopping");
        // A user asking for an unmount may force: they are present and can
        // see what breaks, unlike the reconciler acting on a stale sample.
        self.ops
            .unmount(self.mountpoint_of(&spec), true, lease.clone())
            .await?;

        let when_held = self.eval_ctx().await.when_holds(&spec.when);
        let mut rt = self.runtime.lock().await;
        let entry = rt.entry(name.to_string()).or_default();
        entry.mounted_via = None;
        entry.health_failures = 0;
        // Remember the user's intent: without this the next pass, up to a poll
        // interval away, simply mounts it straight back.
        entry.user_suppressed = true;
        entry.when_held = when_held;
        entry.state = MountState::Unmounted { manual: true };
        Ok(())
    }

    fn mountpoint_of(&self, spec: &MountSpec) -> String {
        mountcmd::normalize_mountpoint(&spec.mountpoint)
            .to_string_lossy()
            .to_string()
    }

    /// A probe closure bound to this engine's `Ops`, for `select_endpoint`.
    fn probe_fn(&self) -> impl Fn(String, u16, Duration) -> crate::BoxFut<bool> + Clone + 'static {
        let ops = self.ops.clone();
        move |address, port, timeout| ops.probe(address, port, timeout)
    }

    /// Claim the right to act on a mount. Returns None when another task is
    /// already mid-attempt for it.
    async fn claim(&self, name: &str) -> Option<OwnedMutexGuard<()>> {
        let lock = self
            .runtime
            .lock()
            .await
            .entry(name.to_string())
            .or_default()
            .operation
            .clone();
        lock.try_lock_owned().ok()
    }

    async fn eval_ctx(&self) -> EvalCtx {
        // Clone the config rather than holding the read lock across these
        // awaits: tokio's RwLock is write-preferring, so one queued reload
        // would otherwise starve every concurrent `status`/`explain` for as
        // long as a wedged dep check takes.
        let cfg = self.cfg.read().await.clone();

        let deps_up = self.ops.check_deps(cfg.clone()).await;
        let snapshot = self.ops.network_snapshot().await;
        let matched_networks = snapshot.matched_networks(&cfg);

        // A changed snapshot is itself a reason to reconcile sooner.
        let mut last = self.last_snapshot.lock().await;
        if last.as_ref().is_some_and(|prev| *prev != snapshot) {
            for entry in self.runtime.lock().await.values_mut() {
                entry.clear_backoff();
            }
            self.hint.notify_one();
        }
        *last = Some(snapshot);

        EvalCtx {
            matched_networks,
            deps_up,
        }
    }

    async fn is_settling(&self) -> bool {
        let settle = self.settle_until.lock().await;
        settle.is_some_and(|until| Instant::now() < until)
    }

    /// Called on every wake, from whichever detector noticed first.
    ///
    /// Two things are true right after wake: the network is not up yet, so
    /// conditions read false; and every monotonic deadline is stale, because
    /// macOS pauses the monotonic clock while asleep — a retry scheduled
    /// before sleep would otherwise hold off a remount for its full backoff
    /// of *awake* time.
    pub async fn note_wake(&self, settle: Duration) {
        let _lifecycle = self.lifecycle.write().await;
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        self.sleeping.store(false, Ordering::SeqCst);
        *self.settle_until.lock().await = Instant::now().checked_add(settle);
        let mut rt = self.runtime.lock().await;
        for entry in rt.values_mut() {
            entry.clear_backoff();
            entry.gone_since = None;
            entry.last_health_check = None;
            entry.health_failures = 0;
            entry.migrate_target = None;
            entry.migrate_streak = 0;
            entry.last_wol_at = None;
        }
        drop(rt);
        self.hint.notify_one();
    }

    /// One full reconcile pass over every configured mount, all concurrently.
    pub async fn reconcile_once(self: &Arc<Self>) {
        let lifecycle = Arc::new(self.lifecycle.clone().read_owned().await);
        if self.sleeping.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let cfg = self.cfg.read().await.clone();
        let ctx = Arc::new(self.eval_ctx().await);
        let defaults = Arc::new(cfg.defaults.clone());
        let settling = self.is_settling().await;

        let mut tasks = tokio::task::JoinSet::new();
        for spec in cfg.mounts {
            let lifecycle = lifecycle.clone();
            let engine = self.clone();
            let ctx = ctx.clone();
            let defaults = defaults.clone();
            tasks.spawn(async move {
                engine
                    .reconcile_mount(spec, ctx, defaults, settling, lifecycle)
                    .await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(e) = result {
                warn!("reconcile task failed: {e}");
            }
        }
    }

    async fn reconcile_mount(
        self: Arc<Self>,
        spec: MountSpec,
        ctx: Arc<EvalCtx>,
        defaults: Arc<Defaults>,
        settling: bool,
        lifecycle: Arc<tokio::sync::OwnedRwLockReadGuard<()>>,
    ) {
        let Some(claim) = self.claim(&spec.name).await else {
            return;
        };
        let lease: OperationLease = Arc::new((claim, lifecycle));
        if self.sleeping.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let entries = match self.ops.list_mounts() {
            Ok(entries) => entries,
            Err(error) => {
                let mut rt = self.runtime.lock().await;
                let entry = rt.entry(spec.name.clone()).or_default();
                entry.last_error = Some(format!("cannot enumerate mounts: {error}"));
                entry.state = MountState::Error { retry_secs: 0 };
                return;
            }
        };
        self.reconcile_mount_inner(&spec, &ctx, &defaults, &entries, settling, &lease)
            .await;
    }

    async fn reconcile_mount_inner(
        &self,
        spec: &MountSpec,
        ctx: &EvalCtx,
        defaults: &Defaults,
        entries: &[MountEntry],
        settling: bool,
        lease: &OperationLease,
    ) {
        let mountpoint = self.mountpoint_of(spec);
        let entry_now = crate::mtab::find_by_mountpoint(entries, &mountpoint);
        let mounted = entry_now.is_some();
        let should = ctx.when_holds(&spec.when);
        let world = ctx.fingerprint();

        // --- bookkeeping that must happen before any decision ---
        let (mounted_via, retry_due) = {
            let mut rt = self.runtime.lock().await;
            let e = rt.entry(spec.name.clone()).or_default();

            // A false -> true edge on `when` means the world moved; an earlier
            // manual unmount and any backoff no longer describe the situation.
            if should && !e.when_held {
                e.user_suppressed = false;
                e.clear_backoff();
            }
            e.when_held = should;

            if should {
                e.gone_since = None;
            } else if e.gone_since.is_none() {
                e.gone_since = Some(Instant::now());
            }

            // The backoff was earned under a different world; discard it.
            if e.backoff_key.is_some_and(|k| k != world) {
                e.clear_backoff();
            }

            if !mounted {
                e.health_failures = 0;
                if !matches!(e.state, MountState::Unmounted { manual: true }) {
                    e.mounted_via = None;
                }
            } else {
                let via = entry_now.and_then(|m| crate::mtab::source_address(&m.from));
                if e.mounted_via != via {
                    e.health_failures = 0;
                    e.last_health_check = None;
                    e.migrate_target = None;
                    e.migrate_streak = 0;
                    e.mounted_via = via;
                }
            }

            let retry_due = e.next_retry_at.is_none_or(|at| Instant::now() >= at);
            (e.mounted_via.clone(), retry_due)
        };

        // --- liveness probe of the endpoint we are actually mounted via ---
        if mounted && should && !settling {
            self.health_check(spec, defaults, mounted_via.as_deref())
                .await;
        }

        // --- is a higher-priority endpoint available? ---
        let migrate_to = if mounted && should && !settling && spec.policy.migrate {
            self.migration_target(spec, ctx, defaults, mounted_via.as_deref())
                .await
        } else {
            self.reset_migration(spec).await;
            None
        };

        let view = {
            let rt = self.runtime.lock().await;
            let e = rt.get(&spec.name).cloned().unwrap_or_default();
            MountView {
                mounted,
                user_suppressed: e.user_suppressed,
                gone_for: e.gone_since.map(|since| since.elapsed()),
                settling,
                health_failures: e.health_failures,
                migrate_to,
                retry_due,
            }
        };

        if self.sleeping.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        match reconcile::decide(spec, ctx, &view) {
            Decision::Keep => self.set_kept_state(spec, mounted_via).await,
            Decision::Idle => self.set_idle_state(spec, mounted, mounted_via).await,
            Decision::Waiting => {
                self.set_state(
                    spec,
                    MountState::Waiting {
                        reason: "no eligible endpoint".into(),
                    },
                )
                .await
            }
            Decision::Suppressed => {
                self.set_state(spec, MountState::Unmounted { manual: true })
                    .await
            }
            Decision::Backoff => {} // leave the error state and its countdown
            Decision::Unmount { force } => self.do_policy_unmount(spec, force, lease).await,
            Decision::Recover => {
                self.do_recover(spec, ctx, defaults, mounted_via, lease)
                    .await
            }
            Decision::Migrate(target) => {
                self.do_migrate(spec, target, mounted_via, defaults, lease)
                    .await;
            }
            Decision::Attempt => self.do_attempt(spec, ctx, defaults, world, lease).await,
        }
    }

    /// Re-probe the endpoint a mounted share is realized via. `getfsstat` can
    /// never tell us a mount has gone dead — it deliberately does not talk to
    /// the server — so this is the only thing standing between a vanished NAS
    /// and a mountpoint that hangs everything touching it.
    async fn health_check(&self, spec: &MountSpec, defaults: &Defaults, via: Option<&str>) {
        let Some(address) = via else { return };

        let interval = Duration::from_secs(defaults.health_interval_secs);
        let due = {
            let rt = self.runtime.lock().await;
            rt.get(&spec.name)
                .and_then(|e| e.last_health_check)
                .is_none_or(|last| last.elapsed() >= interval)
        };
        if !due {
            return;
        }

        let crate::config::Probe::Tcp(port) = spec.effective_probe();
        let timeout = Duration::from_millis(defaults.probe_timeout_ms);
        let up = self.ops.probe(address.to_string(), port, timeout).await;

        let mut rt = self.runtime.lock().await;
        let e = rt.entry(spec.name.clone()).or_default();
        e.last_health_check = Some(Instant::now());
        if up {
            e.health_failures = 0;
        } else {
            e.health_failures = e.health_failures.saturating_add(1);
            let failures = e.health_failures;
            if failures < spec.policy.health_failures {
                e.state = MountState::Suspect {
                    via: address.to_string(),
                    failures,
                };
            }
            warn!(
                mount = %spec.name, %address, failures,
                "mounted endpoint failed its liveness probe"
            );
        }
    }

    /// The highest-priority endpoint that outranks the current one and is
    /// reachable, once it has proved itself for `migrate_confirmations`
    /// consecutive checks — otherwise a marginal LAN link would flap the mount
    /// back and forth.
    async fn migration_target(
        &self,
        spec: &MountSpec,
        ctx: &EvalCtx,
        defaults: &Defaults,
        via: Option<&str>,
    ) -> Option<usize> {
        let current = via?;
        let current_rank = spec.endpoints.iter().position(|e| e.address == current)?;
        if current_rank == 0 {
            return None; // already on the preferred endpoint
        }

        let better: Vec<_> = reconcile::candidates(spec, ctx, defaults.probe_timeout_ms)
            .into_iter()
            .filter(|c| c.index < current_rank)
            .collect();
        if better.is_empty() {
            self.reset_migration(spec).await;
            return None;
        }

        let winner = crate::probe::select_endpoint(&better, PROBE_STAGGER, self.probe_fn()).await;

        let mut rt = self.runtime.lock().await;
        let e = rt.entry(spec.name.clone()).or_default();
        match winner {
            Some(idx) if e.migrate_target == Some(idx) => {
                e.migrate_streak = e.migrate_streak.saturating_add(1);
            }
            Some(idx) => {
                e.migrate_target = Some(idx);
                e.migrate_streak = 1;
            }
            None => {
                e.migrate_target = None;
                e.migrate_streak = 0;
            }
        }
        if e.migrate_streak >= spec.policy.migrate_confirmations {
            e.migrate_target
        } else {
            None
        }
    }

    async fn reset_migration(&self, spec: &MountSpec) {
        let mut rt = self.runtime.lock().await;
        let e = rt.entry(spec.name.clone()).or_default();
        e.migrate_target = None;
        e.migrate_streak = 0;
    }

    async fn set_state(&self, spec: &MountSpec, state: MountState) {
        let mut rt = self.runtime.lock().await;
        rt.entry(spec.name.clone()).or_default().state = state;
    }

    async fn set_kept_state(&self, spec: &MountSpec, via: Option<String>) {
        let mut rt = self.runtime.lock().await;
        let e = rt.entry(spec.name.clone()).or_default();
        // Preserve an in-progress suspicion; `Keep` here only means "no
        // decision to act yet", not "healthy".
        if e.health_failures > 0 {
            if let Some(address) = via.clone() {
                e.state = MountState::Suspect {
                    via: address,
                    failures: e.health_failures,
                };
                return;
            }
        }
        e.state = match via {
            Some(address) => MountState::Mounted { via: address },
            None => MountState::Adopted { via: None },
        };
        // A mount that came back on its own clears the last failure.
        e.last_error = None;
        e.clear_backoff();
    }

    /// `Idle` covers two very different situations: nothing is mounted and
    /// nothing should be, or a pinned mount is deliberately outliving its
    /// conditions. The second must not be reported as unmounted, and must keep
    /// its endpoint attribution — the migration and health paths need it.
    async fn set_idle_state(&self, spec: &MountSpec, mounted: bool, via: Option<String>) {
        let mut rt = self.runtime.lock().await;
        let e = rt.entry(spec.name.clone()).or_default();
        if mounted {
            e.state = MountState::MountedPinned { via: via.clone() };
            e.mounted_via = via;
        } else {
            e.state = MountState::Idle;
            e.mounted_via = None;
        }
    }

    async fn do_policy_unmount(&self, spec: &MountSpec, force: bool, lease: &OperationLease) {
        info!(mount = %spec.name, force, "conditions gone, unmounting");
        let mountpoint = self.mountpoint_of(spec);
        match self.ops.unmount(mountpoint, force, lease.clone()).await {
            Ok(()) => {
                let mut rt = self.runtime.lock().await;
                let e = rt.entry(spec.name.clone()).or_default();
                e.mounted_via = None;
                e.gone_since = None;
                e.health_failures = 0;
                e.state = MountState::UnmountedConditionsGone;
                e.last_error = None;
            }
            Err(e) => {
                // Busy is the common case and is not an error worth escalating:
                // the conditions are gone, so the share is unreachable anyway,
                // and the next pass will try again.
                warn!(mount = %spec.name, error = %e, "unmount failed; will retry");
                let mut rt = self.runtime.lock().await;
                rt.entry(spec.name.clone()).or_default().last_error = Some(e.to_string());
            }
        }
    }

    /// Tear down a mount whose server has stopped answering, then remount it
    /// in the same pass if any endpoint is reachable.
    async fn do_recover(
        &self,
        spec: &MountSpec,
        ctx: &EvalCtx,
        defaults: &Defaults,
        via: Option<String>,
        lease: &OperationLease,
    ) {
        let address = via.clone().unwrap_or_else(|| "unknown".into());
        warn!(mount = %spec.name, %address, "endpoint is dead; recovering the mount");
        self.set_state(spec, MountState::Recovering { via: address })
            .await;

        // Forcing is right here and only here: the probe has failed repeatedly,
        // so the server is gone and a polite unmount would just hang.
        let mountpoint = self.mountpoint_of(spec);
        if self.stopping.load(Ordering::SeqCst) || self.sleeping.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.ops.unmount(mountpoint, true, lease.clone()).await {
            warn!(mount = %spec.name, error = %e, "could not unmount the dead share");
            let mut rt = self.runtime.lock().await;
            rt.entry(spec.name.clone()).or_default().last_error = Some(e.to_string());
            return;
        }

        {
            let mut rt = self.runtime.lock().await;
            let e = rt.entry(spec.name.clone()).or_default();
            e.health_failures = 0;
            e.mounted_via = None;
        }

        self.do_attempt(spec, ctx, defaults, ctx.fingerprint(), lease)
            .await;
    }

    /// Move an existing mount to a higher-priority endpoint. If the polite
    /// unmount fails the share is in use, so the migration is abandoned rather
    /// than forced — a faster path is never worth interrupting a running copy.
    async fn do_migrate(
        &self,
        spec: &MountSpec,
        target: usize,
        via: Option<String>,
        defaults: &Defaults,
        lease: &OperationLease,
    ) {
        let Some(endpoint) = spec.endpoints.get(target) else {
            return;
        };
        let from = via.unwrap_or_else(|| "unknown".into());
        let to = endpoint.address.clone();
        info!(mount = %spec.name, %from, %to, "migrating to a higher-priority endpoint");
        self.set_state(
            spec,
            MountState::Migrating {
                from: from.clone(),
                to: to.clone(),
            },
        )
        .await;

        let mountpoint = self.mountpoint_of(spec);
        if self.stopping.load(Ordering::SeqCst) || self.sleeping.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.ops.unmount(mountpoint, false, lease.clone()).await {
            info!(mount = %spec.name, error = %e, "share is busy; staying put");
            self.reset_migration(spec).await;
            self.set_kept_state(spec, Some(from)).await;
            return;
        }

        match self.attempt_mount(spec, &to, defaults, false, lease).await {
            Ok(()) => {
                let mut rt = self.runtime.lock().await;
                rt.entry(spec.name.clone()).or_default().note_mounted(&to);
                info!(mount = %spec.name, %to, "migrated");
            }
            Err(e) => {
                warn!(mount = %spec.name, error = %e, "migration failed; the mount is now down");
                let mut rt = self.runtime.lock().await;
                let entry = rt.entry(spec.name.clone()).or_default();
                entry.last_error = Some(e.to_string());
                entry.mounted_via = None;
                entry.migrate_target = None;
                entry.migrate_streak = 0;
                entry.state = MountState::Error { retry_secs: 0 };
            }
        }
    }

    async fn do_attempt(
        &self,
        spec: &MountSpec,
        ctx: &EvalCtx,
        defaults: &Defaults,
        world: u64,
        lease: &OperationLease,
    ) {
        let cands = reconcile::candidates(spec, ctx, defaults.probe_timeout_ms);
        let chosen = crate::probe::select_endpoint(&cands, PROBE_STAGGER, self.probe_fn()).await;

        let Some(idx) = chosen else {
            // Nothing answered. Before settling into "waiting", see whether one
            // of these endpoints is a machine we are allowed to wake.
            if let Some(address) = self.try_wake_on_lan(spec, &cands).await {
                self.set_state(spec, MountState::Waking { address }).await;
            } else {
                self.set_state(
                    spec,
                    MountState::Waiting {
                        reason: "no endpoint reachable".into(),
                    },
                )
                .await;
            }
            return;
        };

        let address = spec.endpoints[idx].address.clone();
        info!(mount = %spec.name, %address, "probe passed, mounting");
        match self
            .attempt_mount(spec, &address, defaults, false, lease)
            .await
        {
            Ok(()) => {
                info!(mount = %spec.name, %address, "mounted");
                let mut rt = self.runtime.lock().await;
                rt.entry(spec.name.clone())
                    .or_default()
                    .note_mounted(&address);
            }
            Err(e) => {
                warn!(mount = %spec.name, error = %e, "mount failed");
                let mut rt = self.runtime.lock().await;
                let entry = rt.entry(spec.name.clone()).or_default();
                let backoff = reconcile::next_backoff(entry.backoff, &spec.policy.retry);
                entry.backoff = Some(backoff);
                entry.next_retry_at = Instant::now().checked_add(backoff);
                entry.backoff_key = Some(world);
                entry.last_error = Some(e.to_string());
                entry.state = MountState::Error {
                    retry_secs: backoff.as_secs(),
                };
            }
        }
    }

    /// Send a magic packet to the first unreachable endpoint configured for
    /// Wake-on-LAN, at most once per `wol_wait_secs` so a permanently-off
    /// server is not spammed every poll.
    async fn try_wake_on_lan(
        &self,
        spec: &MountSpec,
        cands: &[crate::probe::Candidate],
    ) -> Option<String> {
        if self.sleeping.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
            return None;
        }
        for cand in cands {
            let endpoint = spec.endpoints.get(cand.index)?;
            let Some(wol) = endpoint.wake_on_lan.clone() else {
                continue;
            };

            let wait = Duration::from_secs(endpoint.wol_wait_secs);
            {
                let rt = self.runtime.lock().await;
                let recently_sent = rt
                    .get(&spec.name)
                    .and_then(|e| e.last_wol_at)
                    .is_some_and(|at| at.elapsed() < wait);
                if recently_sent {
                    return Some(endpoint.address.clone());
                }
            }

            match self.ops.send_wol(wol).await {
                Ok(()) => {
                    info!(
                        mount = %spec.name, address = %endpoint.address,
                        "endpoint unreachable; sent wake-on-LAN"
                    );
                    let mut rt = self.runtime.lock().await;
                    rt.entry(spec.name.clone()).or_default().last_wol_at = Some(Instant::now());
                    return Some(endpoint.address.clone());
                }
                Err(e) => {
                    warn!(mount = %spec.name, error = %e, "wake-on-LAN failed");
                }
            }
        }
        None
    }

    /// Mount with credentials: for SMB mounts with a username, fetch the
    /// password from the Keychain first (item server = `keychain_server` or the
    /// endpoint address).
    async fn attempt_mount(
        &self,
        spec: &MountSpec,
        address: &str,
        defaults: &Defaults,
        forced: bool,
        lease: &OperationLease,
    ) -> Result<()> {
        let password = match (&spec.protocol, &spec.username) {
            (Protocol::Smb, Some(user)) => {
                let server = spec.keychain_server.as_deref().unwrap_or(address);
                let outcome = self
                    .ops
                    .fetch_password(
                        server.to_string(),
                        user.clone(),
                        KEYCHAIN_TIMEOUT,
                        lease.clone(),
                    )
                    .await;
                match outcome {
                    FetchOutcome::Found(pw) => Some(pw),
                    FetchOutcome::CleanupPending => {
                        return Err(crate::mountcmd::CleanupPending.into())
                    }
                    other => {
                        // Mounting anyway can succeed as guest against a server
                        // with anonymous access, silently leaving the user
                        // authenticated as nobody with the wrong permissions.
                        if !spec.policy.allow_passwordless {
                            anyhow::bail!(
                                "no usable credential for `{server}` as `{user}` ({}); \
                                 set policy.allow_passwordless to mount anyway",
                                other.reason()
                            );
                        }
                        warn!(
                            mount = %spec.name, %server, %user, reason = other.reason(),
                            "no Keychain credential; mounting without one as configured"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        self.ops.prepare_mount(spec.clone(), lease.clone()).await?;
        let ctx = self.eval_ctx().await;
        anyhow::ensure!(
            forced || ctx.when_holds(&spec.when),
            "mount conditions changed"
        );
        let candidate = reconcile::candidates(spec, &ctx, defaults.probe_timeout_ms)
            .into_iter()
            .find(|c| c.address == address)
            .context("endpoint no longer eligible")?;
        anyhow::ensure!(
            self.ops
                .probe(candidate.address, candidate.port, candidate.timeout)
                .await,
            "endpoint failed final reachability probe"
        );
        anyhow::ensure!(!self.sleeping.load(Ordering::SeqCst), "system is sleeping");
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "daemon is stopping");
        self.ops
            .mount(spec.clone(), address.to_string(), password, lease.clone())
            .await
    }

    /// Explain the evaluation of one mount (or all), including live probe
    /// results for eligible endpoints.
    pub async fn explain(&self, name: Option<&str>) -> Result<Vec<reconcile::Explanation>> {
        let cfg = self.cfg.read().await.clone();
        if let Some(n) = name {
            cfg.mount(n)
                .with_context(|| format!("no mount named `{n}`"))?;
        }
        let ctx = self.eval_ctx().await;
        let entries = self.ops.list_mounts()?;
        let settling = self.is_settling().await;
        let mut out = Vec::new();

        for spec in cfg
            .mounts
            .iter()
            .filter(|m| name.is_none_or(|n| m.name == n))
        {
            let mountpoint = self.mountpoint_of(spec);
            let mounted = crate::mtab::find_by_mountpoint(&entries, &mountpoint).is_some();
            let rt = {
                let runtime = self.runtime.lock().await;
                runtime.get(&spec.name).cloned().unwrap_or_default()
            };
            let view = MountView {
                mounted,
                user_suppressed: rt.user_suppressed,
                gone_for: rt.gone_since.map(|s| s.elapsed()),
                settling,
                health_failures: rt.health_failures,
                migrate_to: None,
                retry_due: rt.next_retry_at.is_none_or(|at| Instant::now() >= at),
            };

            let mut ex = reconcile::explain(spec, &ctx, &view);
            ex.state = rt.state.to_string();
            let cands = reconcile::candidates(spec, &ctx, cfg.defaults.probe_timeout_ms);
            for c in &cands {
                let up = self.ops.probe(c.address.clone(), c.port, c.timeout).await;
                if let Some(ep) = ex.endpoints.get_mut(c.index) {
                    ep.reachable = Some(up);
                    ep.current = rt.mounted_via.as_deref() == Some(c.address.as_str());
                }
            }
            out.push(ex);
        }
        Ok(out)
    }

    /// Unmount everything that asked to be clean before sleep. Politely: the
    /// machine is about to suspend, and forcing an unmount out from under an
    /// in-flight write is exactly the data loss this is meant to prevent.
    pub async fn unmount_for_sleep(&self) {
        self.sleeping.store(true, Ordering::SeqCst);
        let budget = self.cfg.read().await.defaults.sleep_unmount_budget_secs;
        let work = async {
            let lifecycle = Arc::new(self.lifecycle.clone().write_owned().await);
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            let cfg = self.cfg.read().await.clone();
            let entries = match self.ops.list_mounts() {
                Ok(entries) => entries,
                Err(error) => {
                    warn!(%error, "cannot enumerate mounts before sleep");
                    return;
                }
            };
            let mut tasks = tokio::task::JoinSet::new();
            for spec in cfg.mounts {
                let mountpoint = self.mountpoint_of(&spec);
                if !spec.policy.unmount_before_sleep
                    || crate::mtab::find_by_mountpoint(&entries, &mountpoint).is_none()
                {
                    continue;
                }
                let ops = self.ops.clone();
                let Some(claim) = self.claim(&spec.name).await else {
                    continue;
                };
                let lease: OperationLease = Arc::new((claim, lifecycle.clone()));
                tasks
                    .spawn(async move { (spec.name, ops.unmount(mountpoint, false, lease).await) });
            }
            while let Some(result) = tasks.join_next().await {
                if let Ok((name, result)) = result {
                    match result {
                        Ok(()) => {
                            let mut rt = self.runtime.lock().await;
                            let entry = rt.entry(name).or_default();
                            entry.mounted_via = None;
                            entry.health_failures = 0;
                            entry.state = MountState::Unmounted { manual: false };
                        }
                        Err(e) => warn!(mount = %name, error = %e, "pre-sleep unmount failed"),
                    }
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(budget), work)
            .await
            .is_err()
        {
            warn!("pre-sleep unmount budget exhausted");
        }
    }

    /// Main loop: reconcile on a poll interval, and sooner when hinted
    /// (debounced so event storms coalesce into one pass). Wake is detected
    /// both by IOKit (precise) and by comparing wall-clock against monotonic
    /// elapsed (a fallback that works even if registration failed).
    pub async fn run(
        self: Arc<Self>,
        mut power: Option<tokio::sync::mpsc::UnboundedReceiver<power::PowerEvent>>,
    ) {
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            let defaults = self.cfg.read().await.defaults.clone();
            let settle = Duration::from_secs(defaults.wake_settle_secs);
            let wall_before = std::time::SystemTime::now();
            let mono_before = Instant::now();
            let work = async {
                self.reconcile_once().await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(defaults.poll_interval_secs.max(1))) => {}
                    _ = self.hint.notified() => {
                        tokio::time::sleep(Duration::from_secs(defaults.debounce_secs)).await;
                    }
                }
            };
            let event = tokio::select! {
                biased;
                event = async {
                    match power.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => Some(event),
                _ = work => None,
            };
            // Aborted tasks transfer their leases to any child still being reaped.
            match event {
                Some(Some(power::PowerEvent::WillSleep { ack })) => {
                    self.unmount_for_sleep().await;
                    let _ = ack.send(());
                }
                Some(Some(power::PowerEvent::DidWake)) => self.note_wake(settle).await,
                Some(None) => power = None,
                None => {}
            }
            if wake_gap(
                wall_before.elapsed().unwrap_or_default(),
                mono_before.elapsed(),
            )
            .is_some()
            {
                self.note_wake(settle).await;
            }
        }
    }
}

/// If wall-clock time advanced substantially more than monotonic time, the
/// system slept in between; returns the sleep gap.
pub fn wake_gap(wall_elapsed: Duration, mono_elapsed: Duration) -> Option<Duration> {
    const THRESHOLD: Duration = Duration::from_secs(30);
    wall_elapsed
        .checked_sub(mono_elapsed)
        .filter(|gap| *gap > THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::fake::FakeOps;

    const MP: &str = "/private/tmp/mountd-test-nonexistent";

    /// Synthetic mountpoints use lexical paths with no filesystem lookup.
    fn mp(suffix: &str) -> String {
        mountcmd::normalize_mountpoint(&format!("{MP}{suffix}"))
            .to_string_lossy()
            .to_string()
    }

    fn engine_with(toml: &str, ops: Arc<FakeOps>) -> (Arc<Engine>, Arc<FakeOps>) {
        let cfg = Config::from_toml_str(toml).unwrap();
        let engine = Engine::with_ops(PathBuf::from("/nonexistent/config.toml"), cfg, ops.clone());
        (engine, ops)
    }

    /// One SMB mount, two endpoints: LAN preferred, tailnet fallback.
    fn two_endpoint_toml() -> String {
        format!(
            r#"
[defaults]
probe_timeout_ms = 50
health_interval_secs = 0

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"

  [mount.policy]
  allow_passwordless = true

  [[mount.endpoint]]
  address = "10.0.0.1"

  [[mount.endpoint]]
  address = "100.64.0.1"
"#
        )
    }

    async fn notified(signal: &Notify) {
        tokio::time::timeout(Duration::from_secs(3), signal.notified())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_credentials_retain_claim_and_reload_guard_until_reap() {
        let child = Arc::new(crate::ops::fake::FakeChild::default());
        let config =
            two_endpoint_toml().replace("share = \"s\"", "share = \"s\"\nusername = \"test\"");
        let (eng, ops) = engine_with(
            &config,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.credential_child = Some(child.clone());
            }),
        );
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_mount("nas").await })
        };
        notified(&child.started).await;
        task.abort();
        let _ = task.await;
        notified(&child.cancelled).await;
        assert!(eng.claim("nas").await.is_none());
        assert!(eng.lifecycle.try_write().is_err());
        assert!(ops.mount_calls().is_empty());
        child.release.notify_one();
        notified(&child.reaped).await;
        assert!(eng.claim("nas").await.is_some());
        assert!(eng.lifecycle.try_write().is_ok());
    }

    #[tokio::test]
    async fn passwordless_does_not_bypass_pending_credential_cleanup() {
        let config =
            two_endpoint_toml().replace("share = \"s\"", "share = \"s\"\nusername = \"test\"");
        let (eng, ops) = engine_with(
            &config,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.passwords
                    .insert("10.0.0.1".into(), FetchOutcome::CleanupPending);
            }),
        );
        assert!(eng.force_mount("nas").await.is_err());
        assert!(ops.mount_calls().is_empty());
    }

    #[tokio::test]
    async fn preparation_is_followed_by_sleep_shutdown_and_probe_gates() {
        for change in ["sleep", "shutdown", "disconnect"] {
            let child = Arc::new(crate::ops::fake::FakeChild::default());
            let (eng, ops) = engine_with(
                &two_endpoint_toml(),
                FakeOps::with(|st| {
                    st.reachable.insert("10.0.0.1".into());
                    st.prepare_child = Some(child.clone());
                }),
            );
            let task = {
                let eng = eng.clone();
                tokio::spawn(async move { eng.force_mount("nas").await })
            };
            notified(&child.started).await;
            match change {
                "sleep" => eng.sleeping.store(true, Ordering::SeqCst),
                "shutdown" => eng.begin_shutdown(),
                _ => ops.state().reachable.clear(),
            }
            child.release.notify_one();
            assert!(task.await.unwrap().is_err(), "{change}");
            assert!(ops.mount_calls().is_empty(), "{change}");
        }
    }

    #[tokio::test]
    async fn network_identity_is_refreshed_after_dependencies() {
        for forced in [false, true] {
            let mut config = two_endpoint_toml().replace(
                "[[mount]]",
                "[networks.home]\nsubnet = \"10.0.0.0/24\"\n[[mount]]",
            );
            config = if forced {
                config
                    .replace(
                        "address = \"10.0.0.1\"",
                        "address = \"10.0.0.1\"\nonly_if = \"network:home\"",
                    )
                    .replace(
                        "address = \"100.64.0.1\"",
                        "address = \"100.64.0.1\"\nonly_if = \"network:home\"",
                    )
            } else {
                config.replace(
                    "[mount.policy]",
                    "[mount.when]\nall_of = [\"network:home\"]\n[mount.policy]",
                )
            };
            config = config.replace("share = \"s\"", "share = \"s\"\nusername = \"test\"");
            let child = Arc::new(crate::ops::fake::FakeChild::default());
            let (eng, ops) = engine_with(
                &config,
                FakeOps::with(|st| {
                    st.reachable.insert("10.0.0.1".into());
                    st.snapshot
                        .local_addrs
                        .push(("10.0.0.2".parse().unwrap(), 24));
                    st.credential_child = Some(child.clone());
                }),
            );
            let task = {
                let eng = eng.clone();
                tokio::spawn(async move {
                    if forced {
                        eng.force_mount("nas").await
                    } else {
                        eng.reconcile_once().await;
                        Ok(())
                    }
                })
            };
            notified(&child.started).await;
            // Change networks during the final dependency check, after initial selection.
            ops.state().snapshot_after_deps = Some(crate::fingerprint::NetSnapshot::default());
            child.release.notify_one();
            let result = task.await.unwrap();
            if forced {
                assert!(result.is_err());
            }
            assert!(ops.mount_calls().is_empty(), "forced={forced}");
        }
    }

    #[tokio::test]
    async fn cancelled_mount_retains_both_guards_until_child_reap() {
        let child = Arc::new(crate::ops::fake::FakeChild::default());
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.mount_child = Some(child.clone());
            }),
        );
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_mount("nas").await })
        };
        notified(&child.started).await;
        task.abort();
        let _ = task.await;
        notified(&child.cancelled).await;
        assert!(eng.claim("nas").await.is_none());
        assert!(eng.lifecycle.try_write().is_err());
        assert!(eng.force_unmount("nas").await.is_err());
        eng.begin_shutdown();
        assert!(tokio::time::timeout(Duration::from_millis(20), eng.drain())
            .await
            .is_err());
        child.release.notify_one();
        notified(&child.reaped).await;
        tokio::time::timeout(Duration::from_secs(1), eng.drain())
            .await
            .unwrap();
        assert!(eng.claim("nas").await.is_some());
        eng.note_wake(Duration::ZERO).await;
        assert!(eng.force_mount("nas").await.is_err());
        assert!(eng.force_unmount("nas").await.is_err());
        eng.reconcile_once().await;
        assert!(ops.mount_calls().is_empty());
        assert!(ops.unmount_calls().is_empty());
    }

    #[tokio::test]
    async fn sleep_interrupt_keeps_cancelled_child_claim_until_reap() {
        let child = Arc::new(crate::ops::fake::FakeChild::default());
        let config =
            two_endpoint_toml().replace("[defaults]", "[defaults]\nsleep_unmount_budget_secs = 1");
        let (eng, _) = engine_with(
            &config,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.mount_child = Some(child.clone());
            }),
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.run(Some(rx)).await })
        };
        notified(&child.started).await;
        let (ack, received) = tokio::sync::oneshot::channel();
        tx.send(power::PowerEvent::WillSleep { ack }).unwrap();
        notified(&child.cancelled).await;
        tokio::time::timeout(Duration::from_secs(3), received)
            .await
            .unwrap()
            .unwrap();
        assert!(eng.claim("nas").await.is_none());
        assert!(eng.lifecycle.try_write().is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), eng.note_wake(Duration::ZERO))
                .await
                .is_err()
        );
        assert!(eng.sleeping.load(Ordering::SeqCst));
        task.abort();
        let _ = task.await;
        child.release.notify_one();
        notified(&child.reaped).await;
        eng.note_wake(Duration::ZERO).await;
        assert!(eng.claim("nas").await.is_some());
    }

    #[tokio::test]
    async fn sleep_unmount_budget_keeps_writer_until_child_reap() {
        let child = Arc::new(crate::ops::fake::FakeChild::default());
        let config =
            two_endpoint_toml().replace("[defaults]", "[defaults]\nsleep_unmount_budget_secs = 1");
        let (eng, _) = engine_with(
            &config,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
                st.unmount_child = Some(child.clone());
            }),
        );
        let sleep = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.unmount_for_sleep().await })
        };
        notified(&child.started).await;
        tokio::time::timeout(Duration::from_secs(3), sleep)
            .await
            .unwrap()
            .unwrap();
        notified(&child.cancelled).await;
        assert!(eng.claim("nas").await.is_none());
        assert!(eng.lifecycle.try_read().is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), eng.note_wake(Duration::ZERO))
                .await
                .is_err()
        );
        child.release.notify_one();
        notified(&child.reaped).await;
        eng.note_wake(Duration::ZERO).await;
        assert!(eng.claim("nas").await.is_some());
    }

    #[tokio::test]
    async fn dependency_change_during_credentials_prevents_mount() {
        let config = two_endpoint_toml()
            .replace(
                "[[mount]]",
                "[deps.vpn]\ncmd = \"/synthetic/vpn\"\nargs = []\n\n[[mount]]",
            )
            .replace("share = \"s\"", "share = \"s\"\nusername = \"test\"")
            .replace(
                "[mount.policy]",
                "[mount.when]\nall_of = [\"dep:vpn\"]\n[mount.policy]",
            );
        let (eng, ops) = engine_with(
            &config,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.drop_deps_on_password = true;
            }),
        );
        eng.reconcile_once().await;
        assert!(ops.mount_calls().is_empty());
        assert!(eng.status().await[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("conditions changed"));
    }

    #[tokio::test]
    async fn failed_reload_keeps_runtime_and_repeated_reload_resets_changed_spec() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = two_endpoint_toml();
        let ops = FakeOps::with(|st| {
            st.reachable.insert("10.0.0.1".into());
        });
        let eng = Engine::with_ops(path.clone(), Config::from_toml_str(&config).unwrap(), ops);
        eng.force_mount("nas").await.unwrap();
        std::fs::write(&path, "invalid = [").unwrap();
        assert!(eng.reload().await.is_err());
        assert_eq!(eng.status().await[0].state_kind, "mounted");
        std::fs::write(&path, config.replace("10.0.0.1", "10.0.0.2")).unwrap();
        eng.reload().await.unwrap();
        eng.reload().await.unwrap();
        assert_eq!(eng.status().await[0].state_kind, "unknown");
        assert!(eng.runtime.lock().await.is_empty());
        assert_eq!(
            eng.cfg.read().await.mount("nas").unwrap().endpoints[0].address,
            "10.0.0.2"
        );
    }

    #[tokio::test]
    async fn network_snapshot_change_clears_backoff_without_named_conditions() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.mount_fails
                    .insert("10.0.0.1".into(), "synthetic failure".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);
        {
            let mut state = ops.state();
            state.mount_fails.clear();
            state.snapshot.utun_present = true;
        }
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 2);
        assert_eq!(eng.status().await[0].state_kind, "mounted");
    }

    #[tokio::test]
    async fn changed_mount_source_starts_a_new_health_streak() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.reconcile_once().await;
        eng.runtime
            .lock()
            .await
            .get_mut("nas")
            .unwrap()
            .health_failures = 2;
        ops.state()
            .mounted
            .insert(mp(""), "//user@100.64.0.1/s".into());
        eng.reconcile_once().await;
        assert!(ops.unmount_calls().is_empty());
        let runtime = eng.runtime.lock().await;
        let entry = runtime.get("nas").unwrap();
        assert_eq!(entry.health_failures, 1);
        assert_eq!(entry.mounted_via.as_deref(), Some("100.64.0.1"));
    }

    #[tokio::test]
    async fn enumeration_failure_never_permits_a_mount() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.list_mounts_fails = true;
            }),
        );
        eng.reconcile_once().await;
        assert!(eng.force_mount("nas").await.is_err());
        assert!(eng.explain(None).await.is_err());
        eng.unmount_for_sleep().await;
        assert!(ops.calls().is_empty());
        assert!(eng.status().await[0]
            .last_error
            .as_deref()
            .unwrap()
            .contains("enumerate"));
    }

    #[tokio::test]
    async fn cancelled_manual_attempt_releases_claim() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.probe_delay
                    .insert("10.0.0.1".into(), Duration::from_secs(60));
            }),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), eng.force_mount("nas"))
                .await
                .is_err()
        );
        ops.state().probe_delay.clear();
        eng.force_mount("nas").await.unwrap();
        assert_eq!(ops.mount_calls().len(), 1);
    }

    #[tokio::test]
    async fn unmount_during_attempt_is_rejected_without_side_effects() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.probe_delay
                    .insert("10.0.0.1".into(), Duration::from_secs(60));
            }),
        );
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_mount("nas").await })
        };
        wait_for_probe(&ops).await;
        assert!(eng.force_unmount("nas").await.is_err());
        assert!(ops.unmount_calls().is_empty());
        task.abort();
        let _ = task.await;
    }

    async fn wait_for_probe(ops: &FakeOps) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ops
                .calls()
                .iter()
                .any(|c| matches!(c, crate::ops::fake::Call::Probe { .. }))
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn credential_wait_invalidates_old_probe() {
        let toml =
            two_endpoint_toml().replace("share = \"s\"", "share = \"s\"\nusername = \"test\"");
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.disconnect_on_password = true;
            }),
        );
        eng.reconcile_once().await;
        assert!(ops.mount_calls().is_empty());
        assert_eq!(eng.status().await[0].state_kind, "error");
    }

    #[tokio::test]
    async fn sleep_interrupts_probe_and_blocks_mounts_until_wake() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.probe_delay
                    .insert("10.0.0.1".into(), Duration::from_secs(60));
            }),
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.run(Some(rx)).await })
        };
        wait_for_probe(&ops).await;
        let (ack, received) = tokio::sync::oneshot::channel();
        tx.send(power::PowerEvent::WillSleep { ack }).unwrap();
        tokio::time::timeout(Duration::from_millis(500), received)
            .await
            .unwrap()
            .unwrap();
        ops.state().probe_delay.clear();
        eng.reconcile_once().await;
        assert!(eng.force_mount("nas").await.is_err());
        assert!(ops.mount_calls().is_empty());
        eng.note_wake(Duration::ZERO).await;
        task.abort();
        let _ = task.await;
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);
    }

    #[tokio::test]
    async fn reload_waits_for_attempt_without_blocking_status_and_discards_old_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = two_endpoint_toml();
        std::fs::write(&path, &config).unwrap();
        let ops = FakeOps::with(|st| {
            st.reachable.insert("10.0.0.1".into());
            st.probe_delay
                .insert("10.0.0.1".into(), Duration::from_secs(60));
        });
        let eng = Engine::with_ops(
            path.clone(),
            Config::from_toml_str(&config).unwrap(),
            ops.clone(),
        );
        let attempt = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_mount("nas").await })
        };
        wait_for_probe(&ops).await;
        std::fs::write(&path, config.replace("name = \"nas\"", "name = \"new\"")).unwrap();
        let reload = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.reload().await })
        };
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_millis(100), eng.status())
            .await
            .unwrap();
        assert!(!reload.is_finished());
        attempt.abort();
        let _ = attempt.await;
        reload.await.unwrap().unwrap();
        eng.reload().await.unwrap();
        assert!(eng.runtime.lock().await.is_empty());
        assert_eq!(eng.status().await[0].name, "new");
    }

    #[tokio::test]
    async fn wake_settling_does_not_recover_a_suspect_mount() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.note_wake(Duration::from_secs(60)).await;
        for _ in 0..5 {
            eng.reconcile_once().await;
        }
        assert!(ops.unmount_calls().is_empty());
        assert!(ops.calls().is_empty());
    }

    #[tokio::test]
    async fn migration_reprobes_after_unmount_and_reports_failure() {
        let toml = two_endpoint_toml().replace(
            "allow_passwordless = true",
            "allow_passwordless = true\nmigrate = true\nmigrate_confirmations = 1",
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        ops.revive_endpoint("10.0.0.1");
        ops.state().disconnect_on_unmount = Some("10.0.0.1".into());
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);
        assert_eq!(eng.status().await[0].state_kind, "error");
        assert!(eng.status().await[0].mounted_via.is_none());
    }

    #[tokio::test]
    async fn status_reports_all_configured_mounts() {
        let (eng, _) = engine_with(
            &format!(
                r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "{MP}-a"
  [[mount.endpoint]]
  address = "203.0.113.9"

[[mount]]
name = "b"
protocol = "nfs"
export = "/e"
mountpoint = "{MP}-b"
  [[mount.endpoint]]
  address = "203.0.113.10"
"#
            ),
            FakeOps::with(|_| {}),
        );
        let st = eng.status().await;
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].name, "a");
        assert_eq!(st[0].state, "unknown"); // no reconcile yet
        assert_eq!(st[0].state_kind, "unknown");
    }

    #[tokio::test]
    async fn unreachable_endpoint_waits_without_erroring() {
        let (eng, ops) = engine_with(&two_endpoint_toml(), FakeOps::with(|_| {}));
        eng.reconcile_once().await;
        let st = eng.status().await;
        assert_eq!(st[0].state, "waiting (no endpoint reachable)");
        assert!(st[0].last_error.is_none());
        assert!(ops.mount_calls().is_empty(), "no mount may be attempted");
    }

    #[tokio::test]
    async fn mounts_via_the_highest_priority_reachable_endpoint() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls(), vec![("nas".into(), "10.0.0.1".into())]);
        let st = eng.status().await;
        assert_eq!(st[0].state, "mounted via 10.0.0.1");
        assert_eq!(st[0].state_kind, "mounted");
    }

    #[tokio::test]
    async fn falls_back_to_the_tailnet_endpoint_when_the_lan_is_gone() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls(), vec![("nas".into(), "100.64.0.1".into())]);
    }

    // ---- the review's Critical finding, end to end ----

    #[tokio::test]
    async fn a_healthy_mount_survives_a_transient_condition_loss() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[networks.home]
subnet = "10.0.0.0/24"

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.when]
  all_of = ["network:home"]
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        // Mounted, but the network no longer matches (the host has no
        // 10.0.0.0/24 address), so `when` reads false on every pass.
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
                st.reachable.insert("10.0.0.1".into());
            }),
        );

        eng.reconcile_once().await;
        eng.reconcile_once().await;

        assert!(
            ops.unmount_calls().is_empty(),
            "a mount must survive the grace period, got {:?}",
            ops.unmount_calls()
        );
        assert_eq!(eng.status().await[0].state_kind, "mounted");
    }

    #[tokio::test]
    async fn a_policy_unmount_is_never_forced() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[networks.home]
subnet = "10.0.0.0/24"

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.when]
  all_of = ["network:home"]
  [mount.policy]
  unmount_grace_secs = 0
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        // First pass records when conditions went; second acts on it.
        eng.reconcile_once().await;
        eng.reconcile_once().await;

        let calls = ops.unmount_calls();
        assert!(!calls.is_empty(), "the grace period has elapsed");
        assert!(
            calls.iter().all(|(_, force)| !force),
            "policy unmounts must be polite, got {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_settling_system_does_not_unmount() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50
wake_settle_secs = 60

[networks.home]
subnet = "10.0.0.0/24"

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.when]
  all_of = ["network:home"]
  [mount.policy]
  unmount_grace_secs = 0
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.note_wake(Duration::from_secs(60)).await;
        eng.reconcile_once().await;
        eng.reconcile_once().await;

        assert!(
            ops.unmount_calls().is_empty(),
            "conditions are not trustworthy right after wake"
        );
    }

    // ---- health / recovery ----

    #[tokio::test]
    async fn a_dead_mount_is_detected_and_remounted_elsewhere() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);

        // The LAN server goes away while mounted. getfsstat still reports the
        // mount, so only the liveness probe can notice.
        ops.kill_endpoint("10.0.0.1");
        for _ in 0..3 {
            eng.reconcile_once().await;
        }

        let unmounts = ops.unmount_calls();
        assert!(!unmounts.is_empty(), "the dead mount must be torn down");
        assert!(
            unmounts.iter().all(|(_, force)| *force),
            "a provably dead server is the one case where forcing is right"
        );
        assert_eq!(
            ops.mount_calls().last(),
            Some(&("nas".to_string(), "100.64.0.1".to_string())),
            "it should come back via the surviving endpoint"
        );
    }

    #[tokio::test]
    async fn a_healthy_mount_is_never_recovered() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        for _ in 0..5 {
            eng.reconcile_once().await;
        }
        assert_eq!(ops.mount_calls().len(), 1, "mounted once and left alone");
        assert!(ops.unmount_calls().is_empty());
    }

    // ---- manual unmount ----

    #[tokio::test]
    async fn a_manual_unmount_is_not_undone_by_the_reconciler() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);

        eng.force_unmount("nas").await.unwrap();
        eng.reconcile_once().await;
        eng.reconcile_once().await;

        assert_eq!(
            ops.mount_calls().len(),
            1,
            "the daemon must respect an explicit unmount"
        );
        let st = eng.status().await;
        assert_eq!(st[0].state, "unmounted (manual, until conditions change)");
    }

    #[tokio::test]
    async fn a_conditions_edge_clears_a_manual_unmount() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[deps.tailscale]
cmd = "/usr/bin/true"

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.when]
  all_of = ["dep:tailscale"]
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        eng.force_unmount("nas").await.unwrap();
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);

        // Simulate `when` going false and then true again.
        {
            let mut rt = eng.runtime.lock().await;
            rt.get_mut("nas").unwrap().when_held = false;
        }
        eng.reconcile_once().await;
        assert_eq!(
            ops.mount_calls().len(),
            2,
            "a fresh conditions edge means the user's unmount no longer applies"
        );
    }

    // ---- credentials ----

    #[tokio::test]
    async fn a_missing_credential_fails_loudly_instead_of_mounting_as_guest() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
username = "chris"
mountpoint = "{MP}"
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        eng.reconcile_once().await;

        assert!(ops.mount_calls().is_empty(), "no guest mount may happen");
        let st = eng.status().await;
        assert_eq!(st[0].state_kind, "error");
        let err = st[0].last_error.clone().unwrap();
        assert!(err.contains("no usable credential"), "{err}");
        assert!(err.contains("no Keychain item"), "{err}");
    }

    #[tokio::test]
    async fn allow_passwordless_restores_the_old_behavior() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
username = "chris"
mountpoint = "{MP}"
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);
    }

    // ---- pinned mounts ----

    #[tokio::test]
    async fn a_pinned_mount_keeps_its_endpoint_attribution() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[networks.home]
subnet = "10.0.0.0/24"

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.when]
  all_of = ["network:home"]
  [mount.policy]
  unmount_when_gone = false
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, _) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.reconcile_once().await;
        let st = eng.status().await;
        assert_eq!(st[0].state_kind, "mounted");
        assert_eq!(
            st[0].mounted_via.as_deref(),
            Some("10.0.0.1"),
            "the via-address must survive, the migration and health paths need it"
        );
        assert!(st[0].state.contains("pinned"));
    }

    // ---- migration (roadmap) ----

    #[tokio::test]
    async fn migration_moves_to_a_better_endpoint_once_confirmed() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50
health_interval_secs = 0

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.policy]
  migrate = true
  migrate_confirmations = 2
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
  [[mount.endpoint]]
  address = "100.64.0.1"
"#
        );
        // Only the tailnet endpoint is up at first.
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls(), vec![("nas".into(), "100.64.0.1".into())]);

        // The LAN comes back. It takes migrate_confirmations passes.
        ops.revive_endpoint("10.0.0.1");
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1, "one good probe is not enough");

        eng.reconcile_once().await;
        assert_eq!(
            ops.mount_calls().last(),
            Some(&("nas".to_string(), "10.0.0.1".to_string()))
        );
        assert!(
            ops.unmount_calls().iter().all(|(_, force)| !force),
            "migration must never force an unmount"
        );
    }

    #[tokio::test]
    async fn migration_is_off_by_default() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        ops.revive_endpoint("10.0.0.1");
        for _ in 0..4 {
            eng.reconcile_once().await;
        }
        assert_eq!(ops.mount_calls().len(), 1, "no migration without opt-in");
    }

    #[tokio::test]
    async fn a_busy_share_is_not_migrated() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50
health_interval_secs = 0

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.policy]
  migrate = true
  migrate_confirmations = 2
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
  [[mount.endpoint]]
  address = "100.64.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        ops.state().unmount_needs_force.insert(mp(""));
        ops.revive_endpoint("10.0.0.1");
        eng.reconcile_once().await;
        eng.reconcile_once().await;

        assert_eq!(
            ops.mount_calls().len(),
            1,
            "a copy in flight is worth more than a faster path"
        );
        assert_eq!(eng.status().await[0].state_kind, "mounted");
        assert_eq!(ops.unmount_calls().len(), 1);
        ops.state().unmount_needs_force.clear();
        eng.reconcile_once().await;
        assert_eq!(
            ops.unmount_calls().len(),
            1,
            "busy migration resets confirmations"
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 2);
        assert!(ops.unmount_calls().iter().all(|(_, force)| !force));
    }

    // ---- wake-on-lan (roadmap) ----

    #[tokio::test]
    async fn wake_on_lan_fires_for_an_unreachable_endpoint() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
  wol_wait_secs = 600
  [mount.endpoint.wake_on_lan]
  mac = "aa:bb:cc:dd:ee:ff"
  broadcast = "10.0.0.255"
"#
        );
        let (eng, ops) = engine_with(&toml, FakeOps::with(|_| {}));
        eng.reconcile_once().await;

        assert_eq!(ops.wol_calls(), vec!["aa:bb:cc:dd:ee:ff".to_string()]);
        assert_eq!(eng.status().await[0].state_kind, "waking");

        // Not resent inside the wait window.
        eng.reconcile_once().await;
        assert_eq!(ops.wol_calls().len(), 1);
    }

    #[tokio::test]
    async fn no_wake_on_lan_without_configuration() {
        let (eng, ops) = engine_with(&two_endpoint_toml(), FakeOps::with(|_| {}));
        eng.reconcile_once().await;
        assert!(ops.wol_calls().is_empty());
        assert_eq!(eng.status().await[0].state_kind, "waiting");
    }

    // ---- pre-sleep unmount (roadmap) ----

    #[tokio::test]
    async fn pre_sleep_unmounts_politely() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.unmount_for_sleep().await;
        assert_eq!(ops.unmount_calls(), vec![(mp(""), false)]);
    }

    #[tokio::test]
    async fn pre_sleep_skips_mounts_that_opted_out() {
        let toml = format!(
            r#"
[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.policy]
  unmount_before_sleep = false
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
            }),
        );
        eng.unmount_for_sleep().await;
        assert!(ops.unmount_calls().is_empty());
    }

    // ---- concurrency ----

    #[tokio::test]
    async fn a_slow_mount_does_not_block_its_peers() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 2000

[[mount]]
name = "slow"
protocol = "smb"
share = "s"
mountpoint = "{MP}-slow"
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.9"

[[mount]]
name = "fast"
protocol = "smb"
share = "s"
mountpoint = "{MP}-fast"
  [mount.policy]
  allow_passwordless = true
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.reachable.insert("10.0.0.9".into());
                // the slow endpoint takes most of a second to answer
                st.probe_delay
                    .insert("10.0.0.9".into(), Duration::from_millis(800));
            }),
        );

        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.reconcile_once().await })
        };
        tokio::time::timeout(Duration::from_millis(500), async {
            while !ops.mount_calls().iter().any(|(name, _)| name == "fast") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fast share must mount while the slow probe is pending");
        assert!(!task.is_finished());
        task.await.unwrap();
        assert_eq!(ops.mount_calls().len(), 2);
    }

    #[tokio::test]
    async fn concurrent_attempts_for_one_mount_are_serialized() {
        let child = Arc::new(crate::ops::fake::FakeChild::default());
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.mount_child = Some(child.clone());
            }),
        );
        let first = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.reconcile_once().await })
        };
        notified(&child.started).await;
        eng.reconcile_once().await;
        assert!(ops.mount_calls().is_empty());
        child.release.notify_one();
        first.await.unwrap();
        assert_eq!(
            ops.mount_calls().len(),
            1,
            "two passes must not stack two mounts"
        );
    }

    // ---- backoff ----

    #[tokio::test]
    async fn a_failed_mount_backs_off_and_recovers_on_a_world_change() {
        let toml = format!(
            r#"
[defaults]
probe_timeout_ms = 50

[[mount]]
name = "nas"
protocol = "smb"
share = "s"
mountpoint = "{MP}"
  [mount.policy]
  allow_passwordless = true
  [mount.policy.retry]
  start_secs = 300
  max_secs = 300
  [[mount.endpoint]]
  address = "10.0.0.1"
"#
        );
        let (eng, ops) = engine_with(
            &toml,
            FakeOps::with(|st| {
                st.reachable.insert("10.0.0.1".into());
                st.mount_fails
                    .insert("10.0.0.1".into(), "server rejected".into());
            }),
        );
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);
        assert_eq!(eng.status().await[0].state, "error (retry in 300s)");

        // Still backing off: no second attempt.
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 1);

        // Waking clears stale monotonic deadlines, so the daemon retries at
        // once rather than sitting out five minutes of awake time.
        eng.note_wake(Duration::from_millis(0)).await;
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls().len(), 2);
    }

    #[test]
    fn wake_gap_detects_sleep() {
        assert_eq!(
            wake_gap(Duration::from_secs(5), Duration::from_secs(5)),
            None
        );
        // small scheduling jitter is not a wake
        assert_eq!(
            wake_gap(Duration::from_secs(7), Duration::from_secs(5)),
            None
        );
        // slept 10 minutes: wall advanced, monotonic didn't
        assert_eq!(
            wake_gap(Duration::from_secs(605), Duration::from_secs(5)),
            Some(Duration::from_secs(600))
        );
        // clock skew backwards must not underflow
        assert_eq!(
            wake_gap(Duration::from_secs(3), Duration::from_secs(5)),
            None
        );
    }

    #[tokio::test]
    async fn explain_includes_probe_results_and_state() {
        let (eng, _) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.reachable.insert("100.64.0.1".into());
            }),
        );
        eng.reconcile_once().await;
        let ex = eng.explain(None).await.unwrap();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].endpoints[0].reachable, Some(false));
        assert_eq!(ex[0].endpoints[1].reachable, Some(true));
        assert!(ex[0].endpoints[1].current, "marks the endpoint in use");
        assert!(ex[0].state.starts_with("mounted"));

        assert!(eng.explain(Some("nope")).await.is_err());
    }

    #[tokio::test]
    async fn force_mount_on_an_already_mounted_share_is_a_no_op() {
        let (eng, ops) = engine_with(
            &two_endpoint_toml(),
            FakeOps::with(|st| {
                st.mounted.insert(mp(""), "//user@10.0.0.1/s".into());
                st.reachable.insert("10.0.0.1".into());
            }),
        );
        eng.force_mount("nas").await.unwrap();
        assert!(ops.mount_calls().is_empty());
    }

    #[tokio::test]
    async fn force_mount_unknown_name_errors() {
        let (eng, _) = engine_with(&two_endpoint_toml(), FakeOps::with(|_| {}));
        assert!(eng.force_mount("nope").await.is_err());
    }
}
