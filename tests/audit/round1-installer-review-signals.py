#!/usr/bin/env python3
"""Observe owner-shell termination during rollback without claiming crash recovery."""
import hashlib
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True
source = Path(__file__).with_name("round1-installer-review.py")
spec = importlib.util.spec_from_file_location("installer_review", source)
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)
review.HOOK += '''
if TOOL == "launchctl" and mode == "shell-signal" and args[0] == "bootstrap" and (home / "stopped").exists():
    gate("fault")
'''


def check(f, sig):
    f.run(tag="old")
    (f.home / "fail-stage").write_text("plist-write")
    owner = f.spawn(tag="owner", mode="shell-signal")
    f.wait("owner", "fault", owner)
    assert f.binary.read_text() == "old"
    backups = list((f.home / "Applications").glob(".mountd-install.*"))
    assert len(backups) == 1
    f.reject(checkout=f.work_b, tag="before-signal")
    os.kill(owner.pid, sig)
    # Observe the shell, not its process group. The helper remains bounded and
    # owned by this fixture, and the driver settles it before any retry.
    deadline = time.monotonic() + 0.3
    while owner.poll() is None and time.monotonic() < deadline:
        time.sleep(0.01)
    exited = owner.poll() is not None
    probe = subprocess.run(["zsh", "-fc", 'zmodload zsh/system; zsystem flock -t 0 "$1"',
                            "probe", str(f.install_lock)], env=f.env,
                           capture_output=True, text=True, timeout=3)
    print("SIGNAL", sig.name, "shell_exited=", exited, "shell_status=", owner.poll(),
          "lock_probe=", probe.returncode, "probe_stderr=", repr(probe.stderr), flush=True)
    assert (probe.returncode == 0) == exited, "lock lifetime differs from owner-shell lifetime"
    assert backups[0].exists(), "staging removed while rollback helper was gated"
    f.release("fault")
    f.finish(owner, False)
    if exited:
        assert backups[0].exists(), "termination removed retained recovery files"
        print("AFTER OWNER EXIT helper completed bootstrap=", (f.home / "loaded").exists(), flush=True)
    f.run(checkout=f.work_b, tag="after-settlement")
    assert f.binary.read_text() == "after-settlement"


if __name__ == "__main__":
    assert os.environ["TMPDIR"] == "/home/christian/.local/share/agent-tmp"
    for path in [Path(__file__), source, review.REPO / "menubar/build.sh"]:
        print("SHA256", hashlib.sha256(path.read_bytes()).hexdigest(), path, flush=True)
    for sig in [signal.SIGTERM, signal.SIGHUP]:
        review.run_case("owner " + sig.name + " during rollback restart", lambda f, sig=sig: check(f, sig))
