//! Network identity via fingerprints (subnet + gateway MAC), not SSID.
//! SSID is unreadable without a Location-authorized app bundle on modern
//! macOS; the gateway MAC identifies a network on both Wi-Fi and Ethernet
//! with no TCC prompts.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use crate::config::{normalize_mac, Config, NetworkFingerprint};

/// A point-in-time view of the local network state. Equality is used for
/// change detection (a changed snapshot is a reconcile hint).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetSnapshot {
    pub gateway_ip: Option<Ipv4Addr>,
    /// Normalized (lowercase, zero-padded) MAC of the default gateway.
    pub gateway_mac: Option<String>,
    /// Local interface IPv4 addresses with prefix length, loopback excluded.
    pub local_addrs: Vec<(Ipv4Addr, u8)>,
    /// Whether any utun interface holds an address (VPN/Tailscale hint).
    pub utun_present: bool,
}

impl NetSnapshot {
    /// A fingerprint matches when any local address is inside its subnet and,
    /// if the fingerprint pins a gateway MAC, the current gateway MAC equals it.
    pub fn matches(&self, fp: &NetworkFingerprint) -> bool {
        let subnet_ok = self
            .local_addrs
            .iter()
            .any(|(ip, _)| fp.subnet.contains(*ip));
        let mac_ok = match &fp.gateway_mac {
            None => true,
            Some(want) => self.gateway_mac.as_deref() == Some(want.as_str()),
        };
        subnet_ok && mac_ok
    }

