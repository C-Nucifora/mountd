//! Unix-socket IPC between the daemon and `mountctl`: one newline-delimited
//! JSON request per connection, one JSON response back.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::warn;

use crate::config::MAX_CONTROL_REQUEST_BYTES as MAX_REQUEST_BYTES;
use crate::engine::{Engine, MountStatus};

/// Socket path: `~/Library/Application Support/mountd/mountd.sock`.
pub fn socket_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/mountd/mountd.sock")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    Mount {
        name: String,
    },
    Unmount {
        name: String,
    },
    Reload,
    Poke,
    Explain {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<MountStatus>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanations: Option<Vec<crate::reconcile::Explanation>>,
}

impl Response {
    fn ok() -> Self {
        Response {
            ok: true,
            error: None,
            status: None,
            explanations: None,
        }
    }
    fn err(e: impl ToString) -> Self {
        Response {
            ok: false,
            error: Some(e.to_string()),
            status: None,
            explanations: None,
        }
    }
}

pub async fn handle_request(engine: &Engine, req: Request) -> Response {
    match req {
        Request::Status => Response {
            ok: true,
            error: None,
            status: Some(engine.status().await),
            explanations: None,
        },
        Request::Explain { name } => match engine.explain(name.as_deref()).await {
            Ok(explanations) => Response {
                ok: true,
                error: None,
                status: None,
                explanations: Some(explanations),
            },
            Err(e) => Response::err(format!("{e:#}")),
        },
        Request::Mount { name } => match engine.force_mount(&name).await {
            Ok(()) => Response::ok(),
            Err(e) => Response::err(format!("{e:#}")),
        },
        Request::Unmount { name } => match engine.force_unmount(&name).await {
            Ok(()) => Response::ok(),
            Err(e) => Response::err(format!("{e:#}")),
        },
        Request::Reload => match engine.reload().await {
            Ok(()) => Response::ok(),
            Err(e) => Response::err(format!("{e:#}")),
        },
        Request::Poke => {
            engine.hint.notify_one();
            Response::ok()
        }
    }
}

/// A client that connects and then says nothing must not pin a task and an fd
/// forever; fd exhaustion is what turns a stuck client into a dead server.
const CONN_TIMEOUT: Duration = Duration::from_secs(10);

// A capped reader must distinguish a complete frame from EOF or a cap hit.
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(reader: R, cap: u64) -> Result<String> {
    let mut line = String::new();
    BufReader::new(reader.take(cap + 1))
        .read_line(&mut line)
        .await?;
    ensure!(line.len() as u64 <= cap, "IPC frame exceeds {cap} bytes");
    ensure!(line.ends_with('\n'), "IPC frame missing newline");
    Ok(line)
}

fn same_user(stream: &UnixStream) -> Result<()> {
    // peer_cred uses LOCAL_PEERCRED/getpeereid on macOS and SO_PEERCRED on Linux.
    ensure!(
        stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
        "IPC peer belongs to another user"
    );
    Ok(())
}

async fn serve_conn(engine: Arc<Engine>, stream: UnixStream) -> Result<()> {
    same_user(&stream)?;
    let (r, mut w) = stream.into_split();
    let line = tokio::time::timeout(CONN_TIMEOUT, read_frame(r, MAX_REQUEST_BYTES)).await??;
    let resp = match serde_json::from_str::<Request>(line.trim()) {
        // Do not cancel a mutating engine operation at the transport deadline.
        Ok(req) => handle_request(&engine, req).await,
        Err(e) => Response::err(format!("bad request: {e}")),
    };
    let mut out = serde_json::to_string(&resp)?;
    if out.len() as u64 >= MAX_RESPONSE_BYTES {
        out = serde_json::to_string(&Response::err("IPC response too large"))?;
    }
    out.push('\n');
    tokio::time::timeout(CONN_TIMEOUT, w.write_all(out.as_bytes())).await??;
    Ok(())
}

/// Acquire a persistent lock before considering removal of a stale socket.
pub async fn bind_socket(path: &std::path::Path) -> Result<(UnixListener, File)> {
    let dir = path.parent().context("socket needs a parent directory")?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o022 == 0,
        "socket directory must be owned by this user and not writable by others"
    );
    // Keep other users outside even during bind's initial umask-dependent mode.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))?;
    let meta = lock.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.nlink() == 1
            && meta.mode() & 0o077 == 0,
        "unsafe IPC lock file"
    );
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another mountd instance owns the IPC socket"
    );
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() },
                "refusing to replace non-socket or foreign IPC path"
            );
            match tokio::time::timeout(CONN_TIMEOUT, UnixStream::connect(path)).await {
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {}
                _ => bail!("IPC socket is active or cannot be proven stale"),
            }
            std::fs::remove_file(path)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, lock))
}

