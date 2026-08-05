//! Reachability probes. Gating uses TCP connects to the service port —
//! a rehearsal of the mount's own first syscall — never ICMP, which
//! false-positives (host up, service down) and false-negatives (filtered).

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::BoxFut;

/// Share of the probe budget DNS may consume before the connect gets the rest.
/// A hostname endpoint whose resolver is wedged (MagicDNS while tailscaled is
/// unhappy) must not eat the whole deadline and read as unreachable.
const DNS_BUDGET_FRACTION: u32 = 2;
const DNS_BUDGET_CAP: Duration = Duration::from_millis(750);

/// True if a TCP connection to `host:port` succeeds within `deadline`.
///
/// Resolution gets its own sub-deadline, and resolved addresses are tried
/// IPv4-first: on a v4-only network a blackholed AAAA would otherwise consume
/// the entire budget before the working A record is ever attempted.
pub async fn probe_tcp(host: &str, port: u16, deadline: Duration) -> bool {
    let attempt = async {
        let dns_budget = (deadline / DNS_BUDGET_FRACTION).min(DNS_BUDGET_CAP);
        let resolved = timeout(dns_budget, tokio::net::lookup_host((host, port)))
            .await
            .ok()?
            .ok()?;

        let mut addrs: Vec<SocketAddr> = resolved.collect();
        addrs.sort_by_key(|a| !a.is_ipv4()); // false (0) sorts first

        for addr in addrs {
            if TcpStream::connect(addr).await.is_ok() {
                return Some(());
            }
        }
        None
    };
    matches!(timeout(deadline, attempt).await, Ok(Some(())))
}

/// A probe candidate: one endpoint of a mount, in priority order.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Index into the mount's endpoint list (identifies the winner).
    pub index: usize,
    pub address: String,
    pub port: u16,
    pub timeout: Duration,
}

/// Probe all candidates concurrently (staggered by `stagger` per rank so the
/// preferred endpoint gets a head start) and return the index of the
/// highest-priority reachable one. Priority wins, not speed: a lower-priority
/// success is only used after every higher-priority candidate has failed.
///
/// `probe` is injected so tests can control per-address latency and prove the
/// priority property, which is impossible against real localhost sockets that
/// all answer instantly.
pub async fn select_endpoint<P>(cands: &[Candidate], stagger: Duration, probe: P) -> Option<usize>
where
    P: Fn(String, u16, Duration) -> BoxFut<bool> + Clone + Send + 'static,
{
    let handles: Vec<_> = cands
        .iter()
        .enumerate()
        .map(|(rank, c)| {
            let c = c.clone();
            let probe = probe.clone();
            tokio::spawn(async move {
                tokio::time::sleep(stagger * rank as u32).await;
                probe(c.address, c.port, c.timeout).await
            })
        })
        .collect();

    // Every handle is awaited even after a winner is known, so no probe task
    // outlives this call still holding a socket.
    let mut winner = None;
    for (c, handle) in cands.iter().zip(handles) {
        let up = handle.await.unwrap_or(false);
        if winner.is_none() && up {
            winner = Some(c.index);
        }
    }
    winner
}

/// `select_endpoint` against the real network.
pub async fn select_endpoint_live(cands: &[Candidate], stagger: Duration) -> Option<usize> {
    select_endpoint(cands, stagger, |addr, port, to| {
        Box::pin(async move { probe_tcp(&addr, port, to).await })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    async fn listener() -> (TcpListener, u16) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    fn cand(index: usize, address: &str, port: u16, ms: u64) -> Candidate {
        Candidate {
            index,
            address: address.to_string(),
            port,
            timeout: Duration::from_millis(ms),
        }
    }

    /// A probe that answers `up` after `delay` for whichever addresses are
    /// listed, letting a test invert speed against priority.
    fn scripted(
        script: Vec<(&'static str, bool, u64)>,
        calls: Arc<AtomicUsize>,
    ) -> impl Fn(String, u16, Duration) -> BoxFut<bool> + Clone + Send + 'static {
        move |addr: String, _port, _to| {
            let script = script.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let (_, up, delay) = script
                    .iter()
                    .find(|(a, _, _)| *a == addr)
                    .copied()
                    .unwrap_or(("", false, 0));
                tokio::time::sleep(Duration::from_millis(delay)).await;
                up
            })
        }
    }

    #[tokio::test]
    async fn probe_succeeds_against_listener() {
        let (_l, port) = listener().await;
        assert!(probe_tcp("127.0.0.1", port, Duration::from_millis(500)).await);
    }

    #[tokio::test]
    async fn probe_fails_against_closed_port() {
        let (l, port) = listener().await;
        drop(l); // port now closed -> fast RST
        assert!(!probe_tcp("127.0.0.1", port, Duration::from_millis(500)).await);
    }

    #[tokio::test]
    async fn probe_times_out_on_blackhole() {
        // TEST-NET-3 is unroutable; the timeout must bound the wait.
        let start = std::time::Instant::now();
        assert!(!probe_tcp("203.0.113.1", 445, Duration::from_millis(200)).await);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn probe_fails_on_unresolvable_host() {
        assert!(!probe_tcp("nonexistent.invalid", 445, Duration::from_millis(500)).await);
    }

    #[tokio::test]
    async fn slow_dns_cannot_consume_the_whole_budget() {
        // The DNS sub-deadline is a fraction of the total, so an unresolvable
        // name returns well before the overall deadline elapses.
        let start = std::time::Instant::now();
        assert!(!probe_tcp("nonexistent.invalid", 445, Duration::from_secs(4)).await);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn selects_highest_priority_live_endpoint() {
        let (_l1, p1) = listener().await;
        let (_l2, p2) = listener().await;
        let cands = vec![cand(0, "127.0.0.1", p1, 500), cand(1, "127.0.0.1", p2, 500)];
        assert_eq!(
            select_endpoint_live(&cands, Duration::from_millis(10)).await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn priority_beats_speed_even_when_the_preferred_endpoint_is_slower() {
        // The real property: endpoint 0 answers 200ms later than endpoint 1,
        // and must still win. A "first to answer" implementation returns 1.
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = scripted(
            vec![("slow.lan", true, 200), ("fast.tailnet", true, 0)],
            calls.clone(),
        );
        let cands = vec![
            cand(0, "slow.lan", 445, 1000),
            cand(1, "fast.tailnet", 445, 1000),
        ];
        assert_eq!(
            select_endpoint(&cands, Duration::from_millis(0), probe).await,
            Some(0)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn falls_through_to_next_priority_when_first_dead() {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = scripted(
            vec![("dead.lan", false, 0), ("live.tailnet", true, 0)],
            calls,
        );
        let cands = vec![
            cand(0, "dead.lan", 445, 300),
            cand(7, "live.tailnet", 445, 300),
        ];
        // returns the candidate's own index, not its rank
        assert_eq!(
            select_endpoint(&cands, Duration::from_millis(0), probe).await,
            Some(7)
        );
    }

    #[tokio::test]
    async fn returns_none_when_all_dead() {
        let (d1, p1) = listener().await;
        let (d2, p2) = listener().await;
        drop(d1);
        drop(d2);
        let cands = vec![cand(0, "127.0.0.1", p1, 300), cand(1, "127.0.0.1", p2, 300)];
        assert_eq!(
            select_endpoint_live(&cands, Duration::from_millis(10)).await,
            None
        );
    }

    #[tokio::test]
    async fn empty_candidates_yield_none() {
        assert_eq!(
            select_endpoint_live(&[], Duration::from_millis(10)).await,
            None
        );
    }
}
