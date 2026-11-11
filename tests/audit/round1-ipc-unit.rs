use super::*;
use crate::config::Config;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::sync::oneshot;

fn parse_config(name: &str) -> Result<Config, crate::config::ConfigError> {
    let value = format!(
        "[[mount]]\nname={}\nprotocol=\"nfs\"\nexport=\"/synthetic\"\nmountpoint=\"/synthetic\"\n[[mount.endpoint]]\naddress=\"127.0.0.1\"\n",
        serde_json::to_string(name).unwrap()
    );
    Config::from_toml_str(&value)
}

fn config(name: &str) -> Config {
    parse_config(name).unwrap()
}

#[test]
fn config_name_limit_matches_named_request_frames() {
    let overhead = serde_json::to_vec(&Request::Unmount {
        name: String::new(),
    })
    .unwrap()
    .len()
        + 1;
    let budget = MAX_REQUEST_BYTES as usize - overhead;
    for unit in ["a", "\"", "\\", "日", "😀", "\"\\日"] {
        let unit_bytes = serde_json::to_string(unit).unwrap().len() - 2;
        let mut name = unit.repeat(budget / unit_bytes);
        name.push_str(&"x".repeat(budget % unit_bytes));
        assert!(parse_config(&name).is_ok(), "exact budget must be usable");
        for request in [
            Request::Mount { name: name.clone() },
            Request::Unmount { name: name.clone() },
            Request::Explain {
                name: Some(name.clone()),
            },
        ] {
            let length = serde_json::to_vec(&request).unwrap().len() + 1;
            assert!(length <= MAX_REQUEST_BYTES as usize);
        }
        assert_eq!(
            serde_json::to_vec(&Request::Unmount { name: name.clone() })
                .unwrap()
                .len()
                + 1,
            MAX_REQUEST_BYTES as usize,
        );
        name.push('x');
        assert!(
            parse_config(&name).is_err(),
            "one encoded byte over must fail"
        );
    }
}

struct Server {
    _dir: tempfile::TempDir,
    path: PathBuf,
    engine: Arc<Engine>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl Server {
    async fn new(cfg: Config) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("s");
        let (listener, lock) = bind_socket(&path).await.unwrap();
        let engine = Engine::new(PathBuf::from("/nonexistent"), cfg);
        let (stop, rx) = oneshot::channel();
        let task = tokio::spawn(serve_bound_until(engine.clone(), listener, lock, async {
            let _ = rx.await;
        }));
        Self {
            _dir: dir,
            path,
            engine,
            stop,
            task,
        }
    }

    async fn finish(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(12), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let _replacement = bind_socket(&self.path).await.unwrap();
    }
}

async fn wait_count(count: &std::sync::atomic::AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::SeqCst) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn fds() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

