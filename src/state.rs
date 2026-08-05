//! Per-mount lifecycle state.
//!
//! v0.3 kept a free-form status string, which meant every transition was
//! spelled out at its call site and nothing could be asserted on. The state is
//! now an enum; `Display` reproduces the exact strings the IPC clients already
//! match on (`mounted…`, `error…`, `waiting…`), so the wire format is
//! unchanged while the transitions become testable.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MountState {
    /// No reconcile pass has evaluated this mount yet.
    #[default]
    Unknown,
    /// Conditions do not hold and nothing is mounted.
    Idle,
    /// Conditions hold but no endpoint is eligible or reachable yet.
    Waiting { reason: String },
    /// A magic packet has been sent; the server is being given time to boot.
    Waking { address: String },
    /// Mounted by this daemon via a known endpoint.
    Mounted { via: String },
    /// Mounted, but not by us (Finder, or a mount that outlived a restart).
    Adopted { via: Option<String> },
    /// Still mounted while `when` no longer holds, because
    /// `policy.unmount_when_gone` is false.
    MountedPinned { via: Option<String> },
    /// Mounted, but the endpoint has stopped answering its probe.
    Suspect { via: String, failures: u32 },
    /// Tearing down a suspect mount before remounting it.
    Recovering { via: String },
    /// Moving to a higher-priority endpoint.
    Migrating { from: String, to: String },
    /// Not mounted, deliberately. `manual` means a user asked, so the
    /// reconciler leaves it alone until conditions change.
    Unmounted { manual: bool },
    /// Unmounted because `when` stopped holding.
    UnmountedConditionsGone,
    /// The last attempt failed; a retry is scheduled.
    Error { retry_secs: u64 },
}

impl MountState {
    /// Stable machine-readable discriminant for structured clients.
    pub fn kind(&self) -> &'static str {
        match self {
            MountState::Unknown => "unknown",
            MountState::Idle => "idle",
            MountState::Waiting { .. } => "waiting",
            MountState::Waking { .. } => "waking",
            MountState::Mounted { .. }
            | MountState::Adopted { .. }
            | MountState::MountedPinned { .. } => "mounted",
            MountState::Suspect { .. } => "suspect",
            MountState::Recovering { .. } => "recovering",
            MountState::Migrating { .. } => "migrating",
            MountState::Unmounted { .. } | MountState::UnmountedConditionsGone => "unmounted",
            MountState::Error { .. } => "error",
        }
    }

    /// True when a filesystem is expected to be present at the mountpoint.
    pub fn is_mounted(&self) -> bool {
        matches!(
            self,
            MountState::Mounted { .. }
                | MountState::Adopted { .. }
                | MountState::MountedPinned { .. }
                | MountState::Suspect { .. }
        )
    }
}

impl std::fmt::Display for MountState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountState::Unknown => write!(f, "unknown"),
            MountState::Idle => write!(f, "idle (conditions not met)"),
            MountState::Waiting { reason } => write!(f, "waiting ({reason})"),
            MountState::Waking { address } => write!(f, "waking ({address} sent wake-on-LAN)"),
            MountState::Mounted { via } => write!(f, "mounted via {via}"),
            MountState::Adopted { via: Some(v) } => write!(f, "mounted via {v} (adopted)"),
            MountState::Adopted { via: None } => write!(f, "mounted (adopted)"),
            MountState::MountedPinned { via: Some(v) } => {
                write!(f, "mounted via {v} (pinned; conditions not met)")
            }
            MountState::MountedPinned { via: None } => {
                write!(f, "mounted (pinned; conditions not met)")
            }
            MountState::Suspect { via, failures } => {
                write!(f, "suspect (via {via}, {failures} failed probes)")
            }
            MountState::Recovering { via } => write!(f, "recovering (was via {via})"),
            MountState::Migrating { from, to } => write!(f, "migrating ({from} -> {to})"),
            MountState::Unmounted { manual: true } => {
                write!(f, "unmounted (manual, until conditions change)")
            }
            MountState::Unmounted { manual: false } => write!(f, "unmounted"),
            MountState::UnmountedConditionsGone => write!(f, "unmounted (conditions gone)"),
            MountState::Error { retry_secs } => write!(f, "error (retry in {retry_secs}s)"),
        }
    }
}

/// Everything the engine remembers about one mount between passes.
#[derive(Debug, Default, Clone)]
pub struct MountRuntime {
    pub state: MountState,
    /// The endpoint address this mount is currently realized via.
    pub mounted_via: Option<String>,
    pub last_error: Option<String>,

    // retry
    pub backoff: Option<Duration>,
    pub next_retry_at: Option<Instant>,
    /// Fingerprint of the world when the backoff was set. When the world
    /// changes — a network appears, a dep comes up — the backoff no longer
    /// describes reality and is discarded.
    pub backoff_key: Option<u64>,

