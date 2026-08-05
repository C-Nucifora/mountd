//! The pure decision core: given config + evaluated conditions + what the
//! engine remembers, decide what each logical mount should do. No I/O here —
//! everything is unit-testable.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::config::{CondRef, Endpoint, MountSpec, Probe, Retry, WhenClause};
use crate::probe::Candidate;

/// Evaluated world state fed into decisions.
#[derive(Debug, Clone, Default)]
pub struct EvalCtx {
    pub matched_networks: BTreeSet<String>,
    pub deps_up: BTreeMap<String, bool>,
}

impl EvalCtx {
    pub fn cond_holds(&self, cond: &CondRef) -> bool {
        match cond {
            CondRef::Network(n) => self.matched_networks.contains(n),
            CondRef::Dep(d) => self.deps_up.get(d).copied().unwrap_or(false),
        }
    }

    /// `all_of` must all hold, and if `any_of` is non-empty at least one must
    /// hold. A mount with an empty `when` clause always wants to be mounted.
    pub fn when_holds(&self, when: &WhenClause) -> bool {
        let all_ok = when.all_of.iter().all(|c| self.cond_holds(c));
        let any_ok = when.any_of.is_empty() || when.any_of.iter().any(|c| self.cond_holds(c));
        all_ok && any_ok
    }

    /// A stable fingerprint of the world, used to invalidate a retry backoff
    /// when the conditions that justified it have changed.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for n in &self.matched_networks {
            n.hash(&mut h);
        }
        for (d, up) in &self.deps_up {
            d.hash(&mut h);
            up.hash(&mut h);
        }
        h.finish()
    }
}

/// Endpoints currently worth probing, in priority order: `only_if` condition
/// holds (if set) and `requires` dep is up (if set).
pub fn eligible_endpoints<'a>(spec: &'a MountSpec, ctx: &EvalCtx) -> Vec<(usize, &'a Endpoint)> {
    spec.endpoints
        .iter()
        .enumerate()
        .filter(|(_, ep)| {
            ep.only_if.as_ref().is_none_or(|c| ctx.cond_holds(c))
                && ep.requires.as_ref().is_none_or(|c| ctx.cond_holds(c))
        })
        .collect()
}

/// Build probe candidates for the eligible endpoints of a mount.
pub fn candidates(spec: &MountSpec, ctx: &EvalCtx, default_timeout_ms: u64) -> Vec<Candidate> {
    let Probe::Tcp(port) = spec.effective_probe();
    eligible_endpoints(spec, ctx)
        .into_iter()
        .map(|(index, ep)| Candidate {
            index,
            address: ep.address.clone(),
            port,
            timeout: Duration::from_millis(ep.probe_timeout_ms.unwrap_or(default_timeout_ms)),
        })
        .collect()
}

/// Everything about one mount's current situation that a decision depends on.
/// Built by the engine from config plus `MountRuntime`.
#[derive(Debug, Clone, Default)]
pub struct MountView {
    pub mounted: bool,
    /// A user explicitly unmounted this; leave it alone until conditions move.
    pub user_suppressed: bool,
    /// How long `when` has continuously failed to hold. `None` means it holds,
    /// or that this is the first pass to see it gone.
    pub gone_for: Option<Duration>,
    /// The system woke recently, so condition readings are not yet trustworthy.
    pub settling: bool,
    /// Consecutive failed liveness probes of the endpoint we are mounted via.
    pub health_failures: u32,
    /// A confirmed higher-priority endpoint to move to.
    pub migrate_to: Option<usize>,
    /// The retry backoff has elapsed (or there was none).
    pub retry_due: bool,
}

/// What the engine should do about one mount right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Conditions hold, not mounted, endpoints eligible: probe + mount.
    Attempt,
    /// Conditions hold but no endpoint is currently eligible (e.g. waiting
    /// for Tailscale). Not an error; recheck on the next hint.
    Waiting,
    /// Mounted and healthy.
    Keep,
    /// Conditions stopped holding, and have stayed gone long enough to act.
    Unmount { force: bool },
    /// Mounted but the server has stopped answering: tear it down and remount.
    Recover,
    /// Move to a higher-priority endpoint.
    Migrate(usize),
    /// A user unmounted this deliberately.
    Suppressed,
    /// A retry is scheduled but not yet due.
    Backoff,
    /// Nothing to do (not mounted and shouldn't be, or gone but policy
    /// says leave it).
    Idle,
}