/// Run the IPC server forever. Replaces any stale socket file.
///
/// Unsafe paths, an existing instance, and bind errors are fatal. An `accept` error — `ECONNABORTED` when a client dies
/// between connect and accept, `EMFILE` under fd pressure — used to end the
/// loop and leave the daemon alive but permanently unreachable, with every
/// `mountctl` command reporting that it is not running.
pub async fn serve(engine: Arc<Engine>, path: PathBuf) -> Result<()> {
    let (listener, lock) = bind_socket(&path).await?;
    serve_bound(engine, listener, lock).await
}

/// Serve a socket acquired before starting reconciliation. Keep the lock alive.
pub async fn serve_bound(engine: Arc<Engine>, listener: UnixListener, lock: File) -> Result<()> {
    serve_bound_until(engine, listener, lock, std::future::pending()).await
}

/// Stop accepting on shutdown, then drain requests without cancelling mutations.
/// Callers must await this future before releasing their shutdown ownership guard.
pub async fn serve_bound_until(
    engine: Arc<Engine>,
    listener: UnixListener,
    lock: File,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let lock = Arc::new(lock);
    let mut connections = tokio::task::JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            result = connections.join_next(), if !connections.is_empty() => {
                log_connection(result);
            }
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!("ipc accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                if connections.len() >= 64 {
                    drop(stream);
                    continue;
                }
                let engine = engine.clone();
                let lock = lock.clone();
                connections.spawn(async move {
                    let _lock = lock;
                    serve_conn(engine, stream).await
                });
            }
        }
    }
    drop(listener);
    while let Some(result) = connections.join_next().await {
        log_connection(Some(result));
    }
    Ok(())
}

fn log_connection(result: Option<Result<Result<()>, tokio::task::JoinError>>) {
    match result {
        Some(Ok(Err(e))) => warn!("ipc connection error: {e:#}"),
        Some(Err(e)) => warn!("ipc connection task failed: {e}"),
        _ => {}
    }
}

/// A daemon that accepts a connection and then never answers must not hang the
/// caller forever — the menu bar app spawns one `mountctl` per refresh, and
/// each stuck one costs it a thread and a live child process.
///
/// The ceiling accommodates the slowest legitimate request: a `mount` that has
/// to wait on a Keychain ACL prompt.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Client side: send one request, read one response.
pub async fn client_request(path: &PathBuf, req: &Request) -> Result<Response> {
    client_request_with_timeout(path, req, CLIENT_TIMEOUT).await
}

async fn client_request_with_timeout(
    path: &PathBuf,
    req: &Request,
    deadline: Duration,
) -> Result<Response> {
    tokio::time::timeout(deadline, client_request_inner(path, req))
        .await
        .with_context(|| format!("mountd did not respond within {deadline:?}"))?
}

async fn client_request_inner(path: &PathBuf, req: &Request) -> Result<Response> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("connecting to {} (is mountd running?)", path.display()))?;
    same_user(&stream)?;
    let (r, mut w) = stream.into_split();
    let mut out = serde_json::to_string(req)?;
    ensure!(
        (out.len() as u64) < MAX_REQUEST_BYTES,
        "IPC request too large"
    );
    out.push('\n');
    w.write_all(out.as_bytes()).await?;
    let line = read_frame(r, MAX_RESPONSE_BYTES).await?;
    serde_json::from_str(line.trim()).context("parsing daemon response")
}

