#!/usr/bin/env python3
"""Four pure-helper tests. No native binding, callback, engine or helper process."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = {
    "src/mtab.rs": "aab40c5d1ef589b85e8ee525e961ba8a66c6f26a0fae9a6e74b3e5828ef93a21",
    "src/power.rs": "80425f61482d8c53acfcb0a1f6a46411672d4f47bd05d70fe736919e52e601e0",
    "Cargo.lock": "6408d5a07ea8c80acd23da7efcdeff9165c474625964420ed1fd2abc85cec3e3",
    "Cargo.toml": "96600607dafc9fdf9131da60b851787f9611fcd38f41b040334ae838a4da1d3c",
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def source_check():
    for name, expected in EXPECTED.items():
        assert digest((ROOT / name).read_bytes()) == expected, name
        print("source_sha256", expected, name, flush=True)


def section(data, first, after, name):
    start = data.index(first)
    end = data.index(after, start)
    result = data[start:end]
    print("extraction", name, "offset", start, "bytes", len(result), "sha256", digest(result))
    return result


assert ROOT == Path("/home/christian/Documents/dev/mountd")
assert os.environ["TMPDIR"] == "/home/christian/.local/share/agent-tmp"
for name in ["CARGO_BUILD_JOBS", "RUST_TEST_THREADS"]:
    assert os.environ[name] == "1"
assert os.environ["CARGO_INCREMENTAL"] == "0"
source_check()
power = (ROOT / "src/power.rs").read_bytes()
mtab = (ROOT / "src/mtab.rs").read_bytes()
parts = [
    section(mtab, b"fn cstr_field(", b"/// List current mounts", "cstr_field"),
    section(power, b"#[derive(Debug)]", b"// ----------------", "PowerEvent"),
    section(power, b"const MAX_ACK_WAIT:", b'#[link(name = "IOKit"', "MAX_ACK_WAIT"),
    section(power, b"// Keep the acknowledgement", b"/// Register for system power", "ack_helpers"),
]
fixture_test = ROOT / "tests/audit/round4-native-boundary.rs"
for path in [Path(__file__), fixture_test]:
    print("fixture_sha256", digest(path.read_bytes()), str(path))
generated = b"use std::time::Duration;\nuse tokio::sync::{mpsc, oneshot};\n" + b"\n".join(parts) + fixture_test.read_bytes()
assert b'extern "C"' not in generated and b"getfsstat(" not in generated
packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
reviewed = {p["name"]: p for p in packages if p["name"] in ["libc", "tokio", "pin-project-lite"]}
crate = Path(tempfile.mkdtemp(prefix="mountd-round4-native-boundary-"))
print("owned_scratch", str(crate), flush=True)
(crate / "src").mkdir()
(crate / "src/lib.rs").write_bytes(generated)
manifest = '[package]\nname = "mountd-round4-native-boundary"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\n'
manifest += f'libc = "={reviewed["libc"]["version"]}"\n'
manifest += f'tokio = {{ version = "={reviewed["tokio"]["version"]}", default-features = false, features = ["sync"] }}\n'
(crate / "Cargo.toml").write_text(manifest)
lock = 'version = 4\n\n[[package]]\nname = "mountd-round4-native-boundary"\nversion = "0.0.0"\ndependencies = ["libc", "tokio"]\n'
for name, package in sorted(reviewed.items()):
    lock += "\n[[package]]\n"
    for key in ["name", "version", "source", "checksum"]:
        lock += f"{key} = {json.dumps(package[key])}\n"
    if name == "tokio":
        lock += 'dependencies = ["pin-project-lite"]\n'
(crate / "Cargo.lock").write_text(lock)
for name in ["src/lib.rs", "Cargo.toml", "Cargo.lock"]:
    print("generated_sha256", digest((crate / name).read_bytes()), name)
env = dict(os.environ, CARGO_TARGET_DIR=str(crate / "target"))
start = time.monotonic()
for argv in [["rustc", "--version"], ["cargo", "--version"]]:
    print("argv", json.dumps(argv), flush=True)
    subprocess.run(argv, check=True, env=env, cwd=crate)
argv = ["cargo", "test", "--offline", "--locked", "--lib", "--", "--test-threads=1", "--nocapture"]
print("cwd", str(crate), "argv", json.dumps(argv), flush=True)
result = subprocess.run(argv, env=env, cwd=crate)
print("cargo_exit", result.returncode, "elapsed_seconds", round(time.monotonic() - start, 3), flush=True)
assert (crate / "Cargo.lock").read_text() == lock, "locked dependency bytes changed"
for package in tomllib.loads(lock)["package"]:
    if "source" in package:
        expected = reviewed[package["name"]]
        assert all(package[key] == expected[key] for key in ["version", "source", "checksum"])
        print("dependency_verified", package["name"], package["version"], package["checksum"])
source_check()
if result.returncode == 0:
    shutil.rmtree(crate)
    print("owned_cleanup", str(crate), "removed after Cargo exited", flush=True)
else:
    print("owned_cleanup", str(crate), "retained inactive after failed Cargo", flush=True)
raise SystemExit(result.returncode)