    // hysteresis
    /// When `when` first stopped holding. A mount is only removed once this
    /// has persisted for `policy.unmount_grace_secs`.
    pub gone_since: Option<Instant>,
    /// Whether `when` held on the previous pass, for edge detection.
    pub when_held: bool,

    // health
    pub health_failures: u32,
    pub last_health_check: Option<Instant>,

    // migration
    pub migrate_target: Option<usize>,
    pub migrate_streak: u32,

    // user intent and concurrency
    /// Set by an explicit unmount; cleared by an explicit mount, a reload, or
    /// a false -> true transition of `when`.
    pub user_suppressed: bool,
    /// An attempt is in flight for this mount; nothing else may start one.
    pub attempting: bool,

    // wake-on-lan
    pub last_wol_at: Option<Instant>,
}

impl MountRuntime {
    /// Forget any scheduled retry. Used when the world changes underneath a
    /// backoff, and on wake, where the monotonic deadline no longer means
    /// anything useful.
    pub fn clear_backoff(&mut self) {
        self.backoff = None;
        self.next_retry_at = None;
        self.backoff_key = None;
    }

    /// Record a successful mount.
    pub fn note_mounted(&mut self, address: &str) {
        self.mounted_via = Some(address.to_string());
        self.state = MountState::Mounted {
            via: address.to_string(),
        };
        self.last_error = None;
        self.gone_since = None;
        self.health_failures = 0;
        self.migrate_streak = 0;
        self.migrate_target = None;
        self.user_suppressed = false;
        self.clear_backoff();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_strings_keep_the_prefixes_clients_match_on() {
        // The Swift menu bar client switches on these prefixes; changing them
        // silently breaks its icon and glyph logic.
        assert!(MountState::Mounted {
            via: "10.0.0.1".into()
        }
        .to_string()
        .starts_with("mounted"));
        assert!(MountState::Adopted { via: None }
            .to_string()
            .starts_with("mounted"));
        assert!(MountState::MountedPinned { via: None }
            .to_string()
            .starts_with("mounted"));
        assert!(MountState::Error { retry_secs: 4 }
            .to_string()
            .starts_with("error"));
        assert!(MountState::Waiting {
            reason: "no endpoint reachable".into()
        }
        .to_string()
        .starts_with("waiting"));
    }

    #[test]
    fn legacy_strings_are_reproduced_exactly() {
        assert_eq!(MountState::Unknown.to_string(), "unknown");
        assert_eq!(MountState::Idle.to_string(), "idle (conditions not met)");
        assert_eq!(
            MountState::Waiting {
                reason: "no endpoint reachable".into()
            }
            .to_string(),
            "waiting (no endpoint reachable)"
        );
        assert_eq!(
            MountState::Mounted {
                via: "192.168.1.5".into()
            }
            .to_string(),
            "mounted via 192.168.1.5"
        );
        assert_eq!(
            MountState::Adopted { via: None }.to_string(),
            "mounted (adopted)"
        );
        assert_eq!(
            MountState::UnmountedConditionsGone.to_string(),
            "unmounted (conditions gone)"
        );
        assert_eq!(
            MountState::Error { retry_secs: 8 }.to_string(),
            "error (retry in 8s)"
        );
    }

    #[test]
    fn kinds_are_stable() {
        assert_eq!(
            MountState::Suspect {
                via: "a".into(),
                failures: 3
            }
            .kind(),
            "suspect"
        );
        assert_eq!(MountState::Unmounted { manual: true }.kind(), "unmounted");
        assert_eq!(
            MountState::MountedPinned { via: None }.kind(),
            "mounted",
            "a pinned mount is still mounted as far as clients are concerned"
        );
    }

    #[test]
    fn is_mounted_covers_suspect_but_not_recovering() {
        assert!(MountState::Suspect {
            via: "a".into(),
            failures: 1
        }
        .is_mounted());
        assert!(!MountState::Recovering { via: "a".into() }.is_mounted());
        assert!(!MountState::Idle.is_mounted());
    }

    #[test]
    fn note_mounted_clears_failure_bookkeeping() {
        let mut rt = MountRuntime {
            last_error: Some("boom".into()),
            backoff: Some(Duration::from_secs(60)),
            next_retry_at: Some(Instant::now()),
            backoff_key: Some(7),
            health_failures: 3,
            gone_since: Some(Instant::now()),
            user_suppressed: true,
            ..Default::default()
        };
        rt.note_mounted("10.0.0.1");
        assert_eq!(rt.mounted_via.as_deref(), Some("10.0.0.1"));
        assert!(rt.last_error.is_none());
        assert!(rt.backoff.is_none());
        assert!(rt.next_retry_at.is_none());
        assert!(rt.backoff_key.is_none());
        assert_eq!(rt.health_failures, 0);
        assert!(rt.gone_since.is_none());
        assert!(!rt.user_suppressed);
    }
}