#[tokio::test]
async fn completed_connections_release_slots_and_fds() {
    let server = Server::new(config("normal")).await;
    assert!(
        client_request(&server.path, &Request::Poke)
            .await
            .unwrap()
            .ok
    );
    let before = fds();
    let started = Instant::now();
    for _ in 0..1024 {
        assert!(
            client_request_with_timeout(&server.path, &Request::Poke, Duration::from_secs(1))
                .await
                .unwrap()
                .ok
        );
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    let after = fds();
    eprintln!(
        "CHURN requests=1024 fd_before={before} fd_after={after} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    assert!(after <= before, "completed connection fd growth");
    server.finish().await;
}

#[tokio::test]
async fn saturation_rejects_excess_then_recovers() {
    let server = Server::new(config("normal")).await;
    let before = fds();
    let mut peers = Vec::new();
    for n in 1..=64 {
        let mut peer = UnixStream::connect(&server.path).await.unwrap();
        peer.write_all(b"{\"cmd\":\"reload\"}\n").await.unwrap();
        peers.push(peer);
        wait_count(&server.engine.entered, n).await;
    }
    let saturated = fds();
    for _ in 0..32 {
        assert!(client_request_with_timeout(
            &server.path,
            &Request::Status,
            Duration::from_millis(500)
        )
        .await
        .is_err());
    }
    assert_eq!(server.engine.entered.load(Ordering::SeqCst), 64);
    assert_eq!(fds(), saturated);
    server.engine.permits.add_permits(64);
    for peer in &mut peers {
        let line = read_frame(peer, MAX_RESPONSE_BYTES).await.unwrap();
        assert!(serde_json::from_str::<Response>(&line).unwrap().ok);
    }
    drop(peers);
    assert!(
        client_request(&server.path, &Request::Status)
            .await
            .unwrap()
            .ok
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let after = fds();
    eprintln!("SATURATION admitted=64 rejected=32 completed={} fd_before={before} fd_peak={saturated} fd_after={after}", server.engine.completed.load(Ordering::SeqCst));
    assert!(after <= before);
    server.finish().await;
}

#[tokio::test]
async fn accepted_mutations_survive_disconnect_half_close_and_shutdown() {
    let mut cases = 0;
    for request in [
        Request::Mount {
            name: "normal".into(),
        },
        Request::Unmount {
            name: "normal".into(),
        },
        Request::Reload,
    ] {
        for mode in ["disconnect", "half-close", "no-read"] {
            let server = Server::new(config("normal")).await;
            let mut peer = UnixStream::connect(&server.path).await.unwrap();
            peer.write_all(format!("{}\n", serde_json::to_string(&request).unwrap()).as_bytes())
                .await
                .unwrap();
            wait_count(&server.engine.entered, 1).await;
            if mode == "half-close" {
                peer.shutdown().await.unwrap();
            }
            let mut peer = if mode == "disconnect" {
                drop(peer);
                None
            } else {
                Some(peer)
            };
            assert!(
                client_request(&server.path, &Request::Status)
                    .await
                    .unwrap()
                    .ok
            );
            server.stop.send(()).unwrap();
            let mut task = server.task;
            assert!(tokio::time::timeout(Duration::from_millis(25), &mut task)
                .await
                .is_err());
            assert!(bind_socket(&server.path).await.is_err());
            assert_eq!(server.engine.completed.load(Ordering::SeqCst), 0);
            server.engine.permits.add_permits(1);
            if mode == "half-close" {
                let line = read_frame(peer.as_mut().unwrap(), MAX_RESPONSE_BYTES)
                    .await
                    .unwrap();
                assert!(serde_json::from_str::<Response>(&line).unwrap().ok);
            }
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(server.engine.completed.load(Ordering::SeqCst), 1);
            let _replacement = bind_socket(&server.path).await.unwrap();
            cases += 1;
        }
    }
    eprintln!("MUTATION_PEERS cases={cases} completed=9 premature_unlocks=0");
}

#[tokio::test]
async fn incomplete_mutation_frames_never_mutate() {
    let server = Server::new(config("normal")).await;
    let full = b"{\"cmd\":\"mount\",\"name\":\"normal\"}\n";
    for cut in 0..full.len() {
        let mut peer = UnixStream::connect(&server.path).await.unwrap();
        peer.write_all(&full[..cut]).await.unwrap();
        peer.shutdown().await.unwrap();
        let mut reply = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert!(reply.is_empty());
    }
    assert_eq!(server.engine.entered.load(Ordering::SeqCst), 0);
    eprintln!("PREFIXES rejected={} mutations=0", full.len());
    server.finish().await;
}

#[tokio::test]
async fn idle_saturated_shutdown_is_bounded_and_holds_lock() {
    let server = Server::new(config("normal")).await;
    let before = fds();
    let mut peers = Vec::new();
    for _ in 0..64 {
        peers.push(UnixStream::connect(&server.path).await.unwrap());
    }
    // Linux resource observation establishes all 64 server fds were accepted.
    tokio::time::timeout(Duration::from_secs(2), async {
        while fds() != before + 128 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let started = Instant::now();
    server.stop.send(()).unwrap();
    let mut task = server.task;
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut task)
        .await
        .is_err());
    assert!(bind_socket(&server.path).await.is_err());
    tokio::time::timeout(Duration::from_secs(12), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(9));
    drop(peers);
    eprintln!(
        "IDLE_SHUTDOWN peers=64 elapsed_ms={} fd_before={before} fd_after={}",
        elapsed.as_millis(),
        fds()
    );
    let _replacement = bind_socket(&server.path).await.unwrap();
}

#[tokio::test]
async fn stalled_response_writer_times_out() {
    let engine = Engine::new(PathBuf::from("/nonexistent"), config("normal"));
    engine.padding.store(1024 * 1024, Ordering::SeqCst);
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let sndbuf: libc::c_int = 4096;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &sndbuf as *const _ as *const libc::c_void,
                std::mem::size_of_val(&sndbuf) as libc::socklen_t,
            )
        },
        0
    );
    peer.write_all(b"{\"cmd\":\"status\"}\n").await.unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(Duration::from_secs(12), serve_conn(engine, socket))
        .await
        .unwrap()
        .unwrap_err();
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    eprintln!(
        "STALLED_WRITER payload_bytes=1048576 sendbuf_requested=4096 elapsed_ms={}",
        started.elapsed().as_millis()
    );
}

