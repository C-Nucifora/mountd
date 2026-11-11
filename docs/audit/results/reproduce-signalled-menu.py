#!/usr/bin/env python3
"""Reproduce signal-driven menu rollback only in the installer's fake fixture."""
from pathlib import Path

generator = Path(__file__).resolve().parents[3] / 'menubar/test-build.py'
source = generator.read_text()
source = source[:source.index('    for stage in ["bootstrap", "bootout"]:\n        previous = plist.read_bytes()')]
guard = '''        local launch_exit=$?
        if (( launch_exit > 128 )); then
            UNSETTLED=1
            return 1
        fi
'''
anchor = '    shutil.copy(source / "build.sh", work / "build.sh")'
assert source.count(anchor) == 1
source = source.replace(anchor, anchor + '\n    guard = ' + repr(guard)
    + '\n    script = (work / "build.sh").read_text()\n    assert script.count(guard) == 2'
    + '\n    (work / "build.sh").write_text(script.replace(guard, ""))')
source += '''    binary.write_text("previous app for signal control")
    first = len((home / "calls").read_text().splitlines())
    (home / "signal-client").write_text("bootstrap")
    result = run(False)
    assert (home / "pending").read_text() == "bootstrap"
    assert "service state uncertain" not in result.stderr
    assert binary.read_text() == "previous app for signal control"
    later = (home / "calls").read_text().splitlines()[first:]
    assert sum("'bootstrap'" in call for call in later) == 2
    print("Reproduced: signalled bootstrap restored previous files and submitted another bootstrap while pending")
    print("The mutation affected only the disposable fixture; production source is unchanged")
'''
exec(compile(source, str(generator), 'exec'), {'__file__': str(generator)})
