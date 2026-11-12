#!/usr/bin/env python3
"""Bounded independent installer challenges. Run under the shared heavy lock."""
import hashlib
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
BASE = REPO / "menubar/test-build.py"
SETUP = BASE.read_text().split("    # Failed first installs", 1)[0]

HOOK = r'''
import signal, time
tag = os.environ.get("REVIEW_TAG", "default")
mode = os.environ.get("REVIEW_MODE", "")
with (home / "review-events").open("a") as trace:
    trace.write(repr((tag, TOOL, args)) + "\n")
def gate(phase):
    (home / (tag + "-" + phase + "-ready")).touch()
    deadline = time.monotonic() + 12
    while not (home / ("release-" + phase)).exists():
        if time.monotonic() > deadline:
            sys.exit(98)
        time.sleep(0.01)
if TOOL == "swiftc":
    if mode in ["hold-build", "cold"]:
        gate("compiler")
    pathlib.Path(args[args.index("-o") + 1]).write_text(tag)
    sys.exit(0)
if TOOL == "mkdir" and mode == "cold" and args[-1].endswith("Application Support/mountd"):
    gate("startup")
if TOOL == "cat" and mode == "rollback-bootout" and not args and (home / "stopped").exists():
    (home / "loaded").touch()
    sys.exit(7)
if TOOL == "launchctl" and (home / "stopped").exists():
    if mode == "inspect-signal" and args[0] == "print":
        gate("fault")
        os.kill(os.getpid(), signal.SIGTERM)
    if mode == "inspect-error" and args[0] == "print":
        gate("fault")
        print("synthetic inspection denied", file=sys.stderr)
        sys.exit(5)
    if (mode == "restart-signal" and args[0] == "bootstrap") or (
        mode == "rollback-bootout" and args[0] == "bootout"
    ):
        # A desired-state snapshot must not erase a signalled pending request.
        (home / "pending").write_text(args[0])
        if args[0] == "bootstrap":
            (home / "loaded").touch()
        else:
            (home / "loaded").unlink(missing_ok=True)
        gate("fault")
        os.kill(os.getpid(), signal.SIGTERM)
'''


