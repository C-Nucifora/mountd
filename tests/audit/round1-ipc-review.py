#!/usr/bin/env python3
"""Run one new review suite. Caller must hold heavy.lock; fixtures are disposable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
OWNED = ROOT / 'tests/audit'


def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('suite', choices=['names', 'transport', 'cli'])
    parser.add_argument('--negative', choices=['config', 'menu'])
    args = parser.parse_args()
    assert not args.negative or (args.suite, args.negative) in [('names', 'config'), ('cli', 'menu')]
    env = dict(os.environ, TMPDIR='/home/christian/.local/share/agent-tmp',
               CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='1',
               TOKIO_WORKER_THREADS='1',
               CARGO_TARGET_DIR=str(ROOT / 'target/portable-daemon'))
    fake = runpy.run_path(str(ROOT / 'tests/portable-ipc-config.py'))['FAKE_ENGINE']
    fake = replace_once(fake, 'cfg: Config }', '''cfg: Config,
        pub entered: std::sync::atomic::AtomicUsize,
        pub completed: std::sync::atomic::AtomicUsize,
        pub statuses: std::sync::atomic::AtomicUsize,
        pub permits: tokio::sync::Semaphore,
        pub padding: std::sync::atomic::AtomicUsize,
    }''')
    fake = replace_once(fake, 'cfg })', '''cfg,
        entered: std::sync::atomic::AtomicUsize::new(0),
        completed: std::sync::atomic::AtomicUsize::new(0),
        statuses: std::sync::atomic::AtomicUsize::new(0),
        permits: tokio::sync::Semaphore::new(0),
        padding: std::sync::atomic::AtomicUsize::new(0),
    })''')
    fake = replace_once(fake, 'self.cfg.mounts.iter()',
                        'self.statuses.fetch_add(1, std::sync::atomic::Ordering::SeqCst); self.cfg.mounts.iter()')
    fake = replace_once(fake, 'state: "idle".into()',
                        'state: "x".repeat(self.padding.load(std::sync::atomic::Ordering::SeqCst))')
    for method in ['force_mount', 'force_unmount']:
        fake = replace_once(fake, f'{method}(&self, _: &str)', f'{method}(&self, name: &str)')
    fake = replace_once(fake, 'explain(&self, _: Option<&str>)', 'explain(&self, name: Option<&str>)')
    fake = replace_once(fake, 'anyhow::bail!("fixture forbids real probes")', '''
        anyhow::ensure!(name == Some(self.cfg.mounts[0].name.as_str()), "wrong explain name");
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Vec::new())''')
    for label in ['mounts', 'unmounts', 'reload']:
        check = '' if label == 'reload' else 'anyhow::ensure!(name == self.cfg.mounts[0].name, "wrong mutation name");'
        fake = replace_once(fake, f'anyhow::bail!("fixture forbids real {label}")', check + '''
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.permits.acquire().await.unwrap().forget();
        self.completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())''')
    with tempfile.TemporaryDirectory(prefix='r1-review-', dir=env['TMPDIR']) as scratch:
        fixture = Path(scratch)
        (fixture / 'src').mkdir()
        (fixture / 'tests').mkdir()
        modules = ['config', 'ipc', 'reconcile', 'probe', 'wol', 'mountcmd']
        sources = ['Cargo.toml', 'Cargo.lock', 'src/bin/mountctl.rs', 'menubar/Sources/main.swift']
        sources += [f'src/{module}.rs' for module in modules]
        sources += ['tests/portable-ipc-config.py', 'tests/audit/round1-ipc-breaker.py']
        print('SOURCE_SHA256 ' + json.dumps({p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sources}), flush=True)
        manifest = (ROOT / 'Cargo.toml').read_text()
        start, end = manifest.index('[lib]'), manifest.index('[dependencies]')
        manifest = manifest[:start] + '[lib]\nname = "mountd"\npath = "src/lib.rs"\n\n[[bin]]\nname = "mountctl"\npath = "src/mountctl.rs"\n\n' + manifest[end:]
        package = 'mountd-round1-ipc-independent-review-fixture'
        (fixture / 'Cargo.toml').write_text(manifest.replace('name = "mountd"', f'name = "{package}"', 1))
        (fixture / 'Cargo.lock').write_text((ROOT / 'Cargo.lock').read_text().replace('name = "mountd"', f'name = "{package}"', 1))
        (fixture / 'src/mountctl.rs').write_bytes((ROOT / 'src/bin/mountctl.rs').read_bytes())
        for module in modules:
            source = (ROOT / f'src/{module}.rs').read_text()
            if module == 'config' and args.negative == 'config':
                source = replace_once(source, 'if name_bytes > MAX_CONTROL_REQUEST_BYTES as usize - envelope {',
                                      'if false && name_bytes > MAX_CONTROL_REQUEST_BYTES as usize - envelope {')
            if module == 'ipc':
                source += '\n#[cfg(test)]\n#[path = ' + json.dumps(str(OWNED / 'round1-ipc-review-unit.rs')) + '] mod round1_review;\n'
            (fixture / f'src/{module}.rs').write_text(source)
        menu = (ROOT / 'menubar/Sources/main.swift').read_text()
        if args.negative == 'menu':
            for action in ['mount', 'unmount']:
                menu = replace_once(menu, f'["{action}", "--", s.name]', f'["{action}", s.name]')
        (fixture / 'menu.swift').write_text(menu)
        env['MOUNTD_REVIEW_MENU'] = str(fixture / 'menu.swift')
        lib = 'extern crate self as mountd;\npub type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;\n'
        lib += ''.join(f'pub mod {module};\n' for module in modules)
        (fixture / 'src/lib.rs').write_text(lib + fake)
        (fixture / 'tests/review_cli.rs').write_bytes((OWNED / 'round1-ipc-review-cli.rs').read_bytes())
        env['MOUNTD_REVIEW_COMMON'] = str(OWNED / 'round1-ipc-review-common.rs')
        selection = ['--test', 'review_cli'] if args.suite == 'cli' else ['--lib', f'round1_review::{args.suite}']
        command = ['cargo', 'test', '--offline', '--locked', '--manifest-path', str(fixture / 'Cargo.toml'), *selection, '--', '--nocapture']
        print('COMMAND ' + json.dumps(command), flush=True)
        started = time.monotonic()
        process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
        try:
            result = process.wait(timeout=220)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise
        print(f'CARGO_EXIT={result} ELAPSED={time.monotonic() - started:.2f}', flush=True)
        raise SystemExit(result)


if __name__ == '__main__':
    main()
