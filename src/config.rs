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
    #[error("mount `{mount}`: {msg}")]
    Invalid { mount: String, msg: String },
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
/// macOS `arp` prints octets without leading zeros (`aa:bb:c:d:ee:ff`).
pub fn normalize_mac(mac: &str) -> Option<String> {
    let octets: Vec<String> = mac
        .trim()
        .split(':')
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
            Some(("tcp", p)) => p
                .parse()
                .map(Probe::Tcp)
                .map_err(|_| format!("invalid probe port in `{s}`")),
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "default_probe_timeout_ms")]
    pub probe_timeout_ms: u64,
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u64,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_mount_root")]
    pub mount_root: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            probe_timeout_ms: default_probe_timeout_ms(),
            debounce_secs: default_debounce_secs(),
            poll_interval_secs: default_poll_interval_secs(),
            mount_root: default_mount_root(),
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default = "default_true")]
    pub unmount_when_gone: bool,
    #[serde(default)]
    pub retry: Retry,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            unmount_when_gone: true,
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
    /// Extra `-o` mount options, passed through verbatim.
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

#[derive(Debug, Clone, Deserialize)]
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
        for net in cfg.networks.values_mut() {
            if let Some(mac) = &net.gateway_mac {
                let norm = normalize_mac(mac).ok_or_else(|| ConfigError::Invalid {
                    mount: "networks".into(),
                    msg: format!("invalid gateway_mac `{mac}`"),
                })?;
                net.gateway_mac = Some(norm);
            }
        }
        cfg.validate()?;
        Ok(cfg)
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
        let mut seen = std::collections::BTreeSet::new();
        for m in &self.mounts {
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
            for r in m.when.any_of.iter().chain(&m.when.all_of) {
                self.check_ref(&m.name, r)?;
            }
            for ep in &m.endpoints {
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
            }
        }
        Ok(())
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
