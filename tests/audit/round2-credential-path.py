#!/usr/bin/env python3
"""Focused source-copy fixture, derived from tests/portable-process.py.

Run only while holding the shared heavy lock. Never invokes native security or
mount helpers. Production code is copied byte-for-byte, then tests appended.
"""
from pathlib import Path
import hashlib
import os
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
TEMP = Path(os.environ["TMPDIR"])
MODULES = ("config", "mountcmd", "cred", "probe", "deps", "fingerprint", "watcher", "mtab", "wol")
PACKAGE = "mountd-round2-credential-path-breaker-fixture"
SOURCES = ["Cargo.toml", "Cargo.lock", "tests/portable-process.py", "src/engine.rs", "src/ops.rs", "tests/audit/round2-credential-path.py", "tests/audit/round2-credential-path-cred.rs", "tests/audit/round2-credential-path-mount.rs"] + [f"src/{m}.rs" for m in MODULES]
original = {name: (ROOT / name).read_bytes() for name in SOURCES}
for name, data in original.items():
    print(f"sha256 {hashlib.sha256(data).hexdigest()} {name}", flush=True)


def interrupted(signum, _frame):
    raise TimeoutError(f"runner received signal {signum}")


signal.signal(signal.SIGTERM, interrupted)
started = time.monotonic()
with tempfile.TemporaryDirectory(prefix="mountd-round2-credential-path-", dir=TEMP) as directory:
    fixture = Path(directory)
    print(f"owned fixture {fixture}", flush=True)
    (fixture / "src").mkdir()
    manifest = original["Cargo.toml"].decode()
    manifest = manifest[:manifest.index("[lib]")] + manifest[manifest.index("[dependencies]"):]
    (fixture / "Cargo.toml").write_text(manifest.replace('name = "mountd"', f'name = "{PACKAGE}"'))
    (fixture / "Cargo.lock").write_text(original["Cargo.lock"].decode().replace('name = "mountd"', f'name = "{PACKAGE}"'))
    for module in MODULES:
        data = original[f"src/{module}.rs"]
        if module in ("cred", "mountcmd"):
            suffix = "cred" if module == "cred" else "mount"
            data += b"\n" + (ROOT / f"tests/audit/round2-credential-path-{suffix}.rs").read_bytes()
        (fixture / f"src/{module}.rs").write_bytes(data)
    (fixture / "src/lib.rs").write_text(
        "pub type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;\n"
        "pub mod engine { pub struct Engine { pub hint: tokio::sync::Notify } }\n"
        + "".join(f"pub mod {m};\n" for m in MODULES))
    # Every Rust test's TempDir is under this owner's root, not shared scratch.
    scratch = fixture / "scratch"
    scratch.mkdir(mode=0o700)
    env = dict(os.environ, TMPDIR=str(scratch), CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0",
               RUST_TEST_THREADS="1", CARGO_TARGET_DIR=str(ROOT / "target/portable-daemon"))
    command = ["cargo", "test", "--locked", "--offline", "--lib", "round2_credential_path", "--", "--nocapture"]
    print(f"cwd={fixture} command={command!r}", flush=True)
    process = subprocess.Popen(command, cwd=fixture, env=env, start_new_session=True)
    try:
        code = process.wait(timeout=215)
    except BaseException:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
        raise
    finally:
        # Fake helpers are finite foreground children, at most three seconds.
        # Cargo normally joins all tests. On outer cancellation allow that bound
        # before removing only this fixture's scratch.
        if process.returncode is not None and process.returncode < 0:
            time.sleep(4)
    print(f"cargo exit={code} elapsed={time.monotonic() - started:.3f}s", flush=True)
    for name, data in original.items():
        assert (ROOT / name).read_bytes() == data, f"source changed during run: {name}"
    print("source bytes unchanged", flush=True)
print(f"owned fixture removed={not fixture.exists()}", flush=True)
raise SystemExit(code)
