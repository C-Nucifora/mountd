#!/usr/bin/env python3
"""Reject overlapping builds/installs before effects, including during rollback."""
from pathlib import Path
import os

os.environ.setdefault("TMPDIR", str(Path.home() / ".local/share/agent-tmp"))
Path(os.environ["TMPDIR"]).mkdir(parents=True, exist_ok=True)
fixture = Path(__file__).resolve().parents[2] / "menubar/test-build.py"
setup = fixture.read_text().split("    # Failed first installs", 1)[0]
scenario = r"""
    import signal
    import time
    # Two independent build directories, one shared disposable install target.
    work_b = root / "menubar-b"
    work_b.mkdir()
    shutil.copy(work / "build.sh", work_b / "build.sh")
    if os.environ.get("MOUNTD_AUDIT_DISABLE_LOCKS") == "1":
        for checkout in [work, work_b]:
            script = checkout / "build.sh"
            original = script.read_text()
            changed = original.replace('if ! zsystem flock -t 0 "$1"; then', 'if false; then')
            assert changed != original, "negative control did not remove acquisition"
            script.write_text(changed)

    # All gates are bounded, and use the real zsh/system locking implementation.
    pause_code = '''
def pause(phase):
    import time
    (home / (phase + "-paused")).touch()
    end = time.monotonic() + 15
    while not (home / ("release-" + phase)).exists():
        if time.monotonic() >= end: sys.exit(98)
        time.sleep(0.01)
'''
    swiftc = tools / "swiftc"
    swiftc.write_text('''#!/usr/bin/python3
import os, pathlib, sys
home = pathlib.Path(os.environ["HOME"])
locks = [path.stat() for path in [
    home / "Library/Application Support/mountd/menubar-install.lock",
    pathlib.Path("build/.menubar-build.lock"),
] if path.exists()]
for descriptor in os.listdir("/dev/fd"):
    try:
        info = os.fstat(int(descriptor))
    except OSError:
        continue
    assert all((info.st_dev, info.st_ino) != (lock.st_dev, lock.st_ino) for lock in locks), \
        "compiler inherited a lock descriptor"
with (home / "compiler-calls").open("a") as log:
    log.write(os.environ["BUILD_TAG"] + "\\n")
''' + pause_code + '''
if os.environ.get("PAUSE_BUILD") == "1":
    pause("build")
    sys.exit(1)
pathlib.Path(sys.argv[sys.argv.index("-o") + 1]).write_text(os.environ["BUILD_TAG"])
''')
    rm = tools / "rm"
    rm.write_text('''#!/usr/bin/python3
import os, pathlib, sys
home = pathlib.Path(os.environ["HOME"])
args = sys.argv[1:]
os.execv("/bin/rm", ["rm"] + args)
''')
    rm.chmod(0o755)
    for name in ["cat", "mv", "launchctl", "rm"]:
        tool = tools / name
        injection = pause_code + '''
if os.environ.get("BUILD_TAG") == "update-a":
    if TOOL == "cat" and not args and (home / "stopped").exists():
        pause("write")
        sys.exit(1)
    if TOOL == "mv" and args[0].endswith("/old.app"):
        pause("rollback")
    if TOOL == "launchctl" and args[0] == "bootstrap":
        pause("restart")
    if TOOL == "rm" and args[-1].startswith(str(home / "Applications/.mountd-install.")):
        pause("cleanup")
'''
        tool.write_text(tool.read_text().replace(
            'args = sys.argv[1:]',
            'args = sys.argv[1:]\n' + injection.replace('TOOL', repr(name)), 1))

    def wait_for(phase, process):
        end = time.monotonic() + 10
        while not (home / (phase + "-paused")).exists():
            assert process.poll() is None, "owner exited before " + phase
            assert time.monotonic() < end, "owner did not reach " + phase
            time.sleep(0.01)

    def release(phase):
        (home / ("release-" + phase)).touch()

    def finish(process):
        for phase in ["write", "rollback", "restart", "cleanup", "build"]:
            release(phase)
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            # Only the synthetic process group created by this fixture is ours.
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate(timeout=5)

    def state():
        files = {}
        for directory in [work / "build", work_b / "build", home]:
            for path in directory.rglob("*"):
                if path.is_file():
                    files[str(path.relative_to(root))] = path.read_bytes()
        return files

    def reject(checkout, install=True, candidate_env=None):
        before = state()
        result = subprocess.run(["zsh", "build.sh"] + (["install"] if install else []),
                                cwd=checkout, env=candidate_env or env_b,
                                capture_output=True, text=True, timeout=5)
        assert result.returncode != 0, "overlapping contender returned success: " + result.stdout
        assert "another menu bar build/install is active" in result.stderr, result.stderr
        assert state() == before, "rejected contender mutated build/install files or tool traces"

    def lock_inodes():
        return [path.stat().st_ino for path in [
            home / "Library/Application Support/mountd/menubar-install.lock",
            work / "build/.menubar-build.lock",
        ]]

    env["BUILD_TAG"] = "old-app"
    run(True)
    binary = home / "Applications/Mountd Menu Bar.app/Contents/MacOS/mountd-menubar"
    plist = home / "Library/LaunchAgents/org.mountd.menubar.plist"
    assert binary.read_text() == "old-app"
    old_plist = plist.read_bytes()
    inodes = lock_inodes()
    env_a = dict(env, BUILD_TAG="update-a")
    env_b = dict(env, BUILD_TAG="update-b")
    a = subprocess.Popen(["zsh", "build.sh", "install"], cwd=work, env=env_a,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                         start_new_session=True)
    try:
        for phase in ["write", "rollback", "restart", "cleanup"]:
            wait_for(phase, a)
            reject(work_b)
            reject(work, install=False)
            assert lock_inodes() == inodes, "owner's lock inode was replaced"
            release(phase)
        stdout, stderr = a.communicate(timeout=10)
        assert a.returncode != 0, stdout + stderr
        assert binary.read_text() == "old-app"
        assert plist.read_bytes() == old_plist
        assert (home / "loaded").exists()
        assert not list((home / "Applications").glob(".mountd-install.*"))
        b = subprocess.run(["zsh", "build.sh", "install"], cwd=work_b, env=env_b,
                           capture_output=True, text=True, timeout=10)
        assert b.returncode == 0, b.stdout + b.stderr
        assert binary.read_text() == "update-b"
        assert lock_inodes() == inodes
        print("install/write, rollback restore/restart/cleanup reject overlap; retry passed")
    finally:
        finish(a)

    # A build-only invocation also owns the shared checkout output. A different
    # HOME must not bypass it, and a compile failure must release both locks.
    other_home = root / "other-home"
    other_home.mkdir()
    (home / "release-build").unlink()
    a = subprocess.Popen(["zsh", "build.sh"], cwd=work,
                         env=dict(env, BUILD_TAG="build-a", PAUSE_BUILD="1"),
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                         start_new_session=True)
    try:
        wait_for("build", a)
        reject(work)
        reject(work, install=False)
        reject(work, install=False, candidate_env=dict(env_b, HOME=str(other_home)))
        release("build")
        stdout, stderr = a.communicate(timeout=10)
        assert a.returncode != 0, stdout + stderr
        assert binary.read_text() == "update-b"
        env["BUILD_TAG"] = "update-c"
        run(True)
        assert binary.read_text() == "update-c"
        assert lock_inodes() == inodes
        print("build-only overlap rejected across HOME; compile-failure cleanup and retry passed")
        print("compiler children inherit no lock descriptors; shell retains transaction ownership")
    finally:
        finish(a)
    assert config.read_text() == "synthetic config must survive\n"
"""
exec(compile(setup + scenario, str(fixture), "exec"), {"__file__": str(fixture)})