pub fn decide(spec: &MountSpec, ctx: &EvalCtx, view: &MountView) -> Decision {
    let should = ctx.when_holds(&spec.when);
    match (should, view.mounted) {
        (true, true) => {
            if view.health_failures >= spec.policy.health_failures {
                Decision::Recover
            } else if let Some(target) = view.migrate_to {
                Decision::Migrate(target)
            } else {
                Decision::Keep
            }
        }
        (true, false) => {
            if view.user_suppressed {
                Decision::Suppressed
            } else if !view.retry_due {
                Decision::Backoff
            } else if eligible_endpoints(spec, ctx).is_empty() {
                Decision::Waiting
            } else {
                Decision::Attempt
            }
        }
        (false, true) => {
            if !spec.policy.unmount_when_gone {
                Decision::Idle
            } else if view.settling {
                // The network has not finished coming up after wake; a
                // condition that reads false right now proves nothing.
                Decision::Keep
            } else {
                let grace = Duration::from_secs(spec.policy.unmount_grace_secs);
                match view.gone_for {
                    Some(gone) if gone >= grace => Decision::Unmount { force: false },
                    _ => Decision::Keep,
                }
            }
        }
        (false, false) => Decision::Idle,
    }
}

/// One condition's contribution to an explanation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CondExplain {
    pub cond: String,
    pub holds: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EndpointExplain {
    pub address: String,
    pub eligible: bool,
    /// The condition that blocked this endpoint, when ineligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    /// Live probe result for eligible endpoints (filled by the engine;
    /// None when no probe was run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachable: Option<bool>,
    /// True when this is the endpoint the mount is currently realized via.
    #[serde(default)]
    pub current: bool,
}

/// Full answer to "why is/isn't this mounted right now".
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Explanation {
    pub name: String,
    pub mounted: bool,
    pub should_mount: bool,
    pub when_any_of: Vec<CondExplain>,
    pub when_all_of: Vec<CondExplain>,
    pub endpoints: Vec<EndpointExplain>,
    pub decision: String,
    /// The lifecycle state the daemon is tracking, e.g. `suspect`.
    #[serde(default)]
    pub state: String,
}

/// Explain a mount's evaluation. Pure — probe results are filled in later
/// by the engine for eligible endpoints.
pub fn explain(spec: &MountSpec, ctx: &EvalCtx, view: &MountView) -> Explanation {
    let cond_explain = |c: &CondRef| CondExplain {
        cond: c.to_string(),
        holds: ctx.cond_holds(c),
    };
    let endpoints = spec
        .endpoints
        .iter()
        .map(|ep| {
            let blocked_by = [&ep.only_if, &ep.requires]
                .into_iter()
                .flatten()
                .find(|c| !ctx.cond_holds(c))
                .map(|c| c.to_string());
            EndpointExplain {
                address: ep.address.clone(),
                eligible: blocked_by.is_none(),
                blocked_by,
                reachable: None,
                current: false,
            }
        })
        .collect();
    Explanation {
        name: spec.name.clone(),
        mounted: view.mounted,
        should_mount: ctx.when_holds(&spec.when),
        when_any_of: spec.when.any_of.iter().map(cond_explain).collect(),
        when_all_of: spec.when.all_of.iter().map(cond_explain).collect(),
        endpoints,
        decision: format!("{:?}", decide(spec, ctx, view)),
        state: String::new(),
    }
}

