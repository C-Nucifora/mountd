# R1-INSTALL-1 transaction exclusion

Implementation handoff only. No commits, pushes, real installations, service
registrations, credentials or shares were used. Changes are limited to
`menubar/build.sh`, `menubar/test-build.py`,
`tests/audit/round1-installer-overlap.py` and this report. Other audit owners'
files remain theirs. Parent source and fixture review completed at checkpoint. No further defect was
identified in that review. A fresh independent post-fix challenge is deferred
under the user's pause instruction.

## Before-fix evidence

The original [breaker report](round1-installer-breaker.md) records the confirmed
failure against `b4f33bf`. A second installer returned success with `update-b`;
the first installer's failed plist-write rollback then replaced it with
`old-app`. The original assertion failed with:

```text
second update returned success with update-b; after first rollback: old-app
AssertionError: failed concurrent installer rolled back a successful update
```

An immutable copy of the original three fixture/source files is retained under
`/home/christian/.local/share/agent-tmp/mountd-r1-install-fix-20261112/before-fix`.
The verification runner repeats that original failing oracle separately from
the updated regression. It does not change the repository back to old source.

## Change

`build.sh` uses the existing zsh `zsh/system` module's `zsystem flock -t 0`.
This uses advisory `fcntl` locking and makes one immediate acquisition attempt.
Unavailable locking or an occupied lock exits before compilation, app/plist
replacement, launchctl calls or rollback setup. No dependency was added.

An install first locks
`$HOME/Library/Application Support/mountd/menubar-install.lock`, which covers
separate checkouts targeting the same HOME. Every invocation then locks
`menubar/build/.menubar-build.lock`, which protects the shared app and staging
build paths even for build-only invocations or different HOME values.
Lock-directory and empty lock-file creation are the only preparatory writes.
The shared mountd state directory is created under `umask 077`, independently
of the caller's umask. Existing directories and symlink targets are never
chmodded. The sequential fixture explicitly uses caller umask `0002`, checks
new state mode `0700` and verifies existing directory/symlink mode and owner
preservation.

The owning zsh process retains both descriptors until it exits, including its
EXIT rollback. There is no explicit unlock trap, PID probing, stale-lock cleanup
or lock-file deletion. Repeated invocations reuse the same inodes. A failed
second acquisition releases the first lock when the rejected shell exits.

RF-1 behavior is unchanged. A signalled mutating launchctl client still marks
the outcome unsettled, skips ordinary rollback and retains recovery files.
These locks do not settle a pending launchd request after its client exits.
The exact service must still be inspected and settled before recovery/retry.

## Verification

The final locked runner exited 0. Every check matched its expected status,
including the two deliberate failure oracles. The runner recorded each exact
command, combined output, exit status and elapsed time in:

```text
/home/christian/.local/share/agent-tmp/mountd-r1-install-fix-20261112/
```

Executed from `/home/christian/Documents/dev/mountd`:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  python3 -u /home/christian/.local/share/agent-tmp/mountd-r1-install-fix-20261112/verify.py
```

| Check | Observed exit | Elapsed | Log in artifact directory |
| --- | --- | --- | --- |
| Frozen original overlap reproduction | 1, expected assertion | 1.71 s | `before-fix.log` |
| Actual Linux zsh lock-module availability | 0 | 0.00 s | `zsh-lock-module.log` |
| `zsh -n menubar/build.sh` | 0 | 0.02 s | `shell-syntax.log` |
| Python source compilation without bytecode writes | 0 | 0.04 s | `python-syntax.log` |
| `python3 menubar/test-build.py` | 0 | 12.88 s | `installer-recovery.log` |
| `python3 tests/audit/round1-installer-overlap.py` | 0 | 3.33 s | `installer-overlap.log` |
| `MOUNTD_AUDIT_DISABLE_LOCKS=1 python3 tests/audit/round1-installer-overlap.py` | 1, expected assertion | 2.29 s | `overlap-negative-control.log` |

The final logs and source hashes are also preserved in
[`tests/audit/round1-installer-results`](../../tests/audit/round1-installer-results).
Only trailing whitespace and extra blank EOF lines were normalized for Git.
`summary.txt` records `failed: []`. `source-hashes.txt` records SHA-256 hashes
for the three verified source/fixture files. The original first queued baseline
wait was cancelled before it acquired the lock so its immutable snapshot could
be queued with the patched checks. Subsequent lock waits yielded exec sessions.
No resource lock was bypassed, and no other owner's processes were cancelled.

The updated concurrency fixture rejects contenders within a five-second
subprocess deadline and compares build/install bytes and fake-tool traces
before and after each rejection. Its owned synthetic helpers have 15-second
gates. It checks rejection at plist write, rollback app restoration, rollback
service restart and final staging cleanup; retained lock inodes; restored
app/plist/loaded state; successful
retry after rollback; build-only exclusion across HOME; and retry after compiler
failure. The existing installer fixture retains all its ordinary, incomplete
recovery, XML/config and signalled-client cases, with a bounded run timeout.
The synthetic compiler checks `/dev/fd` against the lock inodes to require that
external children inherit no lock descriptors. Contenders remain excluded
while that child runs; ownership stays with the shell through rollback.

`MOUNTD_AUDIT_DISABLE_LOCKS=1` disables only acquisition in disposable copies for
a negative control. Production locking has no environment bypass. The expected
negative-control failure is `overlapping contender returned success`.

The first lock patch passed overlap checks but failed the existing failed-restart
recovery case. Its shell trace showed a nonzero synthetic `launchctl print`
becoming status 0 inside the EXIT rollback command substitution. The untouched
sequential fixture passed. `service_loaded` now uses `exec launchctl print`
inside that substitution to return the helper's status directly, while the
owning shell retains its locks. This preserves the existing recovery policy.
The initial failure, both shell traces and the passing baseline are retained in
`installer-recovery-initial.log`, `installer-recovery-diagnostic.log`,
`recovery-trace-baseline.log`, `recovery-trace-patched.log` and
`baseline-sequential.log` in the artifact directory. The patched trace records
the intermediate failure before this correction.

## Native gap

Linux execution must use the actual zsh lock module without a substitute.
It establishes synthetic transaction ordering on this host, not macOS module
availability, filesystem semantics or launchd acceptance. On the minimum
supported macOS 13 version and current macOS, record OS/zsh versions and run:

```sh
/bin/zsh -fc 'zmodload zsh/system && zsystem supports flock'
```

Require exit 0. Then run `python3 menubar/test-build.py` and
`python3 tests/audit/round1-installer-overlap.py` under the native serialized test
runner. Both use isolated HOME and fake launchctl/compiler/signing tools while
leaving zsh locking unchanged. Confirm fail-fast rejection, unchanged protected
files and lock inodes, rollback exclusion and retry on native APFS. A separate
build without `install` still needs native Swift/codesign acceptance.
The failed-restart case must also preserve a nonzero service-inspection status
inside rollback with both native zsh locks held.

The existing native launchd and uncertainty checks remain open. Forced process
death or external deletion of active advisory-lock files is not crash recovery.
No Rust launchd change or separately reproduced sister issue is claimed here.
