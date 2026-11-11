//! Declarative TOML configuration: networks (fingerprints), deps (wait-until
//! predicates), and mounts with ordered endpoint priority.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::str::FromStr;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("mount `{mount}` references unknown {kind} `{name}`")]
    UnknownRef {
        mount: String,
        kind: &'static str,
        name: String,
    },
    #[error("duplicate mount name `{0}`")]
    DuplicateMount(String),
    #[error("mount `{0}` has no endpoints")]
    NoEndpoints(String),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("mount `{mount}`: {msg}")]
    Invalid { mount: String, msg: String },
    #[error("network `{network}`: {msg}")]
    InvalidNetwork { network: String, msg: String },
    #[error("mounts `{a}` and `{b}` share the mountpoint `{path}`")]
    MountpointCollision { a: String, b: String, path: String },
    #[error("mount `{inner}` (`{inner_path}`) is nested inside mount `{outer}` (`{outer_path}`)")]
    MountpointNesting {
        inner: String,
        inner_path: String,
        outer: String,
        outer_path: String,
    },
}

/// An IPv4 CIDR block, e.g. `192.168.1.0/24`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    pub network: Ipv4Addr,
    pub prefix: u8,
}

impl Cidr {
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        if self.prefix == 0 {
            return true;
        }
        let mask = u32::MAX << (32 - u32::from(self.prefix));
        (u32::from(self.network) & mask) == (u32::from(ip) & mask)
    }
}

#[derive(Debug, Error)]
#[error("invalid CIDR `{0}`")]
pub struct CidrParseError(String);

impl FromStr for Cidr {
    type Err = CidrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, prefix) = s.split_once('/').ok_or_else(|| CidrParseError(s.into()))?;
        let network: Ipv4Addr = addr.parse().map_err(|_| CidrParseError(s.into()))?;
        let prefix: u8 = prefix.parse().map_err(|_| CidrParseError(s.into()))?;
        if prefix > 32 {
            return Err(CidrParseError(s.into()));
        }
        Ok(Cidr { network, prefix })
    }
}

impl<'de> Deserialize<'de> for Cidr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Normalize a MAC address: lowercase, each octet zero-padded to two digits.
/// macOS `arp` prints octets without leading zeros (`aa:bb:c:d:ee:ff`); vendor
/// documentation often uses dashes, so both separators are accepted.
pub fn normalize_mac(mac: &str) -> Option<String> {
    let octets: Vec<String> = mac
        .trim()
        .split([':', '-'])
        .map(|o| u8::from_str_radix(o, 16).map(|b| format!("{b:02x}")))
        .collect::<Result<_, _>>()
        .ok()?;
    if octets.len() != 6 {
        return None;
    }
    Some(octets.join(":"))
}

/// A reference to a condition: `network:<name>` or `dep:<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondRef {
    Network(String),
    Dep(String),
}

impl std::fmt::Display for CondRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CondRef::Network(n) => write!(f, "network:{n}"),
            CondRef::Dep(n) => write!(f, "dep:{n}"),
        }
    }
}

impl FromStr for CondRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once(':') {
            Some(("network", n)) if !n.is_empty() => Ok(CondRef::Network(n.to_string())),
            Some(("dep", n)) if !n.is_empty() => Ok(CondRef::Dep(n.to_string())),
            _ => Err(format!(
                "invalid condition `{s}` (expected `network:<name>` or `dep:<name>`)"
            )),
        }
    }
}

