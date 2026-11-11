#!/usr/bin/env python3
"""New round-one IPC regressions; production sources remain unchanged.

Run under the shared heavy.lock. Reuses portable-ipc-config.py's fake boundaries,
adds observable gated mutations, and only runs the new round1 tests. Exit 1 means
at least one genuine regression assertion failed; inspect both retained logs.
"""
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
OWNED = ROOT / 'tests/audit'


def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)


def main():
    env = dict(os.environ, TMPDIR=os.environ.get('TMPDIR', str(Path.home() / '.local/share/agent-tmp')),
               CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='1',
               CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target/portable-daemon')),
               MOUNTD_AUDIT_ROOT=str(ROOT))
    Path(env['TMPDIR']).mkdir(parents=True, exist_ok=True)
    fake = runpy.run_path(str(ROOT / 'tests/portable-ipc-config.py'))['FAKE_ENGINE']
    fake = replace_once(fake, 'cfg: Config }', '''cfg: Config,
        pub entered: std::sync::atomic::AtomicUsize,
        pub completed: std::sync::atomic::AtomicUsize,
        pub permits: tokio::sync::Semaphore,
        pub padding: std::sync::atomic::AtomicUsize,
    }''')
    fake = replace_once(fake, 'cfg })', '''cfg,
        entered: std::sync::atomic::AtomicUsize::new(0),
        completed: std::sync::atomic::AtomicUsize::new(0),
        permits: tokio::sync::Semaphore::new(0),
        padding: std::sync::atomic::AtomicUsize::new(0),
    })''')
    fake = replace_once(fake, 'state: "idle".into()',
                        'state: "x".repeat(self.padding.load(std::sync::atomic::Ordering::SeqCst))')
    for label in ('mounts', 'unmounts', 'reload'):
        fake = replace_once(fake, f'anyhow::bail!("fixture forbids real {label}")',
                            '''self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.permits.acquire().await.unwrap().forget();
            self.completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())''')
    # Native effects remain unreachable. Gated methods only update counters.
    with tempfile.TemporaryDirectory(prefix='round1-ipc-', dir=env['TMPDIR']) as name:
        fixture = Path(name)
        (fixture / 'src').mkdir()
        (fixture / 'tests').mkdir()
        manifest = (ROOT / 'Cargo.toml').read_text()
        start, end = manifest.index('[lib]'), manifest.index('[dependencies]')
        manifest = manifest[:start] + (
            '[lib]\nname = "mountd"\npath = "src/lib.rs"\n\n'
            '[[bin]]\nname = "mountctl"\npath = '
            + json.dumps(str(ROOT / 'src/bin/mountctl.rs')) + '\n\n'
        ) + manifest[end:]
        package = 'mountd-round1-ipc-breaker-fixture'
        (fixture / 'Cargo.toml').write_text(manifest.replace('name = "mountd"', f'name = "{package}"', 1))
        (fixture / 'Cargo.lock').write_text((ROOT / 'Cargo.lock').read_text().replace('name = "mountd"', f'name = "{package}"', 1))
        lib = 'pub type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;\n'
        for module in ('config', 'ipc', 'reconcile', 'probe', 'wol', 'mountcmd'):
            source = ROOT / f'src/{module}.rs'
            if module == 'ipc':
                source = fixture / 'src/ipc.rs'
                source.write_text((ROOT / 'src/ipc.rs').read_text() + '\n#[cfg(test)]\n#[path = '
                                  + json.dumps(str(OWNED / 'round1-ipc-unit.rs'))
                                  + '] mod round1_ipc;\n')
            lib += '#[path = ' + json.dumps(str(source)) + f'] pub mod {module};\n'
        (fixture / 'src/lib.rs').write_text(lib + fake)
        shutil.copyfile(OWNED / 'round1-ipc-cli.rs', fixture / 'tests/round1_ipc_cli.rs')
        failed = False
        for label, selection in (('unit', ['--lib', 'round1_ipc']),
                                 ('cli', ['--test', 'round1_ipc_cli'])):
            command = ['cargo', 'test', '--offline', '--locked', '--manifest-path',
                       str(fixture / 'Cargo.toml'), *selection, '--', '--nocapture']
            suffix = os.environ.get('MOUNTD_AUDIT_LOG_SUFFIX', '')
            log_path = OWNED / f'round1-ipc-{label}{suffix}.log'
            with log_path.open('w') as log:
                log.write('COMMAND ' + json.dumps(command) + '\n')
                log.flush()
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=log,
                                        stderr=subprocess.STDOUT, timeout=240)
                log.write(f'\nCARGO_EXIT={result.returncode}\n')
            print(f'{label}: cargo exit {result.returncode}; {log_path}', flush=True)
            failed |= result.returncode != 0
        raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    main()