#[tokio::test]
async fn request_response_caps_include_newline_consistently() {
    let mut cases = 0;
    for cap in [1, 2, 3, 31, 4096, MAX_REQUEST_BYTES] {
        for len in [cap.saturating_sub(1), cap, cap + 1] {
            if len == 0 {
                continue;
            }
            let mut frame = vec![b' '; len as usize];
            *frame.last_mut().unwrap() = b'\n';
            assert_eq!(read_frame(&frame[..], cap).await.is_ok(), len <= cap);
            cases += 1;
        }
    }
    let server = Server::new(config("normal")).await;
    let base = serde_json::to_string(&handle_request(&server.engine, Request::Status).await)
        .unwrap()
        .len();
    for payload in [
        96 * 1024,
        MAX_RESPONSE_BYTES as usize - 1,
        MAX_RESPONSE_BYTES as usize,
        MAX_RESPONSE_BYTES as usize + 1,
    ] {
        server
            .engine
            .padding
            .store(payload - base, Ordering::SeqCst);
        let response = client_request(&server.path, &Request::Status)
            .await
            .unwrap();
        assert_eq!(response.ok, payload < MAX_RESPONSE_BYTES as usize);
        if !response.ok {
            assert_eq!(response.error.as_deref(), Some("IPC response too large"));
        }
        eprintln!("RESPONSE serialized_bytes={payload} ok={}", response.ok);
    }
    eprintln!("FRAME_BOUNDARIES cases={cases}");
    server.finish().await;
}

#[tokio::test]
async fn valid_unicode_and_escaped_names_round_trip() {
    let names = [
        "ordinary",
        " space name ",
        "quote\"",
        "back\\slash",
        "日本語",
        "é",
        "😀",
        "--help",
    ];
    for name in names {
        let server = Server::new(config(name)).await;
        let response = client_request(&server.path, &Request::Status)
            .await
            .unwrap();
        assert_eq!(response.status.unwrap()[0].name, name);
        server.engine.permits.add_permits(1);
        assert!(
            client_request(&server.path, &Request::Unmount { name: name.into() })
                .await
                .unwrap()
                .ok
        );
        assert_eq!(server.engine.completed.load(Ordering::SeqCst), 1);
        server.finish().await;
    }
    eprintln!("NAME_ROUNDTRIP cases=8");
}

#[tokio::test]
async fn config_acceptance_implies_transportable_name() {
    let name = "\"".repeat(32768);
    let cfg = match parse_config(&name) {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("ACCEPTED_NAME rejected_at_config={error}");
            return;
        }
    };
    let server = Server::new(cfg).await;
    let response = client_request(&server.path, &Request::Status)
        .await
        .unwrap();
    assert_eq!(response.status.unwrap()[0].name, name);
    let request = Request::Unmount { name };
    let bytes = serde_json::to_vec(&request).unwrap().len() + 1;
    server.engine.permits.add_permits(1);
    let outcome = client_request(&server.path, &request).await;
    eprintln!("ACCEPTED_NAME name_bytes=32768 request_frame_bytes={bytes} outcome={outcome:?} mutations={}", server.engine.entered.load(Ordering::SeqCst));
    server.finish().await;
    assert!(
        outcome.is_ok(),
        "validated mount is listed by status but cannot be addressed: {outcome:?}"
    );
}
