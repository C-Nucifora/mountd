#!/usr/bin/env python3
"""Exercise the real daemon entry point, IPC and process drain with fake effects.

Only SystemOps, power notifications and the route executable are replaced in a
temporary source copy. No native API, mount, credential or LaunchAgent is used.
This checks startup/shutdown orchestration, not macOS runtime acceptance.
"""
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SOCKET = "Library/Application Support/mountd/mountd.sock"

FAKE_OPS = r'''
impl Ops for SystemOps {
    fn network_snapshot(&self) -> BoxFut<crate::fingerprint::NetSnapshot> {
        Box::pin(async { crate::fingerprint::NetSnapshot::default() })
    }
    fn check_deps(&self, _: crate::config::Config)
        -> BoxFut<std::collections::BTreeMap<String, bool>> {
        Box::pin(async { std::collections::BTreeMap::new() })
    }
    fn list_mounts(&self) -> std::io::Result<Vec<MountEntry>> { Ok(vec![]) }
    fn prepare_mount(&self, _: MountSpec, _: OperationLease) -> BoxFut<Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn mount(&self, _: MountSpec, _: String, _: Option<String>, lease: OperationLease)
        -> BoxFut<Result<()>> {
        Box::pin(async move {
            let path = std::env::var("MOUNTD_FIXTURE_CHILD_PID")?;
            let mut command = tokio::process::Command::new("/bin/sh");
            command.args(["-c", "printf '%s' $$ > \"$1\"; exec /bin/sleep 30",
                          "synthetic-mount", &path]);
            crate::mountcmd::command_output(command, Duration::from_secs(30), lease).await?;
            Ok(())
        })
    }
    fn unmount(&self, _: String, _: bool, _: OperationLease) -> BoxFut<Result<()>> {
        panic!("fixture forbids unmounts")
    }
    fn fetch_password(&self, _: String, _: String, _: Duration, _: OperationLease) -> BoxFut<FetchOutcome> {
        panic!("fixture forbids credentials")
    }
    fn probe(&self, _: String, _: u16, _: Duration) -> BoxFut<bool> {
        Box::pin(async { true })
    }
    fn send_wol(&self, _: WolSpec) -> BoxFut<Result<()>> {
        panic!("fixture forbids network packets")
    }
}
'''


