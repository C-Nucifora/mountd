//! Reachability probes. Gating uses TCP connects to the service port —
//! a rehearsal of the mount's own first syscall — never ICMP, which
//! false-positives (host up, service down) and false-negatives (filtered).

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::BoxFut;

/// Share of the probe budget DNS may consume before the connect gets the rest.
/// A hostname endpoint whose resolver is wedged (MagicDNS while tailscaled is
/// unhappy) must not eat the whole deadline and read as unreachable.
const DNS_BUDGET_FRACTION: u32 = 2;
const DNS_BUDGET_CAP: Duration = Duration::from_millis(750);
// Timed-out libc calls can survive indefinitely; retries must not grow threads.
static RESOLVERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

/// True if a TCP connection to `host:port` succeeds within `deadline`.
///
/// Resolution gets its own sub-deadline, and resolved addresses are tried
/// IPv4-first: on a v4-only network a blackholed AAAA would otherwise consume
/// the entire budget before the working A record is ever attempted.
pub async fn probe_tcp(host: &str, port: u16, deadline: Duration) -> bool {
    let attempt = async {
        let dns_budget = (deadline / DNS_BUDGET_FRACTION).min(DNS_BUDGET_CAP);
        let mut addrs = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else {
            timeout(dns_budget, resolve(host, port)).await.ok()?.ok()?
        };
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

/// libc resolution can block after cancellation. Keep it outside Tokio's
/// blocking pool so shutdown can explicitly drain it without freezing the UI.
async fn resolve(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    let host = host.to_string();
    resolve_limited(&RESOLVERS, move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|addrs| addrs.collect())
    })
    .await
}

async fn resolve_limited(
    slots: &'static tokio::sync::Semaphore,
    lookup: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
) -> std::io::Result<Vec<SocketAddr>> {
    let slot = slots.try_acquire().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "resolver capacity exhausted",
        )
    })?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let activity = crate::mountcmd::CommandActivity::begin();
    std::thread::Builder::new()
        .name("mountd-dns".into())
        .spawn(move || {
            let _activity = activity;
            let _slot = slot;
            let _ = send.send(lookup());
        })?;
    receive
        .await
        .map_err(|_| std::io::Error::other("resolver stopped"))?
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
    let mut tasks = tokio::task::JoinSet::new();
    for (rank, c) in cands.iter().enumerate() {
        let c = c.clone();
        let probe = probe.clone();
        let activity = crate::mountcmd::CommandActivity::begin();
        tasks.spawn(async move {
            let _activity = activity;
            tokio::time::sleep(stagger.saturating_mul(rank as u32)).await;
            let up = timeout(c.timeout, probe(c.address, c.port, c.timeout))
                .await
                .unwrap_or(false);
            (rank, c.index, up)
        });
    }

    // JoinSet aborts outstanding probes if the caller is cancelled.
    let mut winner = None;
    let mut best_rank = usize::MAX;
    while let Some(result) = tasks.join_next().await {
        if let Ok((rank, index, true)) = result {
            if rank < best_rank {
                best_rank = rank;
                winner = Some(index);
            }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn drain_waits_for_aborted_nested_producer_before_child_registration() {
        let started = Arc::new(tokio::sync::Notify::new());
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Arc::new(std::sync::Mutex::new(blocked));
        let parent = {
            let started = started.clone();
            tokio::spawn(async move {
                let candidates = vec![Candidate {
                    index: 0,
                    address: "synthetic".into(),
                    port: 445,
                    timeout: Duration::from_secs(10),
                }];
                select_endpoint(&candidates, Duration::ZERO, move |_, _, _| {
                    // Hold the current poll before it can register a resolver or command.
                    started.notify_one();
                    blocked
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                    Box::pin(async { false })
                })
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        parent.abort();
        let _ = parent.await;
        let held =
            tokio::time::timeout(Duration::from_millis(30), crate::mountcmd::drain_commands())
                .await;
        release.send(()).unwrap();
        assert!(held.is_err(), "drain escaped a still-polling producer");
        tokio::time::timeout(Duration::from_secs(2), crate::mountcmd::drain_commands())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_resolver_keeps_capacity_until_its_thread_finishes() {
        static SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let worker = tokio::spawn(resolve_limited(&SLOT, move || {
            let _ = started.send(());
            let _ = held.recv();
            Ok(vec![])
        }));
        timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        worker.abort();
        let _ = worker.await;
        let error = resolve_limited(&SLOT, || panic!("must not start another resolver"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        release.send(()).unwrap();
        timeout(Duration::from_secs(2), async {
            while SLOT.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(resolve_limited(&SLOT, || Ok(vec![])).await.is_ok());
    }
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
    async fn owned_resolver_thread_returns_numeric_fixture_address() {
        assert_eq!(
            resolve("127.0.0.1", 12345).await.unwrap(),
            vec!["127.0.0.1:12345".parse::<SocketAddr>().unwrap()]
        );
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
    async fn hung_injected_probe_is_bounded() {
        let cands = vec![cand(0, "fake", 445, 10)];
        let probe = |_: String, _, _| -> BoxFut<bool> { Box::pin(std::future::pending()) };
        assert_eq!(
            timeout(
                Duration::from_secs(1),
                select_endpoint(&cands, Duration::ZERO, probe)
            )
            .await
            .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn cancelled_selection_drops_probe_futures() {
        struct Guard(Arc<AtomicUsize>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let active = Arc::new(AtomicUsize::new(0));
        let count = active.clone();
        let probe = move |_: String, _, _| -> BoxFut<bool> {
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                let _guard = Guard(count);
                std::future::pending().await
            })
        };
        let task = tokio::spawn(async move {
            select_endpoint(&[cand(0, "fake", 445, 30_000)], Duration::ZERO, probe).await
        });
        timeout(Duration::from_secs(1), async {
            while active.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        timeout(Duration::from_secs(1), async {
            while active.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
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
