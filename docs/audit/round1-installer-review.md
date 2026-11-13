Independent resumed round-1 review of `c367823`, at published checkpoint
`0be609a323c8e957bf8e4c20dc55e06d5e2bba07`. No new production defect was
reproduced. Ten new bounded Linux cases pass. Removing the signal guards in
disposable copies makes the new rollback-restart oracle fail as expected.
The two owner-shell signal observations remain unrun after their shared-lock
wait expired. Native acceptance remains open.

The checkout is `/home/christian/Documents/dev/mountd`, branch
`hardening-audit-rounds`. HEAD, the local origin tracking ref and
`git ls-remote origin refs/heads/hardening-audit-rounds` matched the checkpoint.
`git merge-base --is-ancestor c367823 HEAD` returned 0. Origin is
`https://github.com/C-Nucifora/mountd.git`. At entry the tracking reflog recorded
the last push at `2026-11-12 09:31:52 +1000`; the available repository event
query returned no matching push. During this review the shared branch advanced
to `b400b9de679f5bab468518e0baa49b406329e2ce`, `challenge control names and shutdown
admission`, pushed at `2026-11-12 10:03:36 +1000`. Final local HEAD, tracking ref
and remote advertisement agree. The checkpoint remains an ancestor. There is
no checkpoint-to-final diff in the installer or either prior installer fixture;
their hashes are unchanged. This reviewer created no Git branch, checkout,
worktree, thread, commit or push. The unrelated `docs/audit/swarm-rounds.md` edit
and other reviewers' files were preserved.

I read the supplied repository instructions and found no additional AGENTS.md
in this repository or its ancestor directories. Applied ponytail, unslop and
realistic-commits, with the explicit no-commit instruction taking precedence.
Read README, coverage, review-disposition, round1-installer-breaker,
round1-installer-fix, native, review-launchd-followup and mac-verification.
Also inspected review-final, the installer source and both prior Python fixtures.

The source takes HOME ownership before build ownership, before compilation or
installation effects. Failure before the EXIT trap is installed relies on shell
exit to close an acquired first descriptor. The descriptors remain in the shell
during its rollback. Stable lock files are never removed. The `exec launchctl`
inside `service_loaded` preserves failed inspection status inside EXIT recovery.

The new fixture derives only the disposable setup from `menubar/test-build.py`.
It copies the unchanged production installer into two temporary checkouts,
uses the actual Linux zsh 5.9 `zsh/system` fcntl locks, and substitutes compiler,
codesign, launchctl and file-operation helpers. It does not emulate the locking
module. HOME is always below the owned temporary root. Fake launchctl's loaded
and pending markers are a controlled service model, not evidence of launchd
request ordering. No real app, config, LaunchAgent, credential, share or remote
machine was touched. No network listener was used.

| New hypothesis | Evidence and disposition |
| --- | --- |
| A refused second lock leaks HOME ownership | Passed. A build-only owner holds checkout A. An install takes HOME ownership then fails on A's build lock. Before the original build ends, checkout B successfully installs into that HOME. A later retry from A succeeds. |
| Simultaneous cold starts create different lock ownership | Passed. Two checkouts meet at a barrier before state-directory creation. Both are released together. Exactly one reaches the compiler; the loser exits before creating its build directory. Retry reuses the HOME lock inode. The newly created state directory is mode 0700. |
| A malformed lock path permits effects or leaks an acquired first lock | Passed for directories and dangling symlinks to a missing parent, at both lock paths. Append fails before compilation or launchctl. Build-lock failure releases HOME ownership for checkout B. Repair permits a successful retry. This does not claim validation of every possible special file or hostile pathname replacement. |
| Failed rollback inspection is mistaken for absence or success | Passed for exit 5 and actual SIGTERM of the synthetic `print` helper inside EXIT rollback. Both locks exclude contenders while inspection is gated. On failure the installer reports uncertainty, performs no further file recovery and retains the attempted app/plist and previous app/plist backups byte for byte. |
| A signalled rollback client loses uncertainty when the desired state is visible | Passed for rollback bootout and rollback restart. The helper records a pending request and exposes the desired loaded state before actual SIGTERM. No subsequent launchctl inspection or mutation occurs. The files at the fault gate remain unchanged. The restart case retains the previous plist backup; its old app was already restored before the failed restart. |
| Later successful retry deletes an earlier transaction's recovery files | Rejected in all four recovery cases. After the driver settles any synthetic pending request, checkout B successfully installs and leaves every file in the prior staging directory unchanged. |
| Shell death establishes child or service settlement | Not claimed. The TERM/HUP owner-shell fixture did not acquire the shared lock within 600 seconds, so neither observation ran. Source retains descriptors until shell exit; it does not promise ownership or service settlement after runtime destruction. No SIGKILL experiment or crash-recovery requirement was added. |

Each suite independently acquires the shared heavy lock, waits at most 600
seconds, runs under a 230-second timeout with five seconds for termination, and
releases the lock before the next suite. Child gates expire after 12 seconds;
driver waits and joins have shorter deadlines. Cleanup releases only this
fixture's gates and waits for its own processes. A stuck owned process group is
killed only as a final cleanup fallback. TemporaryDirectory removes only that
case's scratch after cleanup. Other jobs and active scratch are untouched.

Exact commands, from `/home/christian/Documents/dev/mountd`:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 230s python3 -u tests/audit/round1-installer-review.py locks \
  > tests/audit/round1-installer-review-locks.log 2>&1

env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 230s python3 -u tests/audit/round1-installer-review.py recovery \
  > tests/audit/round1-installer-review-recovery.log 2>&1

env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  REVIEW_NEGATIVE=signal-guard \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 230s python3 -u tests/audit/round1-installer-review.py recovery \
  > tests/audit/round1-installer-review-negative.log 2>&1

