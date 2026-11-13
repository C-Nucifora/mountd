#!/usr/bin/env python3
"""Run unchanged portable modules in an isolated crate; no macOS emulation.

The only stub is the watcher notification target. Production mount enumeration
still returns Unsupported on Linux. Keychain integration tests stay ignored.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TEMP_ROOT = Path(os.environ.get("TMPDIR", str(Path.home() / ".local/share/agent-tmp")))
TEMP_ROOT.mkdir(parents=True, exist_ok=True)

MODULES = ("config", "mountcmd", "cred", "probe", "deps", "fingerprint", "watcher", "mtab", "wol")

with tempfile.TemporaryDirectory(prefix="mountd-process-fixture-", dir=TEMP_ROOT) as directory:
    fixture = Path(directory)
    (fixture / "src").mkdir()
    manifest = (ROOT / "Cargo.toml").read_text()
    start = manifest.index("[lib]")
    end = manifest.index("[dependencies]")
    manifest = (manifest[:start] + manifest[end:]).replace('name = "mountd"', 'name = "mountd-process-fixture"')
    (fixture / "Cargo.toml").write_text(manifest)
    (fixture / "Cargo.lock").write_text((ROOT / "Cargo.lock").read_text().replace('name = "mountd"', 'name = "mountd-process-fixture"'))
    for module in MODULES:
        shutil.copyfile(ROOT / "src" / f"{module}.rs", fixture / "src" / f"{module}.rs")
    (fixture / "src/lib.rs").write_text(
        "pub type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;\n"
        "pub mod engine { pub struct Engine { pub hint: tokio::sync::Notify } }\n"
        + "".join(f"pub mod {module};\n" for module in MODULES)
    )
    env = dict(os.environ, TMPDIR=str(TEMP_ROOT), CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0", RUST_TEST_THREADS="1")
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target" / "portable-process"))
    subprocess.run(["cargo", "test", "--locked", "--offline", "--lib"], cwd=fixture, env=env, check=True)
    subprocess.run(["cargo", "clippy", "--locked", "--offline", "--all-targets", "--", "-D", "warnings"], cwd=fixture, env=env, check=True)