impl<'de> Deserialize<'de> for CondRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A reachability probe, currently only `tcp:<port>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Tcp(u16),
}

impl FromStr for Probe {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once(':') {
            Some(("tcp", p)) => match p.parse() {
                // port 0 would make the endpoint permanently unreachable with
                // nothing but a "waiting" state to show for it
                Ok(0) | Err(_) => Err(format!("invalid probe port in `{s}`")),
                Ok(port) => Ok(Probe::Tcp(port)),
            },
            _ => Err(format!("invalid probe `{s}` (expected `tcp:<port>`)")),
        }
    }
}

impl<'de> Deserialize<'de> for Probe {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

fn default_probe_timeout_ms() -> u64 {
    1500
}
fn default_debounce_secs() -> u64 {
    3
}
fn default_poll_interval_secs() -> u64 {
    5
}
fn default_mount_root() -> String {
    "~/mnt".to_string()
}
fn default_wake_settle_secs() -> u64 {
    20
}
fn default_health_interval_secs() -> u64 {
    30
}
fn default_sleep_unmount_budget_secs() -> u64 {
    8
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "default_probe_timeout_ms")]
    pub probe_timeout_ms: u64,
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u64,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Mountpoints given as a bare name (no `/`) resolve against this root.
    #[serde(default = "default_mount_root")]
    pub mount_root: String,
    /// After waking, conditions are treated as untrustworthy for this long:
    /// the network has not finished coming up, and acting on a false reading
    /// force-unmounts healthy shares.
    #[serde(default = "default_wake_settle_secs")]
    pub wake_settle_secs: u64,
    /// How often a mounted share's endpoint is re-probed for liveness.
    #[serde(default = "default_health_interval_secs")]
    pub health_interval_secs: u64,
    /// Cleanup budget before sleep, at most 20 seconds. The callback has its
    /// own 25-second ceiling so a stalled engine cannot block acknowledgement.
    #[serde(default = "default_sleep_unmount_budget_secs")]
    pub sleep_unmount_budget_secs: u64,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            probe_timeout_ms: default_probe_timeout_ms(),
            debounce_secs: default_debounce_secs(),
            poll_interval_secs: default_poll_interval_secs(),
            mount_root: default_mount_root(),
            wake_settle_secs: default_wake_settle_secs(),
            health_interval_secs: default_health_interval_secs(),
            sleep_unmount_budget_secs: default_sleep_unmount_budget_secs(),
        }
    }
}

/// A network identity fingerprint. A network matches when any local interface
/// address falls inside `subnet` AND, if given, the default gateway's MAC
/// equals `gateway_mac`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkFingerprint {
    pub subnet: Cidr,
    #[serde(default)]
    pub gateway_mac: Option<String>,
}

fn default_dep_timeout_secs() -> u64 {
    10
}

/// A "wait-until" predicate: run `cmd args`, optionally check a JSON field of
/// its stdout. The dep is up when the command exits 0 and (if configured) the
/// value at `json_path` equals `equals`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepCheck {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub json_path: Option<String>,
    #[serde(default)]
    pub equals: Option<String>,
    #[serde(default = "default_dep_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Smb,
    Nfs,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhenClause {
    #[serde(default)]
    pub any_of: Vec<CondRef>,
    #[serde(default)]
    pub all_of: Vec<CondRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    pub probe: Probe,
}

fn default_wol_wait_secs() -> u64 {
    45
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub address: String,
    #[serde(default)]
    pub only_if: Option<CondRef>,
    #[serde(default)]
    pub requires: Option<CondRef>,
    #[serde(default)]
    pub probe_timeout_ms: Option<u64>,
    /// Wake this host with a magic packet when its probe fails.
    #[serde(default)]
    pub wake_on_lan: Option<crate::wol::WolSpec>,
    /// How long to give the server to boot after a magic packet before
    /// sending another.
    #[serde(default = "default_wol_wait_secs")]
    pub wol_wait_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_retry_start_secs() -> u64 {
    2
}
fn default_retry_max_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    #[serde(default = "default_retry_start_secs")]
    pub start_secs: u64,
    #[serde(default = "default_retry_max_secs")]
    pub max_secs: u64,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            start_secs: default_retry_start_secs(),
            max_secs: default_retry_max_secs(),
        }
    }
}

fn default_unmount_grace_secs() -> u64 {
    30
}
fn default_health_failures() -> u32 {
    3
}
fn default_migrate_confirmations() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default = "default_true")]
    pub unmount_when_gone: bool,
    /// How long `when` must stay false before the mount is actually removed.
    /// A single bad sample — a slow `tailscale status`, an ARP cache that has
    /// not repopulated after wake — must never cost a healthy mount.
    #[serde(default = "default_unmount_grace_secs")]
    pub unmount_grace_secs: u64,
    /// Consecutive failed liveness probes before a mounted share is declared
    /// suspect and recovered.
    #[serde(default = "default_health_failures")]
    pub health_failures: u32,
    /// Mount without a password when the Keychain lookup fails. Off by
    /// default: against a server with guest access this silently mounts as
    /// nobody.
    #[serde(default)]
    pub allow_passwordless: bool,
    /// Move an existing mount to a higher-priority endpoint when one becomes
    /// reachable. Off by default — it costs an unmount/remount cycle.
    #[serde(default)]
    pub migrate: bool,
    /// Consecutive successful probes of the better endpoint before migrating.
    #[serde(default = "default_migrate_confirmations")]
    pub migrate_confirmations: u32,
    /// Unmount cleanly when the system is about to sleep.
    #[serde(default = "default_true")]
    pub unmount_before_sleep: bool,
    #[serde(default)]
    pub retry: Retry,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            unmount_when_gone: true,
            unmount_grace_secs: default_unmount_grace_secs(),
            health_failures: default_health_failures(),
            allow_passwordless: false,
            migrate: false,
            migrate_confirmations: default_migrate_confirmations(),
            unmount_before_sleep: true,
            retry: Retry::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountSpec {
    pub name: String,
    pub protocol: Protocol,
    /// SMB share name (`smb` only).
    #[serde(default)]
    pub share: Option<String>,
    /// NFS export path (`nfs` only).
    #[serde(default)]
    pub export: Option<String>,
    /// SMB username; the password is fetched from the login Keychain.
    #[serde(default)]
    pub username: Option<String>,
    /// Keychain item server attribute to fetch the password from. Defaults to
    /// the endpoint address being mounted; set it so one credential item
    /// covers every endpoint (LAN IP, tailnet IP, hostname) of this mount.
    #[serde(default)]
    pub keychain_server: Option<String>,
    pub mountpoint: String,
    /// Extra `-o` mount options, merged with required nonblocking defaults.
    #[serde(default)]
    pub options: Option<String>,
    #[serde(default)]
    pub when: WhenClause,
    #[serde(default)]
    pub gate: Option<Gate>,
    #[serde(default, rename = "endpoint")]
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub policy: Policy,
}

impl MountSpec {
    /// The probe used to gate mount attempts: explicit gate, else the
    /// protocol's service port (SMB 445, NFS 2049).
    pub fn effective_probe(&self) -> Probe {
        match &self.gate {
            Some(g) => g.probe,
            None => match self.protocol {
                Protocol::Smb => Probe::Tcp(445),
                Protocol::Nfs => Probe::Tcp(2049),
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub networks: BTreeMap<String, NetworkFingerprint>,
    #[serde(default)]
    pub deps: BTreeMap<String, DepCheck>,
    #[serde(default, rename = "mount")]
    pub mounts: Vec<MountSpec>,
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Config, ConfigError> {
        let mut cfg: Config = toml::from_str(s)?;
        for (name, net) in cfg.networks.iter_mut() {
            if let Some(mac) = &net.gateway_mac {
                let norm = normalize_mac(mac).ok_or_else(|| ConfigError::InvalidNetwork {
                    network: name.clone(),
                    msg: format!("invalid gateway_mac `{mac}`"),
                })?;
                net.gateway_mac = Some(norm);
            }
        }
        cfg.resolve_mountpoints();
        cfg.validate()?;
        Ok(cfg)
    }

    /// A mountpoint given as a bare name (no `/`) is resolved against
    /// `defaults.mount_root`, which is what the documented knob has always
    /// implied.
    fn resolve_mountpoints(&mut self) {
        let root = self.defaults.mount_root.trim_end_matches('/').to_string();
        for m in &mut self.mounts {
            if !m.mountpoint.contains('/') && !m.mountpoint.is_empty() && m.mountpoint != "~" {
                m.mountpoint = format!("{}/{}", root, m.mountpoint);
            }
        }
    }

    pub fn mount(&self, name: &str) -> Option<&MountSpec> {
        self.mounts.iter().find(|m| m.name == name)
    }

    fn check_ref(&self, mount: &str, r: &CondRef) -> Result<(), ConfigError> {
        let (kind, name, ok) = match r {
            CondRef::Network(n) => ("network", n, self.networks.contains_key(n)),
            CondRef::Dep(n) => ("dep", n, self.deps.contains_key(n)),
        };
        if ok {
            Ok(())
        } else {
            Err(ConfigError::UnknownRef {
                mount: mount.to_string(),
                kind,
                name: name.clone(),
            })
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let d = &self.defaults;
        for (name, value, min, max) in [
            ("probe_timeout_ms", d.probe_timeout_ms, 1, 120_000),
            ("poll_interval_secs", d.poll_interval_secs, 1, 86_400),
            ("debounce_secs", d.debounce_secs, 0, 86_400),
            ("wake_settle_secs", d.wake_settle_secs, 0, 86_400),
            ("health_interval_secs", d.health_interval_secs, 0, 86_400),
            (
                "sleep_unmount_budget_secs",
                d.sleep_unmount_budget_secs,
                0,
                20,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(ConfigError::InvalidConfig(format!(
                    "defaults.{name} must be between {min} and {max}"
                )));
            }
        }
        let root = &d.mount_root;
        if !(root.starts_with('/') || root == "~" || root.starts_with("~/"))
            || root.contains('\0')
            || root.split('/').any(|p| p == "." || p == "..")
        {
            return Err(ConfigError::InvalidConfig(
                "mount_root must be absolute or start with ~/ and contain no dot components or NUL"
                    .into(),
            ));
        }
        for (name, dep) in &self.deps {
            if dep.cmd.trim().is_empty()
                || dep.cmd.contains('\0')
                || dep.args.iter().any(|a| a.contains('\0'))
                || dep.timeout_secs == 0
                || dep.timeout_secs > 120
            {
                return Err(ConfigError::InvalidConfig(format!("dep `{name}` requires a nonempty command, NUL-free arguments and timeout_secs between 1 and 120")));
            }
            if dep.equals.is_some() && dep.json_path.is_none() {
                return Err(ConfigError::InvalidConfig(format!(
                    "dep `{name}`: equals requires json_path"
                )));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for m in &self.mounts {
            let invalid = |msg: &str| ConfigError::Invalid {
                mount: m.name.clone(),
                msg: msg.into(),
            };
            if m.name.trim().is_empty() || m.name.chars().any(char::is_control) {
                return Err(invalid(
                    "name must be nonempty and contain no control characters",
                ));
            }
            if m.policy.unmount_grace_secs > 86_400
                || m.policy.health_failures == 0
                || m.policy.migrate_confirmations == 0
            {
                return Err(invalid("unmount_grace_secs must be at most 86400 and health_failures and migrate_confirmations must be positive"));
            }
            if [
                m.share.as_deref(),
                m.export.as_deref(),
                m.username.as_deref(),
                m.keychain_server.as_deref(),
                m.options.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|s| s.is_empty() || s.contains('\0'))
            {
                return Err(invalid(
                    "optional mount strings must be nonempty and NUL-free when supplied",
                ));
            }
            if m.protocol == Protocol::Nfs
                && m.export.as_deref().is_some_and(|s| !s.starts_with('/'))
            {
                return Err(invalid("NFS export must be absolute"));
            }
            if !seen.insert(m.name.clone()) {
                return Err(ConfigError::DuplicateMount(m.name.clone()));
            }
            if m.endpoints.is_empty() {
                return Err(ConfigError::NoEndpoints(m.name.clone()));
            }
            match m.protocol {
                Protocol::Smb if m.share.is_none() => {
                    return Err(ConfigError::Invalid {
                        mount: m.name.clone(),
                        msg: "smb mount requires `share`".into(),
                    });
                }
                Protocol::Nfs if m.export.is_none() => {
                    return Err(ConfigError::Invalid {
                        mount: m.name.clone(),
                        msg: "nfs mount requires `export`".into(),
                    });
                }
                _ => {}
            }
            crate::mountcmd::validated_options(m).map_err(|error| invalid(&error.to_string()))?;
            for r in m.when.any_of.iter().chain(&m.when.all_of) {
                self.check_ref(&m.name, r)?;
            }
            for ep in &m.endpoints {
                if ep.probe_timeout_ms.is_some_and(|v| v == 0 || v > 120_000)
                    || ep.wol_wait_secs > 86_400
                {
                    return Err(invalid("endpoint probe_timeout_ms must be between 1 and 120000 and wol_wait_secs at most 86400"));
                }
                // A probe and the mount command must identify the same host.
                if ep.address.is_empty()
                    || ep.address.chars().any(|c| {
                        c.is_whitespace()
                            || c.is_control()
                            || matches!(c, '/' | '\\' | '@' | '?' | '#' | '%' | '[' | ']')
                    })
                    || (ep.address.contains(':')
                        && ep.address.parse::<std::net::Ipv6Addr>().is_err())
                {
                    return Err(invalid(
                        "endpoint address must be a bare hostname or IP address",
                    ));
                }
                if let Some(r) = &ep.only_if {
                    self.check_ref(&m.name, r)?;
                }
                if let Some(r) = &ep.requires {
                    self.check_ref(&m.name, r)?;
                    if let CondRef::Network(n) = r {
                        return Err(ConfigError::Invalid {
                            mount: m.name.clone(),
                            msg: format!(
                                "endpoint `requires` must be a dep, got `network:{n}` \
                                 (use `only_if` for network conditions)"
                            ),
                        });
                    }
                }
                if let Some(wol) = &ep.wake_on_lan {
                    if wol.port == 0 {
                        return Err(invalid("wake_on_lan port must be nonzero"));
                    }
                    if normalize_mac(&wol.mac).is_none() {
                        return Err(ConfigError::Invalid {
                            mount: m.name.clone(),
                            msg: format!("invalid wake_on_lan MAC `{}`", wol.mac),
                        });
                    }
                    if wol.broadcast.parse::<Ipv4Addr>().is_err() {
                        return Err(ConfigError::Invalid {
                            mount: m.name.clone(),
                            msg: format!(
                                "invalid wake_on_lan broadcast address `{}`",
                                wol.broadcast
                            ),
                        });
                    }
                }
                if ep.address.starts_with('-') {
                    return Err(ConfigError::Invalid {
                        mount: m.name.clone(),
                        msg: format!(
                            "endpoint address `{}` starts with `-` and would be \
                             read as a command-line flag",
                            ep.address
                        ),
                    });
                }
            }
            self.check_mountpoint(m)?;
            self.check_retry(m)?;
        }
        self.check_mountpoint_overlap()?;
        Ok(())
    }

    /// A mountpoint must be absolute (or `~`-rooted). A relative path behaves
    /// completely differently depending on the daemon's working directory:
    /// under launchd it is `/`, so every mount attempt fails, while a terminal
    /// run from `$HOME` silently creates a junk directory and appears to work.
    fn check_mountpoint(&self, m: &MountSpec) -> Result<(), ConfigError> {
        let mp = &m.mountpoint;
        if mp.is_empty() {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: "mountpoint is empty".into(),
            });
        }
        if mp.split('/').any(|part| part == ".." || part == ".") || mp.contains('\0') {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: "mountpoint must not contain dot components or NUL".into(),
            });
        }
        let normalized = crate::mountcmd::normalize_mountpoint(mp);
        if normalized == std::path::Path::new("/")
            || dirs::home_dir().is_some_and(|home| normalized == home)
            || mp.trim_end_matches('/').is_empty()
            || mp.trim_end_matches('/') == "~"
        {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: "mountpoint must not cover the filesystem root or home directory".into(),
            });
        }
        if !(mp.starts_with('/') || mp == "~" || mp.starts_with("~/")) {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: format!(
                    "mountpoint `{mp}` must be absolute or start with `~/` \
                     (a relative path resolves against the daemon's working directory)"
                ),
            });
        }
        Ok(())
    }

    /// Unbounded retry values turn one failed mount into a panic in the
    /// reconcile path: the backoff doubles until `Duration` or `Instant`
    /// arithmetic overflows.
    fn check_retry(&self, m: &MountSpec) -> Result<(), ConfigError> {
        const MAX_RETRY_SECS: u64 = 86_400;
        let r = &m.policy.retry;
        if r.max_secs == 0 || r.max_secs > MAX_RETRY_SECS {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: format!("policy.retry.max_secs must be between 1 and {MAX_RETRY_SECS}"),
            });
        }
        if r.start_secs == 0 || r.start_secs > r.max_secs {
            return Err(ConfigError::Invalid {
                mount: m.name.clone(),
                msg: "policy.retry.start_secs must be between 1 and max_secs".into(),
            });
        }
        Ok(())
    }

    /// Two mounts sharing a mountpoint make each read as mounted when only one
    /// is; when one's conditions lapse it unmounts the other's volume, and the
    /// pair oscillates. Nesting has the same problem on unmount ordering.
    fn check_mountpoint_overlap(&self) -> Result<(), ConfigError> {
        let paths: Vec<(String, String)> = self
            .mounts
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    crate::mountcmd::normalize_mountpoint(&m.mountpoint)
                        .to_string_lossy()
                        .trim_end_matches('/')
                        .to_string(),
                )
            })
            .collect();

        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                let (a, pa) = &paths[i];
                let (b, pb) = &paths[j];
                if crate::mountcmd::paths_may_alias(
                    std::path::Path::new(pa),
                    std::path::Path::new(pb),
                ) {
                    return Err(ConfigError::MountpointCollision {
                        a: a.clone(),
                        b: b.clone(),
                        path: pa.clone(),
                    });
                }
                if let Some((outer, outer_path, inner, inner_path)) = nested_pair(a, pa, b, pb) {
                    return Err(ConfigError::MountpointNesting {
                        inner: inner.to_string(),
                        inner_path: inner_path.to_string(),
                        outer: outer.to_string(),
                        outer_path: outer_path.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// If one path is a proper ancestor of the other, return (outer, outer_path,
/// inner, inner_path). Compares whole path components so `/a/bc` is not
/// treated as nested inside `/a/b`.
fn nested_pair<'a>(
    a: &'a str,
    pa: &'a str,
    b: &'a str,
    pb: &'a str,
) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
    let is_ancestor = |outer: &str, inner: &str| {
        let outer = std::path::Path::new(outer);
        let inner = std::path::Path::new(inner);
        inner.components().count() > outer.components().count()
            && crate::mountcmd::path_may_be_beneath(inner, outer)
    };
    if is_ancestor(pa, pb) {
        Some((a, pa, b, pb))
    } else if is_ancestor(pb, pa) {
        Some((b, pb, a, pa))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[defaults]
probe_timeout_ms = 1500
debounce_secs = 3

[networks.home]
subnet      = "192.168.1.0/24"
gateway_mac = "AA:BB:C:D:EE:FF"

[deps.tailscale]
cmd       = "/Applications/Tailscale.app/Contents/MacOS/Tailscale"
args      = ["status", "--json"]
json_path = "BackendState"
equals    = "Running"

[[mount]]
name       = "nas-media"
protocol   = "smb"
share      = "media"
username   = "chris"
mountpoint = "~/mnt/nas-media"

  [mount.when]
  any_of = ["network:home", "dep:tailscale"]

  [mount.gate]
  probe = "tcp:445"

  [[mount.endpoint]]
  address = "192.168.1.5"
  only_if = "network:home"

  [[mount.endpoint]]
  address          = "100.64.0.10"
  requires         = "dep:tailscale"
  probe_timeout_ms = 4000

[[mount]]
name       = "nas-scratch"
protocol   = "nfs"
export     = "/mnt/tank/scratch"
mountpoint = "~/mnt/nas-scratch"
options    = "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp"

  [mount.when]
  all_of = ["dep:tailscale"]

  [[mount.endpoint]]
  address  = "100.64.0.11"
  requires = "dep:tailscale"
"#;

    #[test]
    fn rejects_mountpoint_case_and_unicode_aliases() {
        for (a, b) in [
            ("/local/NAS", "/local/nas"),
            ("/local/NAS", "/LOCAL/nas/child"),
            ("/local/café", "/local/cafe\u{301}"),
        ] {
            let mut cfg = Config::from_toml_str(EXAMPLE).unwrap();
            cfg.mounts[0].mountpoint = a.into();
            cfg.mounts[1].mountpoint = b.into();
            assert!(cfg.validate().is_err(), "{a} and {b}");
        }
    }

    #[test]
    fn rejects_invalid_timers_and_unsafe_mount_values() {
        for (from, to) in [
            ("probe_timeout_ms = 1500", "probe_timeout_ms = 0"),
            ("debounce_secs = 3", "debounce_secs = 9223372036854775807"),
            (
                "mountpoint = \"~/mnt/nas-media\"",
                "mountpoint = \"/tmp/../mnt/share\"",
            ),
            ("mountpoint = \"~/mnt/nas-media\"", "mountpoint = \"/\""),
            ("share      = \"media\"", "share      = \"\""),
            ("address = \"192.168.1.5\"", "address = \"server/share\""),
            ("probe_timeout_ms = 4000", "probe_timeout_ms = 0"),
        ] {
            assert!(EXAMPLE.contains(from), "{from}");
            assert!(
                Config::from_toml_str(&EXAMPLE.replace(from, to)).is_err(),
                "accepted {to}"
            );
        }
        assert!(Config::from_toml_str("[defaults]\nmount_root = \"\"").is_err());
        assert!(Config::from_toml_str("[deps.x]\ncmd = \"fake\"\ntimeout_secs = 0").is_err());
    }

    #[test]
    fn rejects_home_mountpoints_and_ignored_dependency_comparisons() {
        for mountpoint in [
            "~".to_string(),
            "~/".into(),
            dirs::home_dir().unwrap().display().to_string(),
        ] {
            let bad = EXAMPLE.replace("~/mnt/nas-media", &mountpoint);
            assert!(
                Config::from_toml_str(&bad).is_err(),
                "accepted {mountpoint}"
            );
        }
        assert!(Config::from_toml_str("[deps.x]\ncmd = \"fake\"\nequals = \"running\"").is_err());
    }

    #[test]
    fn config_rejects_mount_options_that_escape_the_probe_and_timeout_contract() {
        let old = "options    = \"soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp\"";
        for options in [
            "bg,hard",
            "udp",
            "proto=udp6",
            "localhost",
            "inet",
            "inet6",
            "noinet",
            "noinet6",
            "netid=tcp6",
            "proto=tcp",
            "mntudp",
            "port=9999",
            "mountport=9999",
            "nointr",
            "retrycnt=99",
        ] {
            let bad = EXAMPLE.replace(old, &format!("options = {options:?}"));
            assert!(Config::from_toml_str(&bad).is_err(), "accepted {options}");
        }
        let good = EXAMPLE.replace(old, "options = \"vers=4,ro\"");
        assert!(Config::from_toml_str(&good).is_ok());
    }

    #[test]
    fn parses_full_example() {
        let cfg = Config::from_toml_str(EXAMPLE).unwrap();
        assert_eq!(cfg.defaults.probe_timeout_ms, 1500);
        assert_eq!(cfg.defaults.poll_interval_secs, 5); // default applied
        assert_eq!(cfg.mounts.len(), 2);

        let nas = cfg.mount("nas-media").unwrap();
        assert_eq!(nas.protocol, Protocol::Smb);
        assert_eq!(nas.endpoints.len(), 2);
        assert_eq!(nas.endpoints[0].address, "192.168.1.5");
        assert_eq!(
            nas.endpoints[0].only_if,
            Some(CondRef::Network("home".into()))
        );
        assert_eq!(
            nas.endpoints[1].requires,
            Some(CondRef::Dep("tailscale".into()))
        );
        assert_eq!(nas.endpoints[1].probe_timeout_ms, Some(4000));
        assert_eq!(nas.effective_probe(), Probe::Tcp(445));
        assert!(nas.policy.unmount_when_gone); // default

        let scratch = cfg.mount("nas-scratch").unwrap();
        assert_eq!(scratch.effective_probe(), Probe::Tcp(2049)); // nfs default gate
        assert_eq!(scratch.when.all_of, vec![CondRef::Dep("tailscale".into())]);
    }

    #[test]
    fn normalizes_gateway_mac() {
        let cfg = Config::from_toml_str(EXAMPLE).unwrap();
        assert_eq!(
            cfg.networks["home"].gateway_mac.as_deref(),
            Some("aa:bb:0c:0d:ee:ff")
        );
    }

    #[test]
    fn cidr_contains() {
        let cidr: Cidr = "192.168.1.0/24".parse().unwrap();
        assert!(cidr.contains("192.168.1.55".parse().unwrap()));
        assert!(!cidr.contains("192.168.11.1".parse().unwrap()));
        let all: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains("8.8.8.8".parse().unwrap()));
        assert!("300.1.1.1/24".parse::<Cidr>().is_err());
        assert!("192.168.1.0/33".parse::<Cidr>().is_err());
        assert!("192.168.1.0".parse::<Cidr>().is_err());
    }

    #[test]
    fn normalize_mac_pads_and_lowercases() {
        assert_eq!(
            normalize_mac("AA:BB:C:D:EE:FF").as_deref(),
            Some("aa:bb:0c:0d:ee:ff")
        );
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee").as_deref(), None);
        assert_eq!(normalize_mac("zz:bb:cc:dd:ee:ff").as_deref(), None);
    }

    #[test]
    fn cond_ref_parsing() {
        assert_eq!(
            "network:home".parse::<CondRef>().unwrap(),
            CondRef::Network("home".into())
        );
        assert_eq!(
            "dep:tailscale".parse::<CondRef>().unwrap(),
            CondRef::Dep("tailscale".into())
        );
        assert!("home".parse::<CondRef>().is_err());
        assert!("network:".parse::<CondRef>().is_err());
    }

    #[test]
    fn rejects_unknown_network_ref() {
        let bad = EXAMPLE.replace("network:home", "network:office");
        let err = Config::from_toml_str(&bad).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownRef { .. }), "{err}");
    }

    #[test]
    fn rejects_duplicate_mount_names() {
        let bad = EXAMPLE.replace("nas-scratch", "nas-media");
        let err = Config::from_toml_str(&bad).unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateMount(_)), "{err}");
    }

    #[test]
    fn rejects_mount_without_endpoints() {
        let toml = r#"
[[mount]]
name = "x"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/x"
"#;
        let err = Config::from_toml_str(toml).unwrap_err();
        assert!(matches!(err, ConfigError::NoEndpoints(_)), "{err}");
    }

    #[test]
    fn rejects_smb_without_share() {
        let toml = r#"
[[mount]]
name = "x"
protocol = "smb"
mountpoint = "~/mnt/x"

  [[mount.endpoint]]
  address = "10.0.0.1"
"#;
        let err = Config::from_toml_str(toml).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
    }

    /// A minimal valid config with the given mount bodies spliced in.
    fn with_mounts(bodies: &str) -> Result<Config, ConfigError> {
        Config::from_toml_str(bodies)
    }

    #[test]
    fn rejects_two_mounts_sharing_a_mountpoint() {
        // Whichever mounts first makes the other read as mounted; when the
        // second one's conditions lapse it unmounts the first one's volume.
        let err = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-collide"
  [[mount.endpoint]]
  address = "10.0.0.1"

[[mount]]
name = "b"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-collide/"
  [[mount.endpoint]]
  address = "10.0.0.2"
"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConfigError::MountpointCollision { .. }),
            "{err}"
        );
    }

    #[test]
    fn rejects_nested_mountpoints() {
        let err = with_mounts(
            r#"
[[mount]]
name = "outer"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-nest"
  [[mount.endpoint]]
  address = "10.0.0.1"

[[mount]]
name = "inner"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-nest/deeper"
  [[mount.endpoint]]
  address = "10.0.0.2"
"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConfigError::MountpointNesting { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_shared_prefix_is_not_nesting() {
        // /tmp/mountd-ab must not count as nested inside /tmp/mountd-a.
        let cfg = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [[mount.endpoint]]
  address = "10.0.0.1"

[[mount]]
name = "ab"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-ab"
  [[mount.endpoint]]
  address = "10.0.0.2"
"#,
        );
        assert!(cfg.is_ok(), "{:?}", cfg.err());
    }

    #[test]
    fn rejects_a_relative_mountpoint() {
        // Under launchd the working directory is `/`, so a relative path fails
        // every attempt; run from a terminal it silently makes a junk dir.
        let err = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "~mnt/typo"
  [[mount.endpoint]]
  address = "10.0.0.1"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
        assert!(err.to_string().contains("absolute"), "{err}");
    }

    #[test]
    fn a_bare_mountpoint_name_resolves_against_mount_root() {
        let cfg = with_mounts(
            r#"
[defaults]
mount_root = "/tmp/mountd-root"

[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "media"
  [[mount.endpoint]]
  address = "10.0.0.1"
"#,
        )
        .unwrap();
        assert_eq!(cfg.mounts[0].mountpoint, "/tmp/mountd-root/media");
    }

    #[test]
    fn rejects_unbounded_retry_values() {
        // Doubling an enormous backoff overflows Duration/Instant arithmetic
        // inside the reconcile path.
        let err = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [mount.policy.retry]
  start_secs = 2
  max_secs = 18446744073709551615
  [[mount.endpoint]]
  address = "10.0.0.1"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("max_secs"), "{err}");
    }

    #[test]
    fn rejects_a_start_longer_than_the_cap() {
        let err = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [mount.policy.retry]
  start_secs = 600
  max_secs = 300
  [[mount.endpoint]]
  address = "10.0.0.1"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("start_secs"), "{err}");
    }

    #[test]
    fn rejects_probe_port_zero() {
        assert!("tcp:0".parse::<Probe>().is_err());
        assert!("tcp:445".parse::<Probe>().is_ok());
        assert!("tcp:notaport".parse::<Probe>().is_err());
        assert!("udp:53".parse::<Probe>().is_err());
    }

    #[test]
    fn rejects_an_endpoint_address_that_looks_like_a_flag() {
        let err = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [[mount.endpoint]]
  address = "-o"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("flag"), "{err}");
    }

    #[test]
    fn validates_wake_on_lan_settings() {
        let bad_mac = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [[mount.endpoint]]
  address = "10.0.0.1"
  [mount.endpoint.wake_on_lan]
  mac = "not-a-mac"
"#,
        )
        .unwrap_err();
        assert!(bad_mac.to_string().contains("MAC"), "{bad_mac}");

        let bad_bcast = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [[mount.endpoint]]
  address = "10.0.0.1"
  [mount.endpoint.wake_on_lan]
  mac = "aa:bb:cc:dd:ee:ff"
  broadcast = "nope"
"#,
        )
        .unwrap_err();
        assert!(bad_bcast.to_string().contains("broadcast"), "{bad_bcast}");

        let good = with_mounts(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "/tmp/mountd-a"
  [[mount.endpoint]]
  address = "10.0.0.1"
  wol_wait_secs = 60
  [mount.endpoint.wake_on_lan]
  mac = "AA-BB-CC-DD-EE-FF"
  broadcast = "10.0.0.255"
  port = 7
"#,
        )
        .unwrap();
        let wol = good.mounts[0].endpoints[0].wake_on_lan.as_ref().unwrap();
        assert_eq!(wol.port, 7);
        assert_eq!(good.mounts[0].endpoints[0].wol_wait_secs, 60);
    }

    #[test]
    fn a_bad_gateway_mac_names_the_network_not_a_mount() {
        let err = Config::from_toml_str(
            r#"
[networks.office]
subnet = "10.0.0.0/24"
gateway_mac = "aa:bb:cc:dd:ee"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidNetwork { .. }), "{err}");
        assert!(err.to_string().contains("office"), "{err}");
    }

    #[test]
    fn new_policy_defaults_are_conservative() {
        let cfg = Config::from_toml_str(EXAMPLE).unwrap();
        let p = &cfg.mount("nas-media").unwrap().policy;
        assert_eq!(p.unmount_grace_secs, 30, "never unmount on one bad sample");
        assert_eq!(p.health_failures, 3);
        assert!(!p.allow_passwordless, "a guest mount must be opt-in");
        assert!(!p.migrate, "migration costs an unmount/remount");
        assert!(p.unmount_before_sleep);
        assert_eq!(cfg.defaults.wake_settle_secs, 20);
        assert_eq!(cfg.defaults.health_interval_secs, 30);
    }

    #[test]
    fn sleep_budget_stays_below_the_system_acknowledgement_window() {
        for seconds in [0, 20] {
            let config = format!("[defaults]\nsleep_unmount_budget_secs = {seconds}\n");
            assert_eq!(
                Config::from_toml_str(&config)
                    .unwrap()
                    .defaults
                    .sleep_unmount_budget_secs,
                seconds
            );
        }
        for seconds in [21, 86_400] {
            let config = format!("[defaults]\nsleep_unmount_budget_secs = {seconds}\n");
            assert!(Config::from_toml_str(&config).is_err());
        }
    }

    #[test]
    fn rejects_network_in_requires() {
        let toml = r#"
[networks.home]
subnet = "10.0.0.0/24"

[[mount]]
name = "x"
protocol = "nfs"
export = "/e"
mountpoint = "~/mnt/x"

  [[mount.endpoint]]
  address = "10.0.0.1"
  requires = "network:home"
"#;
        let err = Config::from_toml_str(toml).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
    }
}
