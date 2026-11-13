#!/usr/bin/env python3
"""Inject new lifecycle regressions into the existing portable reconciler fixture.

Run under the shared heavy.lock. Production sources are never edited. A failure
is a regression result, so this runner preserves Cargo's nonzero exit status.
"""
import hashlib
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GENERATOR = ROOT / 'tests/portable-reconciler.py'
TESTS = ROOT / 'tests/audit/round1-lifecycle.rs'

for key, value in {
    'TMPDIR': os.environ.get('TMPDIR', str(Path.home() / '.local/share/agent-tmp')),
    'CARGO_BUILD_JOBS': '1',
    'CARGO_INCREMENTAL': '0',
    'RUST_TEST_THREADS': '1',
    'CARGO_TARGET_DIR': os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target/portable-daemon')),
}.items():
    os.environ[key] = value

for path in (ROOT / 'src/engine.rs', ROOT / 'src/ops.rs', ROOT / 'src/state.rs',
             GENERATOR, TESTS, Path(__file__)):
    print(hashlib.sha256(path.read_bytes()).hexdigest(), path, flush=True)

program = GENERATOR.read_text().replace('mountd-reconciler-fixture',
                                         'mountd-round1-lifecycle-fixture')
anchor = '    env = os.environ.copy()'
assert program.count(anchor) == 1
program = program.replace(anchor, '''    engine_path = dest / "src/engine.rs"
    engine_path.write_text(engine_path.read_text() + TESTS.read_text())
''' + anchor)
start = program.index('    for area in ("engine", "reconcile", "state", "ops"):')
program = program[:start] + '''    completed = subprocess.run(
        base + ["test", "--lib", "round1_lifecycle", "--", "--nocapture"],
        cwd=dest, env=env, timeout=180, check=False)
    result = completed.returncode
raise SystemExit(result)
'''
exec(compile(program, str(GENERATOR), 'exec'), {
    '__file__': str(GENERATOR), '__name__': '__main__', 'TESTS': TESTS,
})
