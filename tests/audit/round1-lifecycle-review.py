#!/usr/bin/env python3
"""One bounded review suite per invocation; caller must hold heavy.lock.

--without-fix removes only R1-L1's correction in the disposable crate and runs
one new negative control. Production and parent-owned fixtures are read-only.
"""
import hashlib
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
GENERATOR = ROOT / 'tests/portable-reconciler.py'
TESTS = ROOT / 'tests/audit/round1-lifecycle-review.rs'
WITHOUT_FIX = sys.argv[1:] == ['--without-fix']
if sys.argv[1:] and not WITHOUT_FIX:
    raise SystemExit('usage: round1-lifecycle-review.py [--without-fix]')
for key, value in {
    'TMPDIR': '/home/christian/.local/share/agent-tmp',
    'CARGO_BUILD_JOBS': '1',
    'CARGO_INCREMENTAL': '0',
    'RUST_TEST_THREADS': '1',
    'CARGO_TARGET_DIR': str(ROOT / 'target/portable-daemon'),
}.items():
    os.environ[key] = value

for path in sorted((ROOT / 'src').rglob('*.rs')) + [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', GENERATOR, TESTS, Path(__file__)]:
    print(hashlib.sha256(path.read_bytes()).hexdigest(), path, flush=True)
print('negative control, correction removed:', WITHOUT_FIX, flush=True)
program = GENERATOR.read_text().replace('mountd-reconciler-fixture', 'mountd-round1-lifecycle-review-fixture')
anchor = '    env = os.environ.copy()'
assert program.count(anchor) == 1
program = program.replace(anchor, '''    engine_path = dest / "src/engine.rs"
    engine = engine_path.read_text()
    if WITHOUT_FIX:
        correction = """        // Accepting a new mount request ends the earlier manual unmount intent.
        self.runtime
            .lock()
            .await
            .entry(name.to_string())
            .or_default()
            .user_suppressed = false;

"""
        assert engine.count(correction) == 1
        engine = engine.replace(correction, "", 1)
    engine_path.write_text(engine + TESTS.read_text())
''' + anchor)
start = program.index('    for area in ("engine", "reconcile", "state", "ops"):')
program = program[:start] + '''    selected = "round1_lifecycle_review"
    if WITHOUT_FIX:
        selected += "::accepted_failures_clear_intent_without_bypassing_gates"
    command = base + ["test", "--lib", selected, "--", "--nocapture"]
    print("fixture:", dest, "command:", command, flush=True)
    result = subprocess.run(command, cwd=dest, env=env, timeout=180, check=False).returncode
raise SystemExit(result)
'''
exec(compile(program, str(GENERATOR), 'exec'), {
    '__file__': str(GENERATOR), '__name__': '__main__',
    'TESTS': TESTS, 'WITHOUT_FIX': WITHOUT_FIX,
})
