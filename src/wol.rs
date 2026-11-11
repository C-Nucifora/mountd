//! Wake-on-LAN: nudge a sleeping file server before giving up on an endpoint.
//!
//! A magic packet is 6 bytes of 0xFF followed by the target MAC repeated 16
//! times, sent as a UDP broadcast (port 9 by convention). It is fire-and-forget:
//! there is no acknowledgement, so the caller pairs it with the ordinary probe
//! gate and simply waits for the server to answer on its service port.

use std::net::{Ipv4Addr, SocketAddrV4};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::normalize_mac;

fn default_broadcast() -> String {
    "255.255.255.255".to_string()
}

fn default_wol_port() -> u16 {
    9
}

/// Wake-on-LAN settings for one endpoint.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WolSpec {
    /// Target MAC address; accepts `:` or `-` separators, any case.
    pub mac: String,
    /// Broadcast address to send to. Prefer the subnet broadcast (e.g.
    /// `192.168.1.255`) — some routers drop 255.255.255.255.
    #[serde(default = "default_broadcast")]
    pub broadcast: String,
    #[serde(default = "default_wol_port")]
    pub port: u16,
}

/// Build the 102-byte magic packet for `mac`.
pub fn magic_packet(mac: &str) -> Result<Vec<u8>> {
    let normalized = normalize_mac(mac).with_context(|| format!("invalid WoL MAC `{mac}`"))?;
    let octets: Vec<u8> = normalized
        .split(':')
        .map(|o| u8::from_str_radix(o, 16))
        .collect::<Result<_, _>>()
        .with_context(|| format!("invalid WoL MAC `{mac}`"))?;

    let mut packet = Vec::with_capacity(6 + 16 * 6);
    packet.extend_from_slice(&[0xFF; 6]);
    for _ in 0..16 {
        packet.extend_from_slice(&octets);
    }
    Ok(packet)
}

/// Send a magic packet for `spec`. Binds an ephemeral UDP socket with
/// broadcast enabled; the send is best-effort by nature.
pub async fn send(spec: &WolSpec) -> Result<()> {
    send_from(spec, Ipv4Addr::UNSPECIFIED).await
}

async fn send_from(spec: &WolSpec, bind_address: Ipv4Addr) -> Result<()> {
    let packet = magic_packet(&spec.mac)?;
    let broadcast: Ipv4Addr = spec
        .broadcast
        .parse()
        .with_context(|| format!("invalid WoL broadcast address `{}`", spec.broadcast))?;

    let socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(bind_address, 0))
        .await
        .context("binding WoL socket")?;
    socket
        .set_broadcast(true)
        .context("enabling broadcast on the WoL socket")?;
    socket
        .send_to(&packet, SocketAddrV4::new(broadcast, spec.port))
        .await
        .with_context(|| format!("sending WoL packet to {}:{}", broadcast, spec.port))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_packet_layout() {
        let p = magic_packet("aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xFF; 6]);
        // the MAC repeats exactly 16 times after the header
        for chunk in p[6..].chunks(6) {
            assert_eq!(chunk, &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        }
        assert_eq!(p[6..].len() / 6, 16);
    }

    #[test]
    fn accepts_short_octets_and_dashes() {
        // macOS `arp` prints octets without leading zeros; dashes are common
        // in vendor documentation. Both normalize to the same packet.
        let a = magic_packet("a4:91:b1:4:5:f6").unwrap();
        let b = magic_packet("A4-91-B1-04-05-F6").unwrap();
        assert_eq!(a, b);
        assert_eq!(&a[6..12], &[0xa4, 0x91, 0xb1, 0x04, 0x05, 0xf6]);
    }

    #[test]
    fn rejects_malformed_mac() {
        assert!(magic_packet("not-a-mac").is_err());
        assert!(magic_packet("aa:bb:cc:dd:ee").is_err());
        assert!(magic_packet("zz:bb:cc:dd:ee:ff").is_err());
    }

    #[tokio::test]
    async fn send_reports_invalid_broadcast() {
        let spec = WolSpec {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            broadcast: "not-an-ip".into(),
            port: 9,
        };
        let err = send(&spec).await.unwrap_err().to_string();
        assert!(err.contains("invalid WoL broadcast"), "{err}");
    }

    #[tokio::test]
    async fn send_to_loopback_broadcast_succeeds() {
        // 127.0.0.1 is not a broadcast address, but the send path (bind,
        // set_broadcast, send_to) is exercised end to end without leaving
        // the machine.
        let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let spec = WolSpec {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            broadcast: "127.0.0.1".into(),
            port: receiver.local_addr().unwrap().port(),
        };
        send_from(&spec, Ipv4Addr::LOCALHOST).await.unwrap();
        let mut packet = [0; 103];
        let (size, source) = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            receiver.recv_from(&mut packet),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(source.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(&packet[..size], magic_packet(&spec.mac).unwrap());
    }
}
