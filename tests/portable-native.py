#!/usr/bin/env python3
"""Run native-area logic with fake process boundaries, never native acceptance.

Only launchctl/plutil executable paths are replaced in a temporary launchd copy.
The command supervisor and power acknowledgement logic are extracted unchanged.
No IOKit, real launchctl, shares, probes, credentials or installed files are used.
"""
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
os.environ.setdefault("TMPDIR", str(Path.home() / ".local/share/agent-tmp"))
Path(os.environ["TMPDIR"]).mkdir(parents=True, exist_ok=True)
power = (root / "src/power.rs").read_text()
event = power[power.index("#[derive(Debug)]"):power.index("// ---------------------------------------------------------------------------")]
constants = power[power.index("/// The system is asking"):power.index('#[link(name = "IOKit"')]
helpers = power[power.index("// Keep the acknowledgement"):power.index("/// Register for system power")]
tests = power[power.index("#[cfg(test)]"):]
commands = (root / "src/mountcmd.rs").read_text()
commands = commands[commands.index("/// Shared engine guards"):commands.index("async fn run(argv:")]
with tempfile.TemporaryDirectory(prefix="mountd-native-fixture-") as temp:
    fixture = Path(temp)
    home = fixture / "synthetic & <user>"
    home.mkdir()
    env = dict(os.environ, HOME=str(home))
    env.setdefault("CARGO_TARGET_DIR", str(root / "target/portable-native"))
    # Preserve cargo/rustup locations while isolating the application's HOME.
    env["CARGO_HOME"] = os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    env.setdefault("CARGO_BUILD_JOBS", "1")
    env.setdefault("CARGO_INCREMENTAL", "0")
    env.setdefault("RUST_TEST_THREADS", "1")
    env["RUSTUP_HOME"] = os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
    manifest = (root / "Cargo.toml").read_text()
    manifest = manifest[:manifest.index("[lib]")] + manifest[manifest.index("[dependencies]"):]
    (fixture / "Cargo.toml").write_text(manifest.replace('name = "mountd"', 'name = "mountd-native-fixture"'))
    (fixture / "Cargo.lock").write_text((root / "Cargo.lock").read_text().replace('name = "mountd"', 'name = "mountd-native-fixture"'))
    launchctl = fixture / "launchctl"
    launchctl.write_text('''#!/usr/bin/python3
import os, pathlib, signal, sys, time
home = pathlib.Path(os.environ["HOME"])
args = sys.argv[1:]
assert args[0] in ["print", "bootout", "bootstrap"], args
if args[0] in ["print", "bootout"]:
    assert args[1] == f"gui/{os.getuid()}/org.mountd.agent", args
else:
    assert args[1] == f"gui/{os.getuid()}", args
    assert args[2] == str(home / "Library/LaunchAgents/org.mountd.agent.plist"), args
with (home / "calls").open("a") as log: log.write(repr(args) + "\\n")
loaded = home / "loaded"
fail = home / "fail"
if fail.exists() and fail.read_text() == args[0]:
    fail.unlink()
    print("synthetic failure", file=sys.stderr)
    sys.exit(1)
if args[0] == "print" and not loaded.exists():
    print('Could not find service "org.mountd.agent"', file=sys.stderr)
    sys.exit(113)
defer = home / "defer"
if defer.exists() and defer.read_text() == args[0]:
    (home / "pending").write_text(args[0])
    if (home / "signal-client").exists():
        os.kill(os.getpid(), signal.SIGTERM)
    time.sleep(30)
    sys.exit(1)
if args[0] == "bootout": loaded.unlink()
if args[0] == "bootstrap":
    if loaded.exists(): sys.exit(17)
    loaded.touch()
after = home / "fail-after"
if after.exists() and after.read_text() == args[0]:
    after.unlink(); sys.exit(1)
delay = home / "delay-after"
if delay.exists() and delay.read_text() == args[0]:
    delay.unlink(); time.sleep(11)
''')
    plutil = fixture / "plutil"
    plutil.write_text('''#!/usr/bin/python3
import json, plistlib, sys
with open(sys.argv[-1], "rb") as stream:
    print(json.dumps(plistlib.load(stream)["ProgramArguments"]))
''')
    launchctl.chmod(0o755)
    plutil.chmod(0o755)
    (fixture / "src").mkdir()
    launchd = (root / "src/launchd.rs").read_text()
    launchd = launchd.replace('"/bin/launchctl"', json.dumps(str(launchctl)))
    launchd = launchd.replace('"/usr/bin/plutil"', json.dumps(str(plutil)))
    (fixture / "src/launchd.rs").write_text(launchd)
    (fixture / "src/lib.rs").write_text(
        'pub mod launchd;\npub mod mountcmd {\nuse std::{sync::Arc, time::Duration};\n'
        'use anyhow::{bail, Context, Result};\nuse tokio::io::AsyncReadExt;\n'
        + commands + '\n}\n#[cfg(test)]\npub mod power {\nuse std::time::Duration;\n'
        'use tokio::sync::{mpsc, oneshot};\n' + event + constants + helpers + tests + '\n}\n'
    )
    (fixture / "src/main.rs").write_text('''#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let result = match std::env::args().nth(1).as_deref() {
        Some("install") => mountd_native_fixture::launchd::install().await,
        Some("uninstall") => mountd_native_fixture::launchd::uninstall().await,
        _ => Err(anyhow::anyhow!("synthetic helper requires install or uninstall")),
    };
    mountd_native_fixture::mountcmd::drain_commands().await;
    result
}
''')
    for args in [["test"], ["clippy", "--all-targets", "--", "-D", "warnings"], ["build"]]:
        subprocess.run(["cargo", args[0], "--offline", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), *args[1:]], check=True, env=env)
    (fixture / "bin").mkdir()
    helper = fixture / "bin/mountd-native-fixture"
    shutil.copyfile(Path(env["CARGO_TARGET_DIR"]) / "debug/mountd-native-fixture", helper)
    helper.chmod(0o700)
    (helper.parent / "mountd").write_text("synthetic sibling daemon")
    config = home / "custom & config.toml"
    config.write_text("synthetic config must survive\n")
    plist = home / "Library/LaunchAgents/org.mountd.agent.plist"
    loaded = home / "loaded"
    def run(command, ok=True):
        result = subprocess.run([str(helper), command], env=env, capture_output=True, text=True, timeout=15)
        assert (result.returncode == 0) == ok, result.stdout + result.stderr
        assert config.read_text() == "synthetic config must survive\n"
        return result

    # A failed fresh install leaves no service or plist behind.
    (home / "fail").write_text("bootstrap")
    run("install", False)
    assert not plist.exists() and not loaded.exists()
    run("install")
    assert loaded.exists()
    assert (home / "Library/Logs").is_dir()
    data = plistlib.loads(plist.read_bytes())
    assert data["ProgramArguments"] == [str(helper.parent / "mountd"), "run"]
    assert data["ExitTimeOut"] == 0
    data["ProgramArguments"] += ["--config", str(config)]
    plist.write_bytes(plistlib.dumps(data))
    previous = plist.read_bytes()
    for was_loaded in [True, False]:
        if was_loaded: loaded.touch()
        else: loaded.unlink(missing_ok=True)
        for stage in ["print", "bootout", "bootstrap"]:
            if stage == "bootout" and not was_loaded: continue
            (home / "fail").write_text(stage)
            run("install", False)
            assert plist.read_bytes() == previous, stage
            assert loaded.exists() == was_loaded, stage
    run("install")
    assert plistlib.loads(plist.read_bytes())["ProgramArguments"][2:] == ["--config", str(config)]
    for stage in ["bootout", "bootstrap"]:
        (home / "fail-after").write_text(stage)
        run("install")
        assert loaded.exists(), stage
        assert plistlib.loads(plist.read_bytes())["ProgramArguments"][2:] == ["--config", str(config)]
    for stage in ["bootout", "bootstrap"]:
        previous = plist.read_bytes()
        backups = set(plist.parent.glob("*.recovery.*"))
        (home / "delay-after").write_text(stage)
        result = run("install", False)
        assert "outcome is uncertain" in result.stderr
        assert plist.exists()
        retained = set(plist.parent.glob("*.recovery.*")) - backups
        assert len(retained) == 1
        assert retained.pop().read_bytes() == previous
        assert loaded.exists() == (stage == "bootstrap")
        # This fake applied the change before its reply delay; it is now settled.
        run("install")

    # An accepted request can act after its client has already been killed/reaped.
    for was_loaded, signalled in [(False, False), (True, False), (False, True), (True, True)]:
        run("uninstall")
        if was_loaded: run("install")
        previous = plist.read_bytes() if plist.exists() else None
        backups = set(plist.parent.glob("*.recovery.*"))
        (home / "defer").write_text("bootstrap")
        if signalled: (home / "signal-client").touch()
        result = run("install", False)
        assert "outcome is uncertain" in result.stderr
        assert (home / "pending").read_text() == "bootstrap"
        assert not loaded.exists() and plist.exists()
        attempted = plist.read_bytes()
        assert "bootstrap" in (home / "calls").read_text().splitlines()[-1]
        retained = set(plist.parent.glob("*.recovery.*")) - backups
        if previous is not None:
            assert len(retained) == 1 and retained.pop().read_bytes() == previous
        else:
            assert not retained
        # Release the synthetic server completion only after recovery could have run.
        loaded.touch()
        assert plist.read_bytes() == attempted
        (home / "defer").unlink()
        (home / "pending").unlink()
        (home / "signal-client").unlink(missing_ok=True)
    previous = plist.read_bytes()
    (home / "defer").write_text("bootout")
    result = run("uninstall", False)
    assert "outcome is uncertain" in result.stderr
    assert loaded.exists() and plist.read_bytes() == previous
    loaded.unlink()  # Release the delayed server-side removal.
    assert plist.read_bytes() == previous
    (home / "defer").unlink()
    (home / "pending").unlink()
    run("install")
    (home / "fail").write_text("bootout")
    run("uninstall", False)
    assert plist.exists() and loaded.exists()
    run("uninstall")
    (home / "fail-after").write_text("bootstrap")
    run("install")
    assert plist.exists() and loaded.exists()
    run("uninstall")
    assert not plist.exists() and not loaded.exists()
    run("uninstall")
    print("synthetic launchd install/update/uninstall/config fixtures passed")