/// `explain --json` over many mounts is the largest legitimate response.
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::Engine;

    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[tokio::test]
    async fn framing_rejects_truncation_and_oversize() {
        assert!(read_frame(&b"{}"[..], 10)
            .await
            .unwrap_err()
            .to_string()
            .contains("newline"));
        assert!(read_frame(&b"{}   \n"[..], 5)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
        assert_eq!(read_frame(&b"{}\n"[..], 3).await.unwrap(), "{}\n");
    }

    #[tokio::test]
    async fn singleton_and_private_permissions() {
        let dir = private_dir();
        let path = dir.path().join("test.sock");
        let (listener, lock) = bind_socket(&path).await.unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(dir.path()).unwrap().mode() & 0o777, 0o700);
        let inode = std::fs::metadata(&path).unwrap().ino();
        assert!(bind_socket(&path).await.is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        drop(listener);
        drop(lock);
        let (_listener, _lock) = bind_socket(&path).await.unwrap();
    }

    #[tokio::test]
    async fn unsafe_paths_and_live_legacy_socket_are_preserved() {
        let dir = private_dir();
        let path = dir.path().join("test.sock");
        std::fs::write(&path, "keep").unwrap();
        assert!(bind_socket(&path).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
        std::fs::remove_file(&path).unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(bind_socket(&path).await.is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        std::fs::remove_file(&path).unwrap();
        let _legacy = UnixListener::bind(&path).unwrap();
        assert!(bind_socket(&path).await.is_err());
        assert!(path.exists());
    }

    #[tokio::test]
    async fn rejects_symlinked_parent_and_lock() {
        let dir = private_dir();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert!(bind_socket(&alias.join("test.sock")).await.is_err());
        let target = dir.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, real.join("test.lock")).unwrap();
        assert!(bind_socket(&real.join("test.sock")).await.is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
    }

    #[tokio::test]
    async fn client_deadline_bounds_an_idle_daemon() {
        let dir = private_dir();
        let path = dir.path().join("test.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let request =
            client_request_with_timeout(&path, &Request::Status, Duration::from_millis(50));
        let (response, peer) = tokio::join!(request, listener.accept());
        let _peer = peer.unwrap();
        assert!(response
            .unwrap_err()
            .to_string()
            .contains("did not respond"));
    }

    #[tokio::test]
    async fn graceful_shutdown_drains_accepted_requests_before_unlocking() {
        let cfg = Config::from_toml_str("").unwrap();
        let engine = Engine::new(PathBuf::from("/nonexistent"), cfg);
        let dir = private_dir();
        let path = dir.path().join("test.sock");
        let (listener, lock) = bind_socket(&path).await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut server = tokio::spawn(serve_bound_until(engine, listener, lock, async {
            let _ = rx.await;
        }));
        let mut stream = UnixStream::connect(&path).await.unwrap();
        stream.write_all(b"{\"cmd\":\"status\"}").await.unwrap();
        // A round trip on another connection establishes that the first was accepted.
        assert!(client_request(&path, &Request::Poke).await.unwrap().ok);
        tx.send(()).unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut server)
            .await
            .is_err());
        assert!(bind_socket(&path).await.is_err());
        stream.write_all(b"\n").await.unwrap();
        let line = read_frame(&mut stream, MAX_RESPONSE_BYTES).await.unwrap();
        assert!(serde_json::from_str::<Response>(&line).unwrap().ok);
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let (_listener, _lock) = bind_socket(&path).await.unwrap();
    }

    #[tokio::test]
    async fn idle_request_is_closed_at_the_transport_deadline() {
        let cfg = Config::from_toml_str("").unwrap();
        let engine = Engine::new(PathBuf::from("/nonexistent"), cfg);
        let (mut client, server) = UnixStream::pair().unwrap();
        tokio::time::timeout(CONN_TIMEOUT + Duration::from_secs(2), async {
            let error = serve_conn(engine, server).await.unwrap_err();
            assert!(error
                .downcast_ref::<tokio::time::error::Elapsed>()
                .is_some());
            assert_eq!(
                client.read_u8().await.unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
        })
        .await
        .unwrap();
    }

    #[test]
    fn request_serialization_round_trip() {
        let reqs = vec![
            Request::Status,
            Request::Mount { name: "x".into() },
            Request::Reload,
        ];
        for req in reqs {
            let s = serde_json::to_string(&req).unwrap();
            let back: Request = serde_json::from_str(&s).unwrap();
            assert_eq!(back, req);
        }
        // tagged form is stable for external clients (SwiftBar etc.)
        assert_eq!(
            serde_json::to_string(&Request::Status).unwrap(),
            r#"{"cmd":"status"}"#
        );
    }

    #[tokio::test]
    async fn server_round_trip_over_socket() {
        let cfg = Config::from_toml_str(
            r#"
[[mount]]
name = "a"
protocol = "smb"
share = "s"
mountpoint = "~/mnt/a"
  [[mount.endpoint]]
  address = "192.0.2.1"
"#,
        )
        .unwrap();
        let engine = Engine::new(PathBuf::from("/nonexistent"), cfg);
        let dir = private_dir();
        let path = dir.path().join("test.sock");

        let server = tokio::spawn(serve(engine, path.clone()));
        // wait for the socket to appear
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let resp = client_request(&path, &Request::Status).await.unwrap();
        assert!(resp.ok);
        let status = resp.status.unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].name, "a");

        let resp = client_request(&path, &Request::Poke).await.unwrap();
        assert!(resp.ok);

        server.abort();
    }

    #[tokio::test]
    async fn bad_request_gets_error_response() {
        let cfg = Config::from_toml_str("").unwrap();
        let engine = Engine::new(PathBuf::from("/nonexistent"), cfg);
        let dir = private_dir();
        let path = dir.path().join("test.sock");
        let server = tokio::spawn(serve(engine, path.clone()));
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let stream = UnixStream::connect(&path).await.unwrap();
        let (r, mut w) = stream.into_split();
        w.write_all(b"not json\n").await.unwrap();
        let mut line = String::new();
        BufReader::new(r).read_line(&mut line).await.unwrap();
        let resp: Response = serde_json::from_str(line.trim()).unwrap();
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("bad request"));

        server.abort();
    }
}
