use mountd::{config::Config, engine::Engine, ipc};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;
use tokio::sync::oneshot;

pub fn config(name: &str) -> Result<Config, mountd::config::ConfigError> {
    // Use TOML's serializer, since JSON's \b and \f escapes are not TOML escapes.
    let value = toml::Value::String(name.into()).to_string();
    Config::from_toml_str(&format!("[[mount]]\nname={value}\nprotocol='nfs'\nexport='/synthetic'\nmountpoint='/synthetic'\n[[mount.endpoint]]\naddress='127.0.0.1'\n"))
}

pub fn generated(seed: u64, encoded_budget: usize) -> String {
    let alphabet = [
        'a',
        '"',
        '\\',
        '日',
        '😀',
        '\u{2028}',
        '\u{2029}',
        '\u{200d}',
        '\u{feff}',
        '\u{10ffff}',
        'é',
        '\u{301}',
    ];
    let mut state = seed;
    let mut name = String::from("--");
    let mut used = 2;
    while used < encoded_budget {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let c = alphabet[state as usize % alphabet.len()];
        let cost = serde_json::to_string(&c.to_string()).unwrap().len() - 2;
        if used + cost > encoded_budget {
            name.push_str(&"x".repeat(encoded_budget - used));
            break;
        }
        name.push(c);
        used += cost;
    }
    name
}

pub fn requests(name: &str) -> [ipc::Request; 3] {
    [
        ipc::Request::Mount { name: name.into() },
        ipc::Request::Unmount { name: name.into() },
        ipc::Request::Explain {
            name: Some(name.into()),
        },
    ]
}

pub struct Server {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub engine: Arc<Engine>,
    pub stop: oneshot::Sender<()>,
    pub task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Server {
    pub async fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir
            .path()
            .join("Library/Application Support/mountd/mountd.sock");
        let (listener, lock) = ipc::bind_socket(&path).await.unwrap();
        let engine = Engine::new(PathBuf::from("/unused"), config(name).unwrap());
        let (stop, rx) = oneshot::channel();
        let task = tokio::spawn(ipc::serve_bound_until(
            engine.clone(),
            listener,
            lock,
            async {
                let _ = rx.await;
            },
        ));
        Self {
            dir,
            path,
            engine,
            stop,
            task,
        }
    }

    pub async fn finish(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(12), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let _replacement = ipc::bind_socket(&self.path).await.unwrap();
    }
}

pub async fn count(value: &std::sync::atomic::AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while value.load(Ordering::SeqCst) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
