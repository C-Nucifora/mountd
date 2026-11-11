#!/usr/bin/env python3
"""Reproduce the reviewed signal/rollback defect only in a disposable fixture.

Remove the signal-classification fix from a temporary copy, submit a synthetic
pending bootstrap, and terminate that client with SIGTERM. The old branch must
delete its attempted plist while the pending request remains recorded.
"""
from pathlib import Path

generator = Path(__file__).resolve().parents[3] / 'tests/portable-native.py'
source = generator.read_text()
source = source[:source.index('    # A failed fresh install')]
guard = '''    if out.status.code().is_none() {
        return Err(
            UnsettledMutation(anyhow::anyhow!("launchctl client terminated by signal")).into(),
        );
    }
'''
anchor = '    launchd = (root / "src/launchd.rs").read_text()'
assert source.count(anchor) == 1
source = source.replace(anchor, anchor + '\n    guard = ' + repr(guard)
                        + '\n    assert launchd.count(guard) == 1\n    launchd = launchd.replace(guard, "")')
source = source.replace('mountd-native-fixture', 'mountd-native-negative-fixture')
source = source.replace('mountd_native_fixture', 'mountd_native_negative_fixture')
checks = 'for args in [["test"], ["clippy", "--all-targets", "--", "-D", "warnings"], ["build"]]:'
assert source.count(checks) == 1
source = source.replace(checks, 'for args in [["build"]]:')
source += '''    (home / "defer").write_text("bootstrap")
    (home / "signal-client").touch()
    result = run("install", False)
    assert (home / "pending").read_text() == "bootstrap"
    assert "outcome is uncertain" not in result.stderr
    assert not plist.exists(), "old rollback did not reproduce the lost attempted plist"
    assert not loaded.exists()
    print("Reproduced: signalled client left pending bootstrap after rollback deleted its plist")
    print("The mutation affected only the disposable fixture; production source is unchanged")
'''
exec(compile(source, str(generator), 'exec'), {'__file__': str(generator)})
