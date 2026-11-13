#!/usr/bin/env python3
"""Exercise build/install/update rollback in a temporary HOME with fake tools."""
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

source = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix="mountd-install-fixture-") as temp:
    root = Path(temp)
    work = root / "menubar"
    work.mkdir()
    shutil.copy(source / "build.sh", work / "build.sh")
    home = root / "synthetic & <user>"
    home.mkdir()
    config = home / ".config/mountd/config.toml"
    config.parent.mkdir(parents=True)
    config.write_text("synthetic config must survive\n")
    tools = root / "tools"
    tools.mkdir()
    scripts = {
        "uname": '#!/bin/sh\nif [ "${1:-}" = -m ]; then echo "${SYNTHETIC_ARCH:-arm64}"; else echo Darwin; fi\n',
        "swiftc": '#!/bin/sh\ncase " $* " in *" -target ${SYNTHETIC_ARCH:-arm64}-apple-macosx13.0 "*) ;; *) echo "missing macos 13 target" >&2; exit 1 ;; esac\nwhile [ "$1" != -o ]; do shift; done\nshift\nprintf synthetic > "$1"\n',
        "codesign": '#!/bin/sh\nexit 0\n',
        "ditto": '#!/bin/sh\nshift; shift; shift\ncp -R "$1" "$2"\n',
        "id": '#!/bin/sh\necho 4242\n',
        "launchctl": '''#!/usr/bin/python3
import os, pathlib, signal, sys
home = pathlib.Path(os.environ["HOME"])
args = sys.argv[1:]
with (home / "calls").open("a") as log: log.write(repr(args) + "\\n")
loaded = home / "loaded"
signal_client = home / "signal-client"
if signal_client.exists() and signal_client.read_text() == args[0]:
    (home / "pending").write_text(args[0])
    os.kill(os.getpid(), signal.SIGTERM)
if args[0] == "print":
    if (home / "permission-denied").exists():
        print("permission denied", file=sys.stderr); sys.exit(1)
    if not loaded.exists():
        print('Could not find service "org.mountd.menubar"', file=sys.stderr); sys.exit(113)
elif args[0] == "bootout":
    if (home / "fail-bootout").exists(): sys.exit(1)
    loaded.unlink(missing_ok=True)
    (home / "stopped").touch()
elif args[0] == "bootstrap":
    if loaded.exists(): sys.exit(17)
    fail = home / "fail-once"
    if fail.exists():
        fail.unlink(); sys.exit(1)
    if (home / "fail-restart").exists(): sys.exit(1)
    loaded.touch()
    (home / "stopped").unlink(missing_ok=True)
else: sys.exit(99)
after = home / "fail-after"
if after.exists() and after.read_text() == args[0]:
    after.unlink(); sys.exit(1)
''',
    }
    for tool in ["cp", "mv", "cat"]:
        scripts[tool] = f'''#!/usr/bin/python3
import os, pathlib, sys
home = pathlib.Path(os.environ["HOME"])
args = sys.argv[1:]
fail = home / "fail-stage"
if fail.exists():
    stage = fail.read_text()
    hit = (
        (stage == "new-copy" and "{tool}" == "cp" and args[-1].endswith("/new.app")) or
        (stage == "plist-backup" and "{tool}" == "cp" and args[-1].endswith("/old.plist")) or
        (stage == "old-move" and "{tool}" == "mv" and args[-1].endswith("/old.app")) or
        (stage == "new-move" and "{tool}" == "mv" and args[0].endswith("/new.app")) or
        (stage == "plist-write" and "{tool}" == "cat" and (home / "stopped").exists())
    )
    if hit:
        fail.unlink(); sys.exit(1)
recovery = home / "fail-recovery"
if recovery.exists():
    stage = recovery.read_text()
    if ((stage == "restore-app" and "{tool}" == "mv" and args[0].endswith("/old.app")) or
        (stage == "restore-plist" and "{tool}" == "cp" and args[0].endswith("/old.plist"))):
        recovery.unlink(); sys.exit(1)
os.execv("/bin/{tool}", ["{tool}"] + args)
'''
    for name, script in scripts.items():
        path = tools / name
        path.write_text(script)
        path.chmod(0o755)
    env = dict(os.environ, HOME=str(home), PATH=str(tools) + os.pathsep + os.environ["PATH"])
    def run(ok):
        result = subprocess.run(["zsh", "build.sh", "install"], cwd=work, env=env,
                                capture_output=True, text=True, timeout=15, umask=0o002)
        assert (result.returncode == 0) == ok, result.stdout + result.stderr
        assert config.read_text() == "synthetic config must survive\n"
        return result
    # Failed first installs must leave neither an app nor a login job behind.
    for stage in ["new-copy", "new-move", "bootstrap"]:
        marker = home / ("fail-once" if stage == "bootstrap" else "fail-stage")
        marker.write_text(stage)
        result = run(False)
        assert not (home / "Applications/Mountd Menu Bar.app").exists(), (stage, result.stderr)
        assert not (home / "Library/LaunchAgents/org.mountd.menubar.plist").exists(), stage
        assert not (home / "loaded").exists(), stage
    (home / "fail-after").write_text("bootstrap")
    run(True)
    state_dir = home / "Library/Application Support/mountd"
    assert state_dir.stat().st_mode & 0o777 == 0o700, "state dir must be private under umask 0002"
    app = home / "Applications/Mountd Menu Bar.app"
    binary = app / "Contents/MacOS/mountd-menubar"
    plist = home / "Library/LaunchAgents/org.mountd.menubar.plist"
    data = plistlib.loads(plist.read_bytes())
    assert data["ProgramArguments"] == [str(binary)]
    assert data["KeepAlive"] == {"SuccessfulExit": False}
    binary.write_text("old app")
    old_plist = plist.read_bytes()
    for stage in ["new-copy", "plist-backup", "old-move", "new-move", "plist-write", "bootstrap", "bootout"]:
        marker = home / ("fail-once" if stage == "bootstrap" else "fail-bootout" if stage == "bootout" else "fail-stage")
        marker.write_text(stage)
        run(False)
        marker.unlink(missing_ok=True)
        assert binary.read_text() == "old app", stage
        assert plist.read_bytes() == old_plist, stage
        assert (home / "loaded").exists(), stage
    # An unloaded old agent must remain unloaded after a failed update.
    (home / "loaded").unlink()
    (home / "fail-once").touch()
    run(False)
    assert binary.read_text() == "old app"
    assert plist.read_bytes() == old_plist
    assert not (home / "loaded").exists()
    (home / "loaded").touch()
    # A failed recovery must preserve the backup and report its path.
    (home / "fail-restart").touch()
    result = run(False)
    assert "rollback incomplete; recovery files retained" in result.stderr, result.stdout + result.stderr
    backups = list((home / "Applications").glob(".mountd-install.*"))
    assert len(backups) == 1
    assert (backups[0] / "old.plist").read_bytes() == old_plist
    assert binary.read_text() == "old app"
    assert plist.read_bytes() == old_plist
    shutil.rmtree(backups[0])
    (home / "fail-restart").unlink()
    (home / "loaded").touch()
    for stage in ["restore-app", "restore-plist"]:
        (home / "fail-once").touch()
        (home / "fail-recovery").write_text(stage)
        result = run(False)
        assert "rollback incomplete; recovery files retained" in result.stderr, result.stdout + result.stderr
        backups = list((home / "Applications").glob(".mountd-install.*"))
        assert len(backups) == 1
        assert (backups[0] / "old.plist").read_bytes() == old_plist
        if stage == "restore-app":
            assert (backups[0] / "old.app/Contents/MacOS/mountd-menubar").read_text() == "old app"
        shutil.rmtree(app, ignore_errors=True)
        binary.parent.mkdir(parents=True)
        binary.write_text("old app")
        plist.write_bytes(old_plist)
        shutil.rmtree(backups[0])
        (home / "loaded").touch()
    (home / "permission-denied").touch()
    run(False)
    assert binary.read_text() == "old app"
    assert plist.read_bytes() == old_plist
    (home / "permission-denied").unlink()
    run(True)
    assert binary.read_text() == "synthetic"
    for stage in ["bootout", "bootstrap"]:
        (home / "fail-after").write_text(stage)
        run(True)
        assert (home / "loaded").exists(), stage
        assert binary.read_text() == "synthetic", stage
    for stage in ["bootstrap", "bootout"]:
        previous = plist.read_bytes()
        (home / "signal-client").write_text(stage)
        result = run(False)
        assert "service state uncertain; recovery files retained" in result.stderr, result.stdout + result.stderr
        assert (home / "pending").read_text() == stage
        assert stage in (home / "calls").read_text().splitlines()[-1]
        assert binary.read_text() == "synthetic" and plist.exists()
        assert (home / "loaded").exists() == (stage == "bootout")
        backups = list((home / "Applications").glob(".mountd-install.*"))
        assert len(backups) == 1
        assert (backups[0] / "old.plist").read_bytes() == previous
        if stage == "bootstrap":
            assert (backups[0] / "old.app/Contents/MacOS/mountd-menubar").read_text() == "synthetic"
            (home / "loaded").touch()
        else:
            (home / "loaded").unlink()
        # The driver has now released and settled the synthetic server mutation.
        (home / "signal-client").unlink()
        (home / "pending").unlink()
        shutil.rmtree(backups[0])
        run(True)
    assert config.read_text() == "synthetic config must survive\n"
    assert not list((home / "Applications").glob(".mountd-install.*"))
    # Existing state directories and symlink targets keep their mode and owner.
    state_dir.chmod(0o750)
    previous = state_dir.stat()
    run(True)
    assert (state_dir.stat().st_mode, state_dir.stat().st_uid, state_dir.stat().st_gid) == (
        previous.st_mode, previous.st_uid, previous.st_gid)
    target = root / "existing-state"
    state_dir.rename(target)
    state_dir.symlink_to(target, target_is_directory=True)
    run(True)
    assert state_dir.is_symlink()
    assert (target.stat().st_mode, target.stat().st_uid, target.stat().st_gid) == (
        previous.st_mode, previous.st_uid, previous.st_gid)
    calls = (home / "calls").read_text()
    assert "gui/4242/org.mountd.menubar" in calls
    assert "pkill" not in (work / "build.sh").read_text()
    print("synthetic install, update, rollback, XML and config fixtures passed")
