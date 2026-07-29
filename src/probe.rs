//! Reachability probes. Gating uses TCP connects to the service port —
//! a rehearsal of the mount's own first syscall — never ICMP, which
//! false-positives (host up, service down) and false-negatives (filtered).

use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;

/// True if a TCP connection to `host:port` succeeds within `deadline`.
pub async fn probe_tcp(host: &str, port: u16, deadline: Duration) -> bool {
    let attempt = async {
        let addrs = tokio::net::lookup_host((host, port)).await.ok()?;
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
pub async fn select_endpoint(cands: &[Candidate], stagger: Duration) -> Option<usize> {
    let handles: Vec<_> = cands
        .iter()
        .enumerate()
        .map(|(rank, c)| {
            let c = c.clone();
            tokio::spawn(async move {
                tokio::time::sleep(stagger * rank as u32).await;
                probe_tcp(&c.address, c.port, c.timeout).await
            })
        })
        .collect();

    for (c, handle) in cands.iter().zip(handles) {
        if handle.await.unwrap_or(false) {
            return Some(c.index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
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
    async fn selects_highest_priority_live_endpoint() {
        let (_l1, p1) = listener().await;
        let (_l2, p2) = listener().await;
        let cands = vec![cand(0, "127.0.0.1", p1, 500), cand(1, "127.0.0.1", p2, 500)];
        assert_eq!(
            select_endpoint(&cands, Duration::from_millis(10)).await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn falls_through_to_next_priority_when_first_dead() {
        let (dead, dead_port) = listener().await;
        drop(dead);
        let (_live, live_port) = listener().await;
        let cands = vec![
            cand(0, "127.0.0.1", dead_port, 300),
            cand(7, "127.0.0.1", live_port, 300),
        ];
        // returns the candidate's own index, not its rank
        assert_eq!(
            select_endpoint(&cands, Duration::from_millis(10)).await,
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
            select_endpoint(&cands, Duration::from_millis(10)).await,
            None
        );
    }

    #[tokio::test]
    async fn empty_candidates_yield_none() {
        assert_eq!(select_endpoint(&[], Duration::from_millis(10)).await, None);
    }
}