/// Exponential backoff with a cap: 0 -> start, then doubling to max.
/// Saturating, so a large configured value can never panic the reconcile path.
pub fn next_backoff(current: Option<Duration>, retry: &Retry) -> Duration {
    let start = Duration::from_secs(retry.start_secs);
    let max = Duration::from_secs(retry.max_secs);
    match current {
        None => start.min(max),
        Some(cur) => cur.checked_mul(2).unwrap_or(max).min(max),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const TOML: &str = r#"
[networks.home]
subnet = "192.168.1.0/24"

[deps.tailscale]
cmd = "/usr/bin/true"

[[mount]]
name = "nas"
protocol = "smb"
share = "media"
mountpoint = "~/mnt/nas"

  [mount.when]
  any_of = ["network:home", "dep:tailscale"]

  [[mount.endpoint]]
  address = "192.168.1.5"
  only_if = "network:home"

  [[mount.endpoint]]
  address = "100.64.0.10"
  requires = "dep:tailscale"
  probe_timeout_ms = 4000

[[mount]]
name = "pinned"
protocol = "nfs"
export = "/e"
mountpoint = "~/mnt/pinned"

  [mount.when]
  all_of = ["dep:tailscale"]

  [mount.policy]
  unmount_when_gone = false

  [[mount.endpoint]]
  address = "100.64.0.11"
  requires = "dep:tailscale"
"#;

    fn cfg() -> Config {
        Config::from_toml_str(TOML).unwrap()
    }

    fn ctx(home: bool, ts: bool) -> EvalCtx {
        let mut c = EvalCtx::default();
        if home {
            c.matched_networks.insert("home".into());
        }
        c.deps_up.insert("tailscale".into(), ts);
        c
    }

    /// A view for a mount that is not mounted and has no pending retry.
    fn absent() -> MountView {
        MountView {
            retry_due: true,
            ..Default::default()
        }
    }

    /// A view for a healthy mounted share.
    fn present() -> MountView {
        MountView {
            mounted: true,
            retry_due: true,
            ..Default::default()
        }
    }

    #[test]
    fn empty_when_clause_always_holds() {
        let w = WhenClause::default();
        assert!(EvalCtx::default().when_holds(&w));
    }

    #[test]
    fn any_of_needs_one() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        assert!(ctx(true, false).when_holds(&m.when));
        assert!(ctx(false, true).when_holds(&m.when));
        assert!(!ctx(false, false).when_holds(&m.when));
    }

    #[test]
    fn all_of_needs_all() {
        let cfg = cfg();
        let m = cfg.mount("pinned").unwrap();
        assert!(ctx(false, true).when_holds(&m.when));
        assert!(!ctx(false, false).when_holds(&m.when));
    }

    #[test]
    fn endpoint_filtering_by_condition() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();

        // at home with tailscale down: only the LAN endpoint
        let eps = eligible_endpoints(m, &ctx(true, false));
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].1.address, "192.168.1.5");

        // away with tailscale up: only the tailnet endpoint
        let eps = eligible_endpoints(m, &ctx(false, true));
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].1.address, "100.64.0.10");

        // both: both, LAN first (priority preserved)
        let eps = eligible_endpoints(m, &ctx(true, true));
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].1.address, "192.168.1.5");
    }

    #[test]
    fn candidates_carry_gate_port_and_timeouts() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let cands = candidates(m, &ctx(true, true), 1500);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].port, 445); // smb default gate
        assert_eq!(cands[0].timeout, Duration::from_millis(1500)); // default
        assert_eq!(cands[1].timeout, Duration::from_millis(4000)); // per-endpoint
        assert_eq!(cands[1].index, 1); // original endpoint index kept
    }

    #[test]
    fn decision_matrix() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();

        assert_eq!(decide(m, &ctx(true, false), &absent()), Decision::Attempt);
        assert_eq!(decide(m, &ctx(true, false), &present()), Decision::Keep);
        assert_eq!(decide(m, &ctx(false, false), &absent()), Decision::Idle);
    }

    // ---- hysteresis (the review's Critical finding) ----

    #[test]
    fn conditions_gone_briefly_does_not_unmount() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            gone_for: Some(Duration::from_secs(5)), // default grace is 30s
            ..present()
        };
        assert_eq!(decide(m, &ctx(false, false), &view), Decision::Keep);
    }

    #[test]
    fn conditions_gone_for_the_full_grace_unmounts_politely() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            gone_for: Some(Duration::from_secs(30)),
            ..present()
        };
        assert_eq!(
            decide(m, &ctx(false, false), &view),
            Decision::Unmount { force: false },
            "policy unmounts must never force: a busy mount would lose writes"
        );
    }

    #[test]
    fn first_pass_seeing_conditions_gone_never_unmounts() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            gone_for: None,
            ..present()
        };
        assert_eq!(decide(m, &ctx(false, false), &view), Decision::Keep);
    }

    #[test]
    fn a_settling_system_never_unmounts_however_long_conditions_have_been_gone() {
        // The observed production failure: 147ms after wake, conditions read
        // false and a healthy Time Machine mount was force-unmounted.
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            gone_for: Some(Duration::from_secs(3600)),
            settling: true,
            ..present()
        };
        assert_eq!(decide(m, &ctx(false, false), &view), Decision::Keep);
    }

    // ---- health / recovery ----

    #[test]
    fn a_mount_whose_endpoint_stopped_answering_is_recovered() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            health_failures: 3, // default threshold
            ..present()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Recover);
    }

    #[test]
    fn health_failures_below_the_threshold_keep_the_mount() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            health_failures: 2,
            ..present()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Keep);
    }

    #[test]
    fn recovery_outranks_migration() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            health_failures: 5,
            migrate_to: Some(0),
            ..present()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Recover);
    }

    // ---- migration ----

    #[test]
    fn a_confirmed_better_endpoint_triggers_migration() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            migrate_to: Some(0),
            ..present()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Migrate(0));
    }

    // ---- manual unmount ----

    #[test]
    fn a_manually_unmounted_share_is_not_remounted() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            user_suppressed: true,
            ..absent()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Suppressed);
    }

    #[test]
    fn a_pending_retry_defers_the_attempt() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let view = MountView {
            retry_due: false,
            ..absent()
        };
        assert_eq!(decide(m, &ctx(true, true), &view), Decision::Backoff);
    }

    #[test]
    fn should_mount_but_no_endpoint_yet_is_waiting() {
        let cfg = cfg();
        let mut m = cfg.mount("nas").unwrap().clone();
        m.endpoints[0].requires = Some(CondRef::Dep("tailscale".into()));
        m.endpoints[1].requires = Some(CondRef::Dep("tailscale".into()));
        assert_eq!(decide(&m, &ctx(true, false), &absent()), Decision::Waiting);
    }

    #[test]
    fn unmount_when_gone_false_leaves_mount() {
        let cfg = cfg();
        let m = cfg.mount("pinned").unwrap();
        let view = MountView {
            gone_for: Some(Duration::from_secs(9999)),
            ..present()
        };
        assert_eq!(decide(m, &ctx(false, false), &view), Decision::Idle);
    }

    // ---- backoff ----

    #[test]
    fn backoff_doubles_to_cap() {
        let retry = Retry {
            start_secs: 2,
            max_secs: 10,
        };
        let b1 = next_backoff(None, &retry);
        assert_eq!(b1, Duration::from_secs(2));
        let b2 = next_backoff(Some(b1), &retry);
        assert_eq!(b2, Duration::from_secs(4));
        let b3 = next_backoff(Some(b2), &retry);
        assert_eq!(b3, Duration::from_secs(8));
        let b4 = next_backoff(Some(b3), &retry);
        assert_eq!(b4, Duration::from_secs(10)); // capped
        assert_eq!(next_backoff(Some(b4), &retry), Duration::from_secs(10));
    }

    #[test]
    fn backoff_saturates_instead_of_overflowing() {
        let retry = Retry {
            start_secs: 1,
            max_secs: 300,
        };
        let huge = Duration::from_secs(u64::MAX / 2 + 1);
        assert_eq!(next_backoff(Some(huge), &retry), Duration::from_secs(300));
    }

    #[test]
    fn world_fingerprint_changes_with_the_world() {
        let a = ctx(true, false).fingerprint();
        let b = ctx(true, true).fingerprint();
        let c = ctx(false, false).fingerprint();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a, ctx(true, false).fingerprint());
    }

    // ---- explain ----

    #[test]
    fn explain_reports_blocking_condition_per_endpoint() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        // away from home, tailscale up
        let ex = explain(m, &ctx(false, true), &absent());
        assert!(ex.should_mount);
        assert_eq!(ex.decision, "Attempt");
        assert_eq!(
            ex.when_any_of,
            vec![
                CondExplain {
                    cond: "network:home".into(),
                    holds: false
                },
                CondExplain {
                    cond: "dep:tailscale".into(),
                    holds: true
                },
            ]
        );
        assert!(!ex.endpoints[0].eligible);
        assert_eq!(ex.endpoints[0].blocked_by.as_deref(), Some("network:home"));
        assert!(ex.endpoints[1].eligible);
        assert!(ex.endpoints[1].blocked_by.is_none());
        assert!(ex.endpoints.iter().all(|e| e.reachable.is_none()));
    }

    #[test]
    fn explain_idle_when_nothing_holds() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let ex = explain(m, &ctx(false, false), &absent());
        assert!(!ex.should_mount);
        assert_eq!(ex.decision, "Idle");
    }

    #[test]
    fn explain_serializes_for_ipc() {
        let cfg = cfg();
        let m = cfg.mount("nas").unwrap();
        let ex = explain(m, &ctx(true, true), &present());
        let json = serde_json::to_string(&ex).unwrap();
        let back: Explanation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ex);
    }

    #[test]
    fn unknown_dep_reads_as_down() {
        let c = EvalCtx::default();
        assert!(!c.cond_holds(&CondRef::Dep("nope".into())));
        assert!(!c.cond_holds(&CondRef::Network("nope".into())));
    }
}