    /// Names of all configured networks this snapshot matches.
    pub fn matched_networks(&self, cfg: &Config) -> BTreeSet<String> {
        cfg.networks
            .iter()
            .filter(|(_, fp)| self.matches(fp))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// Parse `route -n get default` output; returns (gateway_ip, interface).
pub fn parse_route_get(out: &str) -> (Option<Ipv4Addr>, Option<String>) {
    let mut gw = None;
    let mut iface = None;
    for line in out.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("gateway:") {
            gw = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("interface:") {
            iface = Some(v.trim().to_string());
        }
    }
    (gw, iface)
}

/// Parse `arp -n <ip>` output for the MAC address, normalized.
/// Typical line: `? (192.168.1.1) at a4:91:b1:4:5:f6 on en0 ifscope [ethernet]`.
pub fn parse_arp(out: &str) -> Option<String> {
    let after_at = out.split(" at ").nth(1)?;
    let mac = after_at.split_whitespace().next()?;
    normalize_mac(mac)
}

/// Convert a netmask (e.g. 255.255.255.0) to a prefix length.
pub fn mask_to_prefix(mask: Ipv4Addr) -> u8 {
    u32::from(mask).count_ones() as u8
}

/// Enumerate local IPv4 addresses via getifaddrs(3). Returns
/// (non-loopback addrs with prefix, utun_present).
pub fn local_ipv4_addrs() -> (Vec<(Ipv4Addr, u8)>, bool) {
    let mut addrs = Vec::new();
    let mut utun = false;
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return (addrs, utun);
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() {
                continue;
            }
            if (*ifa.ifa_addr).sa_family != libc::AF_INET as libc::sa_family_t {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy();
            let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            if ip.is_loopback() {
                continue;
            }
            if name.starts_with("utun") {
                utun = true;
            }
            let prefix = if ifa.ifa_netmask.is_null() {
                32
            } else {
                let mask = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                mask_to_prefix(Ipv4Addr::from(u32::from_be(mask.sin_addr.s_addr)))
            };
            addrs.push((ip, prefix));
        }
        libc::freeifaddrs(ifap);
    }
    (addrs, utun)
}

/// These are kernel-table lookups that normally return in milliseconds, but
/// they run on every reconcile pass — and the engine has exactly one of those.
/// A wedged `route` or `arp` during interface teardown would otherwise stop the
/// daemon managing anything at all.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Run a short-lived query command, yielding its stdout, or empty on any
/// failure or timeout.
async fn query(program: &str, args: &[&str]) -> String {
    let run = tokio::process::Command::new(program)
        .args(args)
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(QUERY_TIMEOUT, run).await {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(Err(_)) => String::new(),
        Err(_) => {
            tracing::warn!("`{program}` timed out while reading network state");
            String::new()
        }
    }
}

/// Gather a full snapshot: local addresses plus default gateway IP and MAC.
pub async fn gather() -> NetSnapshot {
    let (local_addrs, utun_present) = local_ipv4_addrs();

    let route_out = query("/sbin/route", &["-n", "get", "default"]).await;
    let (gateway_ip, _iface) = parse_route_get(&route_out);

    let gateway_mac = match gateway_ip {
        Some(gw) => {
            let arp_out = query("/usr/sbin/arp", &["-n", &gw.to_string()]).await;
            parse_arp(&arp_out)
        }
        None => None,
    };

    NetSnapshot {
        gateway_ip,
        gateway_mac,
        local_addrs,
        utun_present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Cidr;

    fn fp(subnet: &str, mac: Option<&str>) -> NetworkFingerprint {
        NetworkFingerprint {
            subnet: subnet.parse::<Cidr>().unwrap(),
            gateway_mac: mac.map(|m| normalize_mac(m).unwrap()),
        }
    }

    fn snap(addr: &str, gw_mac: Option<&str>) -> NetSnapshot {
        NetSnapshot {
            gateway_ip: None,
            gateway_mac: gw_mac.map(|m| m.to_string()),
            local_addrs: vec![(addr.parse().unwrap(), 24)],
            utun_present: false,
        }
    }

    #[test]
    fn matches_on_subnet_alone() {
        let fp = fp("192.168.1.0/24", None);
        assert!(snap("192.168.1.42", None).matches(&fp));
        assert!(!snap("192.168.11.42", None).matches(&fp));
    }

    #[test]
    fn pinned_mac_must_match() {
        let fp = fp("192.168.1.0/24", Some("AA:BB:C:D:EE:FF"));
        assert!(snap("192.168.1.42", Some("aa:bb:0c:0d:ee:ff")).matches(&fp));
        // wrong MAC: same subnet elsewhere (e.g. another site using 192.168.1.0/24)
        assert!(!snap("192.168.1.42", Some("11:22:33:44:55:66")).matches(&fp));
        // no gateway MAC known yet -> conservative non-match
        assert!(!snap("192.168.1.42", None).matches(&fp));
    }

    #[test]
    fn parses_route_get_output() {
        let out = "\
   route to: default
destination: default
       mask: default
    gateway: 192.168.1.1
  interface: en0
      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>
";
        let (gw, iface) = parse_route_get(out);
        assert_eq!(gw, Some("192.168.1.1".parse().unwrap()));
        assert_eq!(iface.as_deref(), Some("en0"));
    }

    #[test]
    fn parses_route_get_no_default() {
        let (gw, iface) = parse_route_get("route: writing to routing socket: not in table\n");
        assert_eq!(gw, None);
        assert_eq!(iface, None);
    }

    #[test]
    fn parses_arp_output_with_short_octets() {
        let out = "? (192.168.1.1) at a4:91:b1:4:5:f6 on en0 ifscope [ethernet]\n";
        assert_eq!(parse_arp(out).as_deref(), Some("a4:91:b1:04:05:f6"));
    }

    #[test]
    fn arp_incomplete_yields_none() {
        let out = "? (192.168.1.1) at (incomplete) on en0 ifscope [ethernet]\n";
        assert_eq!(parse_arp(out), None);
    }

    #[test]
    fn netmask_to_prefix() {
        assert_eq!(mask_to_prefix("255.255.255.0".parse().unwrap()), 24);
        assert_eq!(mask_to_prefix("255.255.0.0".parse().unwrap()), 16);
        assert_eq!(mask_to_prefix("255.255.255.255".parse().unwrap()), 32);
    }

    #[test]
    fn local_addrs_excludes_loopback() {
        let (addrs, _) = local_ipv4_addrs();
        assert!(addrs.iter().all(|(ip, _)| !ip.is_loopback()));
    }
}
