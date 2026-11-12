# Mac verification handoff

Status: not run. Linux fixtures and Apple-target type checking do not establish
macOS runtime acceptance. A disposable macOS 13+ user or VM with Xcode Command
Line Tools, Rust, Python 3 and zsh is required. Record the OS build and CPU
architecture. Use this checkout's final local commits. Do not use the installed mountd app,
production configuration or production credentials.

## Build and synthetic checks

In a fresh checkout on the disposable Mac, run:

```sh
export TMPDIR="$HOME/.local/share/agent-tmp"
mkdir -p "$TMPDIR"
bash tests/verify-macos.sh
```

The script prints an artifact directory and writes one log per check, a result
list, revision, tracked diff, worktree status and platform information. It runs
native Rust tests without ignored Keychain integration tests, Swift typechecking,
synthetic command and installer fixtures, and a menu app build without installing
it. It does not start the daemon, register LaunchAgents, read credentials or
mount shares. Return the entire artifact directory. A green script is build and
synthetic acceptance only.

The round-1 checkpoint also needs native validation of the new installer locks:

```sh
/bin/zsh -fc 'zmodload zsh/system && zsystem supports flock'
python3 tests/audit/round1-installer-overlap.py
```

Require the module check to exit 0 on macOS 13 and current macOS. The overlap
fixture uses the real zsh lock module with fake compiler, signing and launchctl
tools in an isolated HOME. Verify contention fails before effects, exclusion
continues through rollback, retries work, and lock inodes remain stable. The
Linux IPC audit's `/proc/self/fd` measurements are not native Mac evidence.
During the menu recovery case below, also use mount names `--help`, `-h` and
`--`; capture fake mountctl argv and require `--` before each name in both actions.

## Runtime checks requiring the disposable Mac

Record each case as pass, fail or not run. For failures, include the isolated
config, exact trigger, timestamps, exit status and redacted logs. Record the
tested commit for every artifact. Never capture a real password in process
listings, screenshots or logs.

| Case | Trigger and expected result | Artifact |
| --- | --- | --- |
| Unix credentials and ownership | Run the singleton and socket unit tests natively. In two disposable users, attempt access to the first user's socket. Filesystem permissions or peer UID checks must reject it. The rejected process must not remove the socket or lock inode. | Test logs, user IDs and before/after mode and inode records |
| Daemon signals and ownership | Run `python3 tests/portable-daemon.py` on the Mac. Its explicit fake Ops, power source and route executable exercise the real daemon entry point. Duplicate start must fail before effects; SIGTERM must close accepts, drain the accepted request, reap synthetic children, and retain the singleton until done. | Fixture log and final child/process observations |
| Framework bindings | Native Rust test/build must link IOKit and CoreFoundation without replacing their implementations. | `rust-tests.log`, `lint.log` |
| Power delivery | Use a separately prepared fake-Ops daemon in the disposable account with the real power watcher. Trigger idle sleep, forced sleep, cancelled sleep and wake. Repeat with fast, failed and over-budget fake unmounts. Each permission-bearing notification must be acknowledged once within the fixed 25-second callback ceiling; engine cleanup must honor its configured 0..20-second budget; no fake mount starts between sleep and wake. Wake resets retry evidence and honors settling. | Notification ID/time, acknowledgement time and synthetic effect trace |
| Menu recovery | Put a synthetic `mountctl` in the disposable user's `.cargo/bin`. Return valid status, malformed JSON, missing fields, stderr failure, more than 1 MiB output, a TERM-resistant hang, and then valid status. Launch only the newly built app. Menus must remain usable, refresh calls must not overlap, and recovery must clear the error state. | Synthetic response files, screen recording and owned child counts |
| Inherited pipes | Extend the Swift command fixture with a helper that exits after leaving a bounded descendant holding stdout. The command deadline must return without blocked read workers accumulating. Observe Foundation child reaping. | Command log and owned-process observations |
| Accessibility | With VoiceOver, navigate each aggregate status and share submenu using the keyboard while refresh completes. The status item must announce state; each share must announce its name and state. Menu tracking must preserve focus and actions. | VoiceOver transcript or recording, Accessibility Inspector capture |
| Reveal | Reveal a disposable local directory; repeat clicks while workspace completion is delayed. The menu must remain responsive with at most one outstanding Reveal. Record that a permanently blocked NSWorkspace call requires completion or app restart before another Reveal. | UI recording and worker count |
| LaunchAgent lifecycle | Only after authorizing fixture registrations, install synthetic daemon/menu executables in the disposable account. Test fresh install, loaded and unloaded update, crash restart, clean exit and uninstall. Preserve a custom `--config` path. Run all fake rollback cases too. Other processes and config bytes must remain unchanged. | Native `launchctl print` output, plist bytes, fixture logs and config hashes |
| Local pathname aliases | Run the Mac-only preparation fixture on case-insensitive and case-sensitive APFS. Test NAS/nas and composed/decomposed Unicode names without shares. Config must reject possible collisions/nesting; preparation must refuse a noncanonical spelling. Mount-table refusal must not walk a possible remote alias. | Filesystem format, fixture log and directory inode/spelling records |
| Uncertain launchd outcome | Verify native ordering of completed error replies separately from client timeouts. A timed-out or cleanup-pending mutation must report uncertainty and preserve attempted files and any previous-plist backup. Inspect and settle the exact service before retrying or restoring; absence/presence alone must not erase an outstanding request. | launchctl timestamps/results, retained plist bytes and recovery paths |
| Plist paths | Use paths containing spaces, `&` and `<`; inspect generated plists with `/usr/bin/plutil -lint`. Confirm exact `gui/<uid>` labels, `KeepAlive.SuccessfulExit = false` and daemon `ExitTimeOut = 0`. Hold synthetic cleanup past the normal launchd exit window and verify the singleton remains held until release. | Plists and plutil output |