class Fixture:
    def __init__(self, context):
        self.__dict__.update({key: context[key] for key in ["root", "work", "home", "tools", "env", "config"]})
        self.children = []
        self.work_b = self.root / "checkout-b"
        self.work_b.mkdir()
        shutil.copy(self.work / "build.sh", self.work_b / "build.sh")
        if os.environ.get("REVIEW_NEGATIVE") == "signal-guard":
            for checkout in [self.work, self.work_b]:
                script = checkout / "build.sh"
                old = script.read_text()
                clause = '        if (( launch_exit > 128 )); then\n            UNSETTLED=1\n            return 1\n        fi\n'
                assert old.count(clause) == 2
                script.write_text(old.replace(clause, ""))
        for tool in ["swiftc", "mkdir"]:
            path = self.tools / tool
            path.write_text('#!/usr/bin/python3\nimport os, pathlib, sys\nhome = pathlib.Path(os.environ["HOME"])\nargs = sys.argv[1:]\nos.execv("/bin/' + tool + '", ["' + tool + '"] + args)\n')
            path.chmod(0o755)
        for tool in ["swiftc", "mkdir", "cat", "launchctl"]:
            path = self.tools / tool
            path.write_text(path.read_text().replace("args = sys.argv[1:]", "args = sys.argv[1:]\n" + HOOK.replace("TOOL", repr(tool)), 1))
        self.binary = self.home / "Applications/Mountd Menu Bar.app/Contents/MacOS/mountd-menubar"
        self.plist = self.home / "Library/LaunchAgents/org.mountd.menubar.plist"
        self.install_lock = self.home / "Library/Application Support/mountd/menubar-install.lock"

    def spawn(self, checkout=None, install=True, tag="default", mode="", home=None):
        env = dict(self.env, REVIEW_TAG=tag, REVIEW_MODE=mode)
        if home is not None:
            env["HOME"] = str(home)
        args = ["zsh", "build.sh"] + (["install"] if install else [])
        print("RUN", args, "cwd=", checkout or self.work, "HOME=", env["HOME"], "tag=", tag, "mode=", mode, flush=True)
        process = subprocess.Popen(args, cwd=checkout or self.work, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.children.append(process)
        return process

    def finish(self, process, ok=None):
        out, err = process.communicate(timeout=6)
        print("RESULT", process.pid, process.returncode, "stdout=", repr(out), "stderr=", repr(err), flush=True)
        if ok is not None:
            assert (process.returncode == 0) == ok, out + err
        return out + err

    def run(self, **kwargs):
        process = self.spawn(**kwargs)
        self.finish(process, True)

    def wait(self, tag, phase, process):
        deadline = time.monotonic() + 6
        while not (self.home / (tag + "-" + phase + "-ready")).exists():
            assert process.poll() is None, "process exited before gate"
            assert time.monotonic() < deadline, "gate was not reached"
            time.sleep(0.01)

    def release(self, phase):
        (self.home / ("release-" + phase)).touch()

    def snapshot(self):
        return {str(p.relative_to(self.root)): p.read_bytes() for folder in [self.home / "Applications", self.home / "Library/LaunchAgents"] for p in folder.rglob("*") if p.is_file()}

    def reject(self, **kwargs):
        before = self.snapshot()
        trace = (self.home / "review-events").read_bytes() if (self.home / "review-events").exists() else b""
        output = self.finish(self.spawn(**kwargs), False)
        assert "another menu bar build/install is active" in output
        assert self.snapshot() == before
        # mkdir may prepare lock directories; no compiler or service calls may occur.
        after = (self.home / "review-events").read_bytes()
        assert all("'mkdir'" in line for line in after[len(trace):].decode().splitlines()), after[len(trace):]

    def cleanup(self):
        for phase in ["startup", "compiler", "fault"]:
            self.release(phase)
        for process in self.children:
            try:
                process.communicate(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate(timeout=3)
        assert self.config.read_text() == "synthetic config must survive\n"


def second_lock(f):
    holder = f.spawn(install=False, tag="holder", mode="hold-build")
    f.wait("holder", "compiler", holder)
    f.reject(tag="second-lock-refused")
    # The refused installer obtained HOME ownership first. A different checkout
    # must now acquire it while the build-only holder still owns checkout A.
    f.run(checkout=f.work_b, tag="independent-install")
    assert holder.poll() is None
    assert f.binary.read_text() == "independent-install"
    f.release("compiler")
    f.finish(holder, True)
    f.run(tag="retry")
    assert f.binary.read_text() == "retry"


def cold_start(f):
    a = f.spawn(tag="a", mode="cold")
    b = f.spawn(checkout=f.work_b, tag="b", mode="cold")
    f.wait("a", "startup", a)
    f.wait("b", "startup", b)
    assert not f.install_lock.exists()
    f.release("startup")
    deadline = time.monotonic() + 6
    while a.poll() is None and b.poll() is None:
        assert time.monotonic() < deadline
        time.sleep(0.01)
    loser, winner = (a, b) if a.poll() is not None else (b, a)
    assert "another menu bar build/install is active" in f.finish(loser, False)
    tag = "b" if winner is b else "a"
    f.wait(tag, "compiler", winner)
    inode = f.install_lock.stat().st_ino
    assert f.install_lock.parent.stat().st_mode & 0o777 == 0o700
    events = (f.home / "review-events").read_text()
    assert events.count("'swiftc'") == 1
    assert not (f.work if loser is a else f.work_b).joinpath("build").exists()
    f.release("compiler")
    f.finish(winner, True)
    assert f.binary.read_text() == tag
    f.run(checkout=f.work_b if loser is b else f.work, tag="retry")
    assert f.install_lock.stat().st_ino == inode


def bad_path(f, where, kind):
    target = f.install_lock if where == "home" else f.work / "build/.menubar-build.lock"
    target.parent.mkdir(parents=True, exist_ok=True)
    if kind == "directory":
        target.mkdir()
    else:
        target.symlink_to(f.root / "missing-parent/lock")
    result = f.finish(f.spawn(tag="bad-path"), False)
    assert "another menu bar build/install" not in result  # append fails first
    assert not f.binary.exists()
    assert "'swiftc'" not in (f.home / "review-events").read_text()
    assert "'launchctl'" not in (f.home / "review-events").read_text()
    if where == "build":
        f.run(checkout=f.work_b, tag="released-home-lock")
    if kind == "directory":
        target.rmdir()
    else:
        target.unlink()
    f.run(tag="repaired")


def recovery(f, mode):
    f.run(tag="old")
    old_plist = f.plist.read_bytes()
    (f.home / "fail-stage").write_text("plist-write")
    p = f.spawn(tag="update", mode=mode)
    f.wait("update", "fault", p)
    before = f.snapshot()
    f.reject(checkout=f.work_b, tag="contender")
    f.reject(install=False, tag="build-contender")
    f.release("fault")
    result = f.finish(p, False)
    assert "recovery files retained" in result, result
    backups = list((f.home / "Applications").glob(".mountd-install.*"))
    assert len(backups) == 1
    stage = backups[0]
    assert (stage / "old.plist").read_bytes() == old_plist
    assert f.snapshot() == before, "fault caused further recovery file mutations"
    if mode in ["restart-signal", "rollback-bootout"]:
        verb = "bootstrap" if mode == "restart-signal" else "bootout"
        assert (f.home / "pending").read_text() == verb
        events = (f.home / "review-events").read_text().splitlines()
        service_events = [line for line in events if "'launchctl'" in line]
        assert repr(verb) in service_events[-1], service_events[-1]
        # The driver settles the pending synthetic server request before retry.
        (f.home / "pending").unlink()
    if mode == "restart-signal":
        assert f.binary.read_text() == "old"
        assert not (stage / "old.app").exists()  # already restored before restart
    else:
        assert f.binary.read_text() == "update"
        assert (stage / "old.app/Contents/MacOS/mountd-menubar").read_text() == "old"
    retained = {str(path.relative_to(stage)): path.read_bytes() for path in stage.rglob("*") if path.is_file()}
    (f.home / "fail-stage").unlink(missing_ok=True)
    f.run(checkout=f.work_b, tag="settled-retry")
    assert f.binary.read_text() == "settled-retry"
    assert retained == {str(path.relative_to(stage)): path.read_bytes() for path in stage.rglob("*") if path.is_file()}


def run_case(name, case):
    print("CASE", name, flush=True)
    def challenge(context):
        f = Fixture(context)
        try:
            case(f)
        finally:
            f.cleanup()
    exec(compile(SETUP + "    challenge(locals())\n", str(BASE), "exec"), {"__file__": str(BASE), "challenge": challenge})
    print("PASS", name, flush=True)


if __name__ == "__main__":
    assert os.environ["TMPDIR"] == "/home/christian/.local/share/agent-tmp"
    print("PLATFORM", sys.platform, subprocess.check_output(["zsh", "--version"], text=True).strip())
    for path in [REPO / "menubar/build.sh", BASE, Path(__file__)]:
        print("SHA256", hashlib.sha256(path.read_bytes()).hexdigest(), path)
    if sys.argv[1] == "locks":
        run_case("second lock refusal releases HOME ownership", second_lock)
        run_case("simultaneous cold startup from separate checkouts", cold_start)
        for where in ["home", "build"]:
            for kind in ["directory", "dangling-link"]:
                run_case(where + " lock " + kind, lambda f, w=where, k=kind: bad_path(f, w, k))
    elif sys.argv[1] == "recovery":
        for mode in ["inspect-error", "inspect-signal", "restart-signal", "rollback-bootout"]:
            run_case(mode, lambda f, mode=mode: recovery(f, mode))
    else:
        raise SystemExit("choose locks or recovery")
