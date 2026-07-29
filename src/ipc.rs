//! Unix-socket IPC between the daemon and `mountctl`: one newline-delimited
//! JSON request per connection, one JSON response back.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::warn;

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
    Mount { name: String },
    Unmount { name: String },
    Reload,
    Poke,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<MountStatus>>,
}

impl Response {
    fn ok() -> Self {
        Response {
            ok: true,
            error: None,
            status: None,
        }
    }
    fn err(e: impl ToString) -> Self {
        Response {
            ok: false,
            error: Some(e.to_string()),
            status: None,
        }
    }
}

pub async fn handle_request(engine: &Engine, req: Request) -> Response {
    match req {
        Request::Status => Response {
            ok: true,
            error: None,
            status: Some(engine.status().await),
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

async fn serve_conn(engine: Arc<Engine>, stream: UnixStream) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await?;
    let resp = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => handle_request(&engine, req).await,
        Err(e) => Response::err(format!("bad request: {e}")),
    };
    let mut out = serde_json::to_string(&resp)?;
    out.push('\n');
    w.write_all(out.as_bytes()).await?;
    Ok(())
}

/// Run the IPC server forever. Replaces any stale socket file.
pub async fn serve(engine: Arc<Engine>, path: PathBuf) -> Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let _ = tokio::fs::remove_file(&path).await;
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("binding {}", path.display()))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let engine = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(engine, stream).await {
                warn!("ipc connection error: {e:#}");
            }
        });
    }
}

/// Client side: send one request, read one response.
pub async fn client_request(path: &PathBuf, req: &Request) -> Result<Response> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("connecting to {} (is mountd running?)", path.display()))?;
    let (r, mut w) = stream.into_split();
    let mut out = serde_json::to_string(req)?;
    out.push('\n');
    w.write_all(out.as_bytes()).await?;
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await?;
    serde_json::from_str(line.trim()).context("parsing daemon response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::Engine;

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
        let dir = tempfile::tempdir().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
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