The power and launchd cases require actual macOS behavior. A fake callback or
fake launchctl executable cannot close those cases. Do not alter a real user's
LaunchAgents to obtain this evidence.

## Separately authorized credentials and test shares

These cases remain blocked until a disposable Keychain and disposable NFS/SMB
servers are explicitly authorized. They are not part of the script above.

- Run only the ignored `cred::tests` integration tests in a disposable account
  after reviewing their temporary Keychain creation and deletion. Cover ASCII,
  Unicode, literal hex, trailing newline, missing item, denied ACL and unanswered
  prompt. Verify one total caller deadline across both security invocations.
  For R2-CRED-1, use only these synthetic credentials to verify that native
  metadata supplies one password record and distinct literal quote boundaries.
  Confirm literal hex and encoded Unicode/newline credentials still work. The
  portable malformed/duplicate-record tests do not establish native grammar.
- Use two approved endpoints for the same synthetic share. Verify initial
  selection, disappearing server, restoration, migration confirmation and busy
  polite-unmount failure. Record endpoint, service port and mount source before
  and after each action. A custom NFS port must match the TCP gate.
- Verify `soft`, NFS `intr,deadtimeout=60,retrycnt=0,nfc,tcp`, and compatible
  extra options such as `vers=4,ro` on the minimum and current macOS versions.
  Attempt rejected configurations through `mountd check`; no mount operation is
  needed to test rejection of background, hard, UDP, localhost, proto/netid, address-family selectors or mismatched-port options.
- Disconnect a disposable server and time repeated `status` and native
  `getfsstat(MNT_NOWAIT)` enumeration. Neither may stat the dead mount. Keep
  tests confined to synthetic data; do not force-unmount a production share.
- Test reload and sleep/wake during delayed synthetic credentials and mount
  helpers before attempting the corresponding disposable-share scenarios.
  Successful reload cannot be followed by an effect from the old configuration.

Passwords remain visible in the SMB helper's argv until it exits. A kernel-stuck
helper can outlive its kill deadline. Shutdown intentionally retains ownership
while unreapable work remains. These limits must be recorded, not marked fixed
by a successful mock run.
