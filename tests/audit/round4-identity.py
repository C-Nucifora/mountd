#!/usr/bin/env python3
"""One source-derived pure configuration/identity suite; invoke under heavy.lock."""
import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import tomllib

ROOT = Path('/home/christian/Documents/dev/mountd')
BASE = '6c51d55a64321e6d8cbff1773ad67bb8d5467c8c'
FILES = ('src/config.rs', 'src/mtab.rs', 'src/mountcmd.rs', 'src/engine.rs',
         'src/wol.rs', 'Cargo.toml', 'Cargo.lock')
PACKAGE = 'mountd-round4-identity-review-fixture'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def between(text, start, end):
    assert text.count(start) == 1, start
    first = text.index(start)
    assert end in text[first:], end
    return text[first:text.index(end, first)]


def main():
    started = time.monotonic()
    assert Path.cwd() == ROOT
    assert subprocess.check_output(['git', 'remote', 'get-url', 'origin'], cwd=ROOT, timeout=5).decode().strip() == 'https://github.com/C-Nucifora/mountd.git'
    assert subprocess.check_output(['git', 'branch', '--show-current'], cwd=ROOT, timeout=5).decode().strip() == 'hardening-audit-rounds'
    subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, 'HEAD'], cwd=ROOT, check=True, timeout=5)
    source = {name: (ROOT / name).read_bytes() for name in FILES}
    for name, data in source.items():
        assert data == subprocess.check_output(['git', 'show', f'{BASE}:{name}'], cwd=ROOT, timeout=5), name
        print(digest(data), name, flush=True)
    texts = {name: data.decode() for name, data in source.items()}
    pieces = {
        'config.rs': texts['src/config.rs'].split('#[cfg(test)]')[0],
        'mtab_type': between(texts['src/mtab.rs'], '#[derive(Debug, Clone, PartialEq, Eq)]', '#[cfg(target_os = "macos")]'),
        'mtab_pure': between(texts['src/mtab.rs'], '/// Trailing slashes are not significant', '#[cfg(test)]'),
        'mountcmd_pure': between(texts['src/mountcmd.rs'], '/// Expand a leading', '/// Shared engine guards'),
        'mountcmd_paths': between(texts['src/mountcmd.rs'], '/// Refuse possible aliases', '/// Bound the caller wait'),
        'wol_schema': between(texts['src/wol.rs'], 'fn default_broadcast()', '/// Build the 102-byte'),
    }
    for label, value in pieces.items():
        print('extract', label, digest(value.encode()), flush=True)
    fixture = (ROOT / 'tests/audit/round4-identity.rs').read_bytes()
    print('fixture', digest(fixture), flush=True)
    print('generator', digest(Path(__file__).read_bytes()), flush=True)
    scratch = Path(tempfile.mkdtemp(prefix='mountd-round4-identity-', dir=os.environ['TMPDIR']))
    print('scratch', scratch, flush=True)
    (scratch / 'src').mkdir()
    generated = {
        'src/config.rs': pieces['config.rs'],
        'src/mtab.rs': pieces['mtab_type'] + pieces['mtab_pure'],
        'src/mountcmd.rs': 'use std::path::{Path, PathBuf};\nuse anyhow::{bail, Context, Result};\nuse crate::config::{MountSpec, Protocol};\n' + pieces['mountcmd_pure'] + pieces['mountcmd_paths'],
        'src/wol.rs': 'use serde::Deserialize;\n' + pieces['wol_schema'],
        'src/lib.rs': '#![allow(dead_code)]\nmod config;\nmod mountcmd;\nmod mtab;\nmod wol;\n#[cfg(test)] mod identity;\n',
        'src/identity.rs': fixture.decode(),
        'Cargo.toml': f'''[package]
name = "{PACKAGE}"
version = "0.4.0"
edition = "2021"
[dependencies]
anyhow = "1"
thiserror = "2"
serde = {{ version = "1", features = ["derive"] }}
toml = "0.8"
dirs = "6"
''',
        'Cargo.lock': texts['Cargo.lock'].replace('name = "mountd"', f'name = "{PACKAGE}"', 1),
    }
    for name, value in generated.items():
        (scratch / name).write_text(value)
        print('generated', name, digest(value.encode()), flush=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target/portable-daemon'))
    manifest = str(scratch / 'Cargo.toml')
    # Prune the seeded lock without compilation; verify all identities before tests.
    metadata = ['cargo', 'metadata', '--offline', '--format-version=1', '--manifest-path', manifest]
    print('command', shlex.join(metadata), flush=True)
    subprocess.run(metadata, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL, timeout=15)
    registry = {(p['name'], p['version'], p.get('source'), p.get('checksum'))
                for p in tomllib.loads(texts['Cargo.lock'])['package'] if 'source' in p}
    lock = (scratch / 'Cargo.lock').read_bytes()
    for package in tomllib.loads(lock.decode())['package']:
        if 'source' in package:
            identity = (package['name'], package['version'], package['source'], package.get('checksum'))
            assert identity in registry, f'dependency drift: {identity}'
            print('dependency', *identity, flush=True)
    print('dependency_identity_before=pass', digest(lock), flush=True)
    command = ['cargo', 'test', '--offline', '--locked', '--manifest-path', manifest,
               '--lib', '--', '--test-threads=1', '--nocapture']
    print('command', shlex.join(command), flush=True)
    # Outer timeout owns the whole command group; scratch is retained on any exit.
    result = subprocess.run(command, cwd=ROOT, env=env)
    print('cargo_exit', result.returncode, flush=True)
    assert (scratch / 'Cargo.lock').read_bytes() == lock
    for name, data in source.items():
        assert (ROOT / name).read_bytes() == data, f'source changed during run: {name}'
    print('source_identity_after=pass', flush=True)
    print('elapsed_seconds', round(time.monotonic() - started, 3), flush=True)
    print('scratch_retained', scratch, flush=True)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
