use super::*;
#[allow(dead_code)]
mod common {
    include!(env!("MOUNTD_REVIEW_COMMON"));
}
use common::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn names_generated_boundaries_and_policy() {
    let budget = MAX_REQUEST_BYTES as usize
        - serde_json::to_vec(&Request::Unmount {
            name: String::new(),
        })
        .unwrap()
        .len()
        - 1;
    let mut roundtrips = 0;
    for seed in 1..=8 {
        for offset in [-1isize, 0, 1] {
            let name = generated(seed, (budget as isize + offset) as usize);
            let frames: Vec<_> = requests(&name)
                .iter()
                .map(|r| serde_json::to_vec(r).unwrap().len() + 1)
                .collect();
            let accepted = config(&name).is_ok();
            assert_eq!(
                accepted,
                frames.iter().all(|n| *n <= MAX_REQUEST_BYTES as usize),
                "seed={seed} offset={offset} frames={frames:?}"
            );
            if !accepted {
                continue;
            }
            let server = Server::new(&name).await;
            server.engine.permits.add_permits(2);
            let status = client_request(&server.path, &Request::Status)
                .await
                .unwrap();
            assert_eq!(status.status.unwrap()[0].name, name);
            for request in requests(&name) {
                assert!(client_request(&server.path, &request).await.unwrap().ok);
                roundtrips += 1;
            }
            assert_eq!(server.engine.completed.load(Ordering::SeqCst), 3);
            server.finish().await;
        }
    }
    for c in (0u32..=0x9f)
        .filter_map(char::from_u32)
        .filter(|c| c.is_control())
    {
        assert!(
            config(&format!("prefix{c}suffix")).is_err(),
            "control U+{:04X}",
            c as u32
        );
    }
    for name in ["", " ", "\u{a0}", "\u{2003}", "\u{2028}", "\u{2029}"] {
        assert!(config(name).is_err());
    }
    for name in [
        "--", "-", "--json", "--help=x", "\u{200d}", "\u{feff}", " x ",
    ] {
        assert!(config(name).is_ok());
    }
    eprintln!("GENERATED seeds=8 boundary_cases=24 status_roundtrips=16 named_roundtrips={roundtrips} controls_rejected=65 empty_whitespace_rejected=6 accepted_edges=7");
}

#[tokio::test]
async fn transport_cancelled_clients_saturated_writers_and_shutdown() {
    let server = Server::new("--").await;
    // Two near-limit responses stall in the Unix send buffers, without 64 large allocations.
    server
        .engine
        .padding
        .store(3 * 1024 * 1024, Ordering::SeqCst);
    let mut writers = Vec::new();
    for expected in 1..=2 {
        let mut peer = UnixStream::connect(&server.path).await.unwrap();
        peer.write_all(b"{\"cmd\":\"status\"}\n").await.unwrap();
        count(&server.engine.statuses, expected).await;
        writers.push(peer);
    }
    // Real client futures disconnect on cancellation; accepted fake effects remain held.
    for expected in 1..=62 {
        let path = server.path.clone();
        let client = tokio::spawn(async move {
            client_request(&path, &Request::Unmount { name: "--".into() }).await
        });
        count(&server.engine.entered, expected).await;
        client.abort();
        assert!(client.await.unwrap_err().is_cancelled());
    }
    for _ in 0..4 {
        assert!(
            client_request_with_timeout(&server.path, &Request::Poke, Duration::from_secs(1))
                .await
                .is_err()
        );
    }
    assert_eq!(server.engine.completed.load(Ordering::SeqCst), 0);
    let started = std::time::Instant::now();
    server.stop.send(()).unwrap();
    let mut task = server.task;
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut task)
        .await
        .is_err());
    assert!(
        UnixStream::connect(&server.path).await.is_err(),
        "listener remains after shutdown"
    );
    assert!(
        bind_socket(&server.path).await.is_err(),
        "singleton released during drain"
    );
    server.engine.permits.add_permits(62);
    count(&server.engine.completed, 62).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut task)
            .await
            .is_err(),
        "stalled writers were cancelled"
    );
    assert!(bind_socket(&server.path).await.is_err());
    tokio::time::timeout(Duration::from_secs(12), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(8));
    drop(writers);
    let (listener, lock) = bind_socket(&server.path).await.unwrap();
    let (stop, rx) = tokio::sync::oneshot::channel();
    let replacement = tokio::spawn(serve_bound_until(
        server.engine.clone(),
        listener,
        lock,
        async {
            let _ = rx.await;
        },
    ));
    assert!(
        client_request(&server.path, &Request::Poke)
            .await
            .unwrap()
            .ok
    );
    stop.send(()).unwrap();
    replacement.await.unwrap().unwrap();
    eprintln!("COMBINED writers=2 cancelled_clients=62 excess_rejected=4 completed=62 drain_ms={} replacement_poke=ok", started.elapsed().as_millis());
}
