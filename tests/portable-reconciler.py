#!/usr/bin/env python3
"""Run synthetic reconciler tests in a disposable crate, without native effects.

Production files are copied unchanged except for the portable fixture's power
module and SystemOps implementation, which panic if a test reaches real effects.
This checks engine logic, not Darwin mount enumeration or IOKit integration.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

os.environ.setdefault("TMPDIR", str(Path.home() / ".local/share/agent-tmp"))
Path(os.environ["TMPDIR"]).mkdir(parents=True, exist_ok=True)
os.environ.setdefault("CARGO_BUILD_JOBS", "1")
os.environ.setdefault("CARGO_INCREMENTAL", "0")
os.environ.setdefault("RUST_TEST_THREADS", "1")
source = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="mountd-reconciler-") as temporary:
    dest = Path(temporary)
    shutil.copytree(source / "src", dest / "src")
    for name in ("Cargo.toml", "Cargo.lock"):
        text = (source / name).read_text().replace('name = "mountd"', 'name = "mountd-reconciler-fixture"', 1)
        (dest / name).write_text(text)
    manifest = (dest / "Cargo.toml").read_text()
    start = manifest.index("[[bin]]")
    end = manifest.index("[dependencies]", start)
    manifest = manifest[:start] + manifest[end:]
    manifest = manifest.replace("[package]", "[package]\nautobins = false")
    (dest / "Cargo.toml").write_text(manifest)
    power = (dest / "src/power.rs").read_text()
    start = power.index("#[derive(Debug)]")
    end = power.index("// ----------------------------------------------------------------", start)
    (dest / "src/power.rs").write_text("use tokio::sync::oneshot;\n" + power[start:end])
    ops_path = dest / "src/ops.rs"
    ops = ops_path.read_text()
    start = ops.index("impl Ops for SystemOps {")
    end = ops.index("\n#[cfg(test)]", start)
    implementation = ops[start:end]
    # Retain every method signature, replace each body with a hard failure.
    position = 0
    while True:
        method = implementation.find("    fn ", position)
        if method == -1:
            break
        body = implementation.index("{", method)
        depth = 1
        tail = body + 1
        while depth:
            depth += (implementation[tail] == "{") - (implementation[tail] == "}")
            tail += 1
        replacement = '{ panic!("SystemOps forbidden in reconciler fixture") }'
        implementation = implementation[:body] + replacement + implementation[tail:]
        position = body + len(replacement)
    ops_path.write_text(ops[:start] + "#[allow(unused_variables)]\n" + implementation + ops[end:])
    env = os.environ.copy()
    env.setdefault("CARGO_TARGET_DIR", str(dest / "target"))
    base = ["cargo", "--offline", "--locked"]
    for area in ("engine", "reconcile", "state", "ops"):
        subprocess.run(base + ["test", "--lib", area + "::tests"], cwd=dest, env=env, check=True)
    subprocess.run(base + ["clippy", "--lib", "--tests", "--", "-D", "warnings"],
                   cwd=dest, env=env, check=True)