env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 230s python3 -u tests/audit/round1-installer-review-signals.py \
  > tests/audit/round1-installer-review-signals.log 2>&1
```

The first recovery command inserted `/usr/bin/time -f 'elapsed=%e exit=%x'`
before `timeout`. It acquired the lock but exited 69 because `/usr/bin/time`
is absent. No test ran. That complete output was moved to
`tests/audit/round1-installer-review-recovery-initial.log` before retrying the
command above without the optional timing tool. This was an execution setup
error, not a product failure or a provider refusal.

The owner-signal command exited 1 after its 600-second flock wait, with no
fixture output. Its empty log is retained. No child fixture was started and no
runtime result is inferred from its syntax check. This extra observation was
left unrun at the bounded resource limit. The completed recovery suite does
exercise actual SIGTERM of inspection and mutating clients during live-shell
rollback.

The new negative control removes only the two `launch_exit > 128` guard blocks
in disposable installer copies. Inspection-failure cases still pass. The
rollback-restart case then fails at the required `recovery files retained`
assertion, exit 1. The signalled client leaves the desired loaded marker visible;
without the guard the installer consults that marker and removes its staging
directory through its normal cleanup branch. The direct failing assertion
checks the required recovery-file diagnostic. This establishes that the new oracle distinguishes retained
uncertainty from a misleading successful snapshot. The fixed source passes the
same case. No production bypass was introduced. The traceback mentions
`menubar/test-build.py` because the fixture compiles its reused setup prefix
under that filename; the failed assertion belongs to the new review fixture.

The logs record source SHA-256 hashes, each subprocess command, isolated HOME,
checkout, exit status, stdout, stderr and case assertions. Production
`menubar/build.sh` hashes to
`26ea441735c84c4a12a506a809890d21b08bd80ddd7d2684ba37ea8bd1fd3a41`.
The reused setup hashes to
`32575de796e3dff7c8ddaa5fe0326c863738dea8f471a67154a7d8d96c2ea37c`.
No production code or prior fixture was edited.

| Check | Result | Retained artifact under `tests/audit/` |
| --- | --- | --- |
| New lock and startup cases | Exit 0, six cases pass | `round1-installer-review-locks.log` |
| Initial recovery runner | Exit 69, missing timing executable, no test ran | `round1-installer-review-recovery-initial.log` |
| New rollback fault cases | Exit 0, four cases pass | `round1-installer-review-recovery.log` |
| Signal-guard negative control | Exit 1 at the expected recovery-file assertion | `round1-installer-review-negative.log` |
| Owner-shell TERM/HUP observations | Exit 1 from the lock wait, neither case ran | `round1-installer-review-signals.log` |
| Source identity, syntax, whitespace and owned scratch cleanup | Passed; all 13 logged temporary roots removed and no recorded installer PID remains | `round1-installer-review-checks.log` |

Log line endings have only trailing whitespace normalized for Git. Commands,
statuses, output text and tracebacks remain intact. The original fixture and
overlap fixture were not rerun, nor were native builds, broader Rust suites or
provider-refused tests.

Prior objections remain part of this disposition. R1-INSTALL-1 is the confirmed
medium rollback-overwrites-concurrent-success defect; the published fix's
original failing oracle, lock-removal negative control, write/restore/restart/
cleanup overlap checks, build-only cross-HOME case, compiler descriptor check
and permission checks were inspected, not counted as new executions here.
N2's exact-service targeting and rollback obligations still apply. RNI-3 and
RF-1 require recovery files and attempted files to survive an unsettled mutation.
A service snapshot must not erase a timeout, pending cleanup or signalled
mutation. The locks do not settle launchd requests after client exit. Retry or
manual restoration still requires settling the exact service first.

RNI-1, RNI-2, RNI-4, E1 through E6, resolver accumulation and the Rust/CLI side
of RF-1 keep their recorded dispositions in review-disposition. This scoped
installer review does not reopen them or claim new execution evidence for them.
Historical N1 and N3 through N7 retain their integrated dispositions and native
gaps as well; the historical native handoff's pending statuses are not new
failures of this checkpoint.
The original direct-child versus arbitrary-descendant limit, trusted foreground
dependencies, kernel/libc cleanup delays, SMB passwords in helper argv,
same-user pathname replacement, conservative alias rejection and NFS ancillary
RPC coverage limits remain. Explicit process/runtime destruction and OS teardown
cannot provide in-process ownership or guaranteed service settlement.

Native acceptance remains unrun. macOS 13 and current macOS must verify system
zsh module availability, fcntl behavior and lock inode stability on APFS,
including rollback inspection status with both locks held. Native Swift build,
codesign, Foundation reap, AppKit recovery, VoiceOver, launchctl diagnostics,
completed-error ordering, delayed or signalled requests, disposable LaunchAgent
lifecycle and daemon exit policy remain open. The wider audit's APFS case and
Unicode spelling, IOKit, Darwin peer credentials, native helper topology,
disposable Keychain and real-share checks also remain open. Linux fake helpers
and Apple-target type checks cannot close them. Provider-refused testing remains
stopped; no refused payload was retried, rephrased or routed elsewhere.

Owned files only:

- `docs/audit/round1-installer-review.md`
- `tests/audit/round1-installer-review.py`
- `tests/audit/round1-installer-review-signals.py`
- `tests/audit/round1-installer-review-locks.log`
- `tests/audit/round1-installer-review-recovery-initial.log`
- `tests/audit/round1-installer-review-recovery.log`
- `tests/audit/round1-installer-review-negative.log`
- `tests/audit/round1-installer-review-signals.log`
- `tests/audit/round1-installer-review-checks.log`
