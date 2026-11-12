#!/usr/bin/env python3
"""Extract unchanged Rust data contracts only. Run under the shared heavy lock."""
import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import tomllib

ROOT = Path('/home/christian/Documents/dev/mountd')
FILES = ('src/state.rs', 'src/engine.rs', 'src/ipc.rs', 'src/reconcile.rs',
         'src/bin/mountctl.rs', 'menubar/Sources/main.swift', 'Cargo.lock')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def between(text, start, end):
    assert text.count(start) == 1, start
    return text[text.index(start):text.index(end, text.index(start))]


def main():
    started = time.monotonic()
    assert Path.cwd() == ROOT
    source = {name: (ROOT / name).read_bytes() for name in FILES}
    for name, data in source.items():
        print(digest(data), name, flush=True)
        committed = subprocess.check_output(['git', 'show', f'2224097:{name}'], cwd=ROOT)
        assert data == committed, f'changed reviewed source: {name}'
    texts = {name: data.decode() for name, data in source.items()}
    pieces = {
        'state': texts['src/state.rs'].split('#[cfg(test)]')[0],
        'status_type': between(texts['src/engine.rs'],
            '#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]', 'pub struct Engine'),
        'status_method': between(texts['src/engine.rs'],
            '    pub async fn status(&self)', '    /// Force an immediate attempt'),
        'response': between(texts['src/ipc.rs'],
            '#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]\npub struct Response',
            'impl Response'),
        'explanation_types': between(texts['src/reconcile.rs'],
            '#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]\npub struct CondExplain',
            '/// Explain a mount\'s evaluation.'),
        'cli_error_guard': between(texts['src/bin/mountctl.rs'], '    if !resp.ok {', '    Ok(resp)'),
        'cli_required_status': between(texts['src/bin/mountctl.rs'],
            '            let status = resp.status.context', '            if json {'),
    }
    # Copy the actual serialization expression; do not implement another CLI.
    cli = texts['src/bin/mountctl.rs']
    expression = 'serde_json::to_string_pretty(&status)?'
    assert cli.count(expression) == 1
    for label, value in pieces.items():
        print('extract', label, digest(value.encode()), flush=True)
    tests = (ROOT / 'tests/audit/round3-status-menu.rs').read_text()
    print('fixture', digest(tests.encode()), flush=True)
    print('generator', digest(Path(__file__).read_bytes()), flush=True)
    scratch = Path(tempfile.mkdtemp(prefix='mountd-round3-status-menu-', dir=os.environ['TMPDIR']))
    (scratch / 'src').mkdir()
    (scratch / 'src/state.rs').write_text(pieces['state'])
    # Minimal storage shell only. Config parsing, engine construction, effects,
    # IPC transport, CLI invocation and Swift/AppKit are not exercised.
    lib = '''#![allow(dead_code)]
use anyhow::{Context, Result};
use serde::{Serialize, Deserialize};
use std::collections::BTreeMap;
use tokio::sync::{Mutex, RwLock};
mod state;
use state::{MountRuntime, MountState};
struct FixtureMount { name: String, mountpoint: String }
struct FixtureConfig { mounts: Vec<FixtureMount> }
struct Engine { cfg: RwLock<FixtureConfig>, runtime: Mutex<BTreeMap<String, MountRuntime>> }
'''
    lib += pieces['status_type'] + 'impl Engine {\n' + pieces['status_method'] + '}\n'
    lib += 'mod reconcile {\n' + pieces['explanation_types'] + '}\n' + pieces['response']
    lib += 'fn cli_json(resp: Response) -> Result<String> {\n' + pieces['cli_error_guard']
    lib += pieces['cli_required_status'] + 'Ok(' + expression + ')\n}\n' + tests
    (scratch / 'src/lib.rs').write_text(lib)
    (scratch / 'Cargo.toml').write_text('''[package]
name = "mountd-round3-status-menu-review-fixture"
version = "0.4.0"
edition = "2021"
[dependencies]
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["sync", "rt", "macros"] }
''')
    (scratch / 'Cargo.lock').write_text(texts['Cargo.lock'].replace(
        'name = "mountd"', 'name = "mountd-round3-status-menu-review-fixture"', 1))
    print('scratch', scratch, flush=True)
    print('generated', digest(lib.encode()), flush=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target/portable-daemon'))
    command = ['cargo', 'test', '--offline', '--manifest-path', str(scratch / 'Cargo.toml'),
               '--lib', '--', '--test-threads=1', '--nocapture']
    print('command', shlex.join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, env=env, timeout=220)
    print('cargo_exit', result.returncode, flush=True)
    original_packages = {(p['name'], p['version'], p.get('checksum'))
                         for p in tomllib.loads(texts['Cargo.lock'])['package'] if 'source' in p}
    fixture_lock = (scratch / 'Cargo.lock').read_bytes()
    for package in tomllib.loads(fixture_lock.decode())['package']:
        if 'source' in package:
            identity = (package['name'], package['version'], package.get('checksum'))
            assert identity in original_packages, f'dependency drift: {identity}'
    print('dependency_identity=pass', digest(fixture_lock), flush=True)
    for name, data in source.items():
        assert (ROOT / name).read_bytes() == data, f'source changed during run: {name}'
    print('source_identity_after=pass', flush=True)
    print('elapsed_seconds', round(time.monotonic() - started, 3), flush=True)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
