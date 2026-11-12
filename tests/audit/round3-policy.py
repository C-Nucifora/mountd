#!/usr/bin/env python3
"""One focused FakeOps policy suite, derived from the portable generator."""
from pathlib import Path
import hashlib
import os
import subprocess
import time

source = Path(__file__).resolve().parents[2]
assert source == Path("/home/christian/Documents/dev/mountd")
os.environ["CARGO_TARGET_DIR"] = str(source / "target/portable-daemon")
paths = sorted((source / "src").rglob("*.rs")) + [
    source / "Cargo.toml", source / "Cargo.lock",
    source / "tests/portable-reconciler.py", Path(__file__),
    source / "tests/audit/round3-policy.rs",
]
before = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}
for path, digest in before.items():
    print(f"sha256 {digest} {path.relative_to(source)}", flush=True)

base = (source / "tests/portable-reconciler.py").read_text()
prefix, tail = base.split('    env = os.environ.copy()\n', 1)
assert tail.startswith('    env.setdefault("CARGO_TARGET_DIR"')
prefix = prefix.replace('source = Path(__file__).resolve().parents[1]', f'source = Path({str(source)!r})')
prefix = prefix.replace('prefix="mountd-reconciler-"', 'prefix="mountd-round3-policy-"')
prefix = prefix.replace('mountd-reconciler-fixture', 'mountd-round3-policy-fixture')
assert 'panic!("SystemOps forbidden in reconciler fixture")' in prefix
prefix += '''    engine = dest / "src/engine.rs"
    original_ops = (source / "src/ops.rs").read_text()
    fake_start = original_ops.index("#[cfg(test)]")
    generated_ops = (dest / "src/ops.rs").read_text()
    assert generated_ops[generated_ops.index("#[cfg(test)]"):] == original_ops[fake_start:]
    system_impl = generated_ops.split("impl Ops for SystemOps {", 1)[1].split("#[cfg(test)]", 1)[0]
    assert system_impl.count("    fn ") == system_impl.count('panic!("SystemOps forbidden in reconciler fixture")') == 9
    print("SystemOps: all 9 methods hard panic; FakeOps: byte-identical", flush=True)
    engine.write_text(engine.read_text() + "\\n" + (source / "tests/audit/round3-policy.rs").read_text())
    print(f"fixture={dest}", flush=True)
    for name in ("src/engine.rs", "src/ops.rs", "src/power.rs", "Cargo.toml", "Cargo.lock"):
        print(f"generated_sha256 {hashlib.sha256((dest / name).read_bytes()).hexdigest()} {name}", flush=True)
    command = ["cargo", "--offline", "--locked", "test", "--lib", "engine::round3_policy::", "--", "--test-threads=1", "--nocapture"]
    print(f"cwd={dest} argv={command!r}", flush=True)
    result = subprocess.run(command, cwd=dest, env=os.environ.copy(), check=False)
    print(f"cargo_exit={result.returncode}", flush=True)
print("owned disposable crate removed", flush=True)
'''
started = time.monotonic()
exec(compile(prefix, str(source / "tests/portable-reconciler.py"), "exec"))
assert all(hashlib.sha256(path.read_bytes()).hexdigest() == digest for path, digest in before.items())
print(f"source_identity=unchanged elapsed_seconds={time.monotonic() - started:.3f}", flush=True)
raise SystemExit(result.returncode)
