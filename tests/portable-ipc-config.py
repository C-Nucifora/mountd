#!/usr/bin/env python3
"""Run real config/IPC/CLI sources with fake engine/launchd boundaries on Unix, without macOS effects.

The temporary crate is test-only. It does not replace production platform behavior.
No mount, probe, Keychain or launchd operations are available to its fake engine.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

FAKE_ENGINE = r'''
pub mod engine {
    use std::{path::PathBuf, sync::Arc};
    use crate::{config::Config, reconcile::Explanation};
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub struct MountStatus {
        pub name: String, pub state: String, pub mountpoint: String,
        pub mounted_via: Option<String>, pub last_error: Option<String>, pub state_kind: String,
    }
    pub struct Engine { pub hint: tokio::sync::Notify, cfg: Config }
    impl Engine {
        pub fn new(_: PathBuf, cfg: Config) -> Arc<Self> {
            Arc::new(Self { hint: tokio::sync::Notify::new(), cfg })
        }
        pub async fn status(&self) -> Vec<MountStatus> {
            self.cfg.mounts.iter().map(|m| MountStatus {
                name: m.name.clone(), state: "idle".into(), mountpoint: m.mountpoint.clone(),
                mounted_via: None, last_error: None, state_kind: "idle".into(),
            }).collect()
        }
        pub async fn explain(&self, _: Option<&str>) -> anyhow::Result<Vec<Explanation>> {
            anyhow::bail!("fixture forbids real probes")
        }
        pub async fn force_mount(&self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture forbids real mounts")
        }
        pub async fn force_unmount(&self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("fixture forbids real unmounts")
        }
        pub async fn reload(&self) -> anyhow::Result<()> {
            anyhow::bail!("fixture forbids real reload")
        }
    }
}
pub mod launchd {
    pub const LABEL: &str = "org.mountd.agent";
    pub async fn install() -> anyhow::Result<()> {
        let marker = std::path::PathBuf::from(std::env::var("MOUNTD_FIXTURE_DRAIN")?);
        let activity = crate::mountcmd::CommandActivity::begin();
        std::thread::spawn(move || {
            let _activity = activity;
            std::fs::write(marker.with_extension("started"), "ready").unwrap();
            let limit = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !marker.with_extension("release").exists() && std::time::Instant::now() < limit {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::fs::write(marker, "finished").unwrap();
        });
        anyhow::bail!("synthetic install failure with cleanup pending")
    }
    pub async fn uninstall() -> anyhow::Result<()> {
        anyhow::bail!("fixture forbids native launchd")
    }
}
pub mod mtab {
    pub struct MountEntry { pub on: String, pub fstype: String }
    pub fn list() -> std::io::Result<Vec<MountEntry>> {
        panic!("fixture forbids native mount enumeration")
    }
    pub fn find_by_mountpoint<'a>(_: &'a [MountEntry], _: &str) -> Option<&'a MountEntry> {
        panic!("fixture forbids native mount lookup")
    }
}
'''


def main():
    manifest = (ROOT / "Cargo.toml").read_text()
    assert 'name = "mountd"' in manifest
    # Only the control CLI is built. Native daemon entry points are excluded.
    start, end = manifest.index("[lib]"), manifest.index("[dependencies]")
    scratch_root = Path(os.environ.get("TMPDIR", str(Path.home() / ".local/share/agent-tmp")))
    scratch_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mountd-ipc-", dir=scratch_root) as scratch:
        fixture = Path(scratch)
        (fixture / "src").mkdir()
        (fixture / "tests").mkdir()
        manifest = manifest[:start] + (
            '[lib]\nname = "mountd"\npath = "src/lib.rs"\n\n'
            '[[bin]]\nname = "mountctl"\npath = '
            + json.dumps(str(ROOT / "src/bin/mountctl.rs")) + "\n\n"
        ) + manifest[end:]
        # Distinct package identity prevents fixture-library cache collisions.
        (fixture / "Cargo.toml").write_text(manifest.replace('name = "mountd"', 'name = "mountd-ipc-fixture"', 1))
        (fixture / "Cargo.lock").write_text((ROOT / "Cargo.lock").read_text().replace('name = "mountd"', 'name = "mountd-ipc-fixture"', 1))
        lib = 'pub type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;\n'
        for module in ("config", "ipc", "reconcile", "probe", "wol", "mountcmd"):
            lib += '#[path = ' + json.dumps(str(ROOT / f"src/{module}.rs")) + f'] pub mod {module};\n'
        (fixture / "src/lib.rs").write_text(lib + FAKE_ENGINE)
        shutil.copyfile(ROOT / "tests/ipc_cli.rs", fixture / "tests/ipc_cli.rs")
        (fixture / "tests/cli_drain.rs").write_text(r'''#[tokio::test]
async fn failed_install_keeps_the_actual_cli_alive_until_registered_cleanup_finishes() {
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("cleanup");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_mountctl"))
        .arg("install").env("MOUNTD_FIXTURE_DRAIN", &marker)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .kill_on_drop(true).spawn().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        while !marker.with_extension("started").exists() {
            anyhow::ensure!(child.try_wait()?.is_none(), "CLI exited before cleanup started");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        anyhow::ensure!(tokio::time::timeout(Duration::from_millis(50), child.wait()).await.is_err(),
            "CLI exited while cleanup was held");
        std::fs::write(marker.with_extension("release"), "release")?;
        anyhow::ensure!(!child.wait().await?.success(), "synthetic install should fail");
        anyhow::ensure!(marker.exists(), "CLI returned before cleanup finished");
        Ok::<_, anyhow::Error>(())
    }).await;
    if !matches!(result, Ok(Ok(()))) { let _ = child.kill().await; }
    result.unwrap().unwrap();
}
''')
        env = os.environ.copy()
        env.setdefault("TMPDIR", str(scratch_root))
        env.setdefault("CARGO_BUILD_JOBS", "1")
        env.setdefault("CARGO_INCREMENTAL", "0")
        env.setdefault("RUST_TEST_THREADS", "1")
        # Build cache only. Runtime data, HOME and sockets remain temporary.
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/portable-ipc-config"))
        cargo = ["cargo", "--offline", "--locked", "--manifest-path", str(fixture / "Cargo.toml")]
        for args in (("test", "--lib", "config::tests"), ("test", "--lib", "ipc::tests"),
                     ("test", "--test", "ipc_cli"), ("test", "--test", "cli_drain"), ("clippy", "--all-targets", "--", "-D", "warnings")):
            subprocess.run([cargo[0], args[0], *cargo[1:], *args[1:]], env=env, check=True)


if __name__ == "__main__":
    main()