def wait_for(predicate, process, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        assert process.poll() is None, "daemon exited before fixture became ready"
        time.sleep(0.02)
    raise AssertionError("fixture readiness deadline exceeded")


def request(path, payload):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(3)
        client.connect(str(path))
        client.sendall(json.dumps(payload).encode() + b"\n")
        with client.makefile("rb") as response:
            return json.loads(response.readline(65536))


def main():
    scratch_root = Path(os.environ.get("TMPDIR", Path.home() / ".local/share/agent-tmp"))
    scratch_root.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TMPDIR=str(scratch_root), CARGO_BUILD_JOBS="1",
               CARGO_INCREMENTAL="0", RUST_TEST_THREADS="1", TOKIO_WORKER_THREADS="2")
    with tempfile.TemporaryDirectory(prefix="mountd-daemon-", dir=scratch_root) as temp:
        fixture = Path(temp)
        shutil.copytree(ROOT / "src", fixture / "src")
        for name in ("Cargo.toml", "Cargo.lock"):
            text = (ROOT / name).read_text().replace('name = "mountd"', 'name = "mountd-daemon-fixture"', 1)
            (fixture / name).write_text(text)
        ops = fixture / "src/ops.rs"
        text = ops.read_text()
        start = text.index("impl Ops for SystemOps {")
        end = text.index("\n#[cfg(test)]", start)
        ops.write_text(text[:start] + FAKE_OPS + text[end:])
        power = fixture / "src/power.rs"
        text = power.read_text()
        start = text.index("#[derive(Debug)]")
        end = text.index("// ----------------------------------------------------------------", start)
        power.write_text("use tokio::sync::{mpsc, oneshot};\n" + text[start:end] + r'''
pub fn watch() -> mpsc::UnboundedReceiver<PowerEvent> {
    let (_send, receive) = mpsc::unbounded_channel();
    receive
}
''')
        route = fixture / "synthetic-route"
        route.write_text('#!/bin/sh\nprintf "%s" $$ > "$MOUNTD_FIXTURE_WATCH_PID"\nexec /bin/sleep 30\n')
        route.chmod(0o700)
        watcher = fixture / "src/watcher.rs"
        text = watcher.read_text()
        assert text.count('Command::new("/sbin/route")') == 1
        watcher.write_text(text.replace('Command::new("/sbin/route")',
                                       "Command::new(" + json.dumps(str(route)) + ")"))
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/portable-daemon"))
        subprocess.run(["cargo", "build", "--offline", "--locked", "--bin", "mountd"],
                       cwd=fixture, env=env, check=True)
        binary = Path(env["CARGO_TARGET_DIR"]) / "debug/mountd"
        # Keep the absolute Unix socket name below both Darwin and Linux limits.
        with tempfile.TemporaryDirectory(prefix=".md-", dir=Path.home()) as home_text:
            home = Path(home_text)
            env.update(HOME=str(home), MOUNTD_FIXTURE_CHILD_PID=str(home / "child.pid"),
                       MOUNTD_FIXTURE_WATCH_PID=str(home / "watch.pid"))
            config = home / "config.toml"
            config.write_text('''[[mount]]
name = "synthetic"
protocol = "nfs"
export = "/synthetic"
mountpoint = "~/mnt/synthetic"
[[mount.endpoint]]
address = "127.0.0.1"
''')
            path = home / SOCKET
            assert len(os.fsencode(path)) < 104
            path.parent.mkdir(parents=True, mode=0o700)
            args = [str(binary), "run", "--config", str(config)]
            lock = path.with_suffix(".lock")
            with lock.open("w") as held:
                lock.chmod(0o600)
                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                refused = subprocess.run(args, cwd=home, env=env, capture_output=True, timeout=10)
                assert refused.returncode != 0
                assert b"another mountd instance" in refused.stderr
                assert not (home / "child.pid").exists()
                assert not (home / "watch.pid").exists()
            with (home / "daemon.log").open("wb") as log:
                daemon = subprocess.Popen(args, cwd=home, env=env, stdout=log, stderr=log)
                child_handles = []
                try:
                    wait_for(lambda: path.exists() and (home / "child.pid").exists()
                             and (home / "watch.pid").exists(), daemon)
                    if hasattr(os, "pidfd_open"):
                        child_handles = [os.pidfd_open(int((home / marker).read_text()))
                                         for marker in ("child.pid", "watch.pid")]
                    assert request(path, {"cmd": "status"})["ok"]
                    inode = path.stat().st_ino
                    refused = subprocess.run(args, cwd=home, env=env, capture_output=True, timeout=10)
                    assert refused.returncode != 0
                    assert path.stat().st_ino == inode
                    assert request(path, {"cmd": "status"})["ok"]
                    # Hold one accepted frame open so drain cannot finish yet.
                    with socket.socket(socket.AF_UNIX) as slow:
                        slow.settimeout(3)
                        slow.connect(str(path))
                        slow.sendall(b'{"cmd":"sta')
                        time.sleep(0.1)
                        daemon.send_signal(signal.SIGTERM)
                        time.sleep(0.1)
                        assert daemon.poll() is None
                        with lock.open("r+") as held:
                            try:
                                fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                            except BlockingIOError:
                                pass
                            else:
                                raise AssertionError("singleton released before IPC drain")
                        slow.sendall(b'tus"}\n')
                        assert slow.recv(65536)
                    assert daemon.wait(timeout=10) == 0
                    for marker in ("child.pid", "watch.pid"):
                        pid = int((home / marker).read_text())
                        try:
                            os.kill(pid, 0)
                        except ProcessLookupError:
                            pass
                        else:
                            raise AssertionError(f"owned child {pid} survived daemon drain")
                    with lock.open("r+") as held:
                        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    assert path.stat().st_ino == inode, "shutdown must preserve socket inode"
                finally:
                    if daemon.poll() is None:
                        daemon.terminate()
                        try:
                            daemon.wait(timeout=12)
                        except subprocess.TimeoutExpired:
                            daemon.kill()
                            daemon.wait()
                    # pidfds cannot accidentally target a reused PID on failure.
                    for handle in child_handles:
                        try:
                            signal.pidfd_send_signal(handle, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        finally:
                            os.close(handle)
    print("synthetic daemon singleton, IPC drain and child-reaping fixtures passed")


if __name__ == "__main__":
    main()
