# Round 1 lifecycle breaker

One confirmed medium finding, R1-L1, with two failing regressions. Production
code was not changed. Both runs passed six new tests and failed the same two.
This is not a clean round.

Reviewed and tested the engine sources from `a66778e` on Linux. The checkout
started at that commit on `hardening-audit-rounds`; the parent's documentation
commit advanced HEAD to `b4f33bf` during the audit. `a66778e` remains an ancestor.
The tested engine, Ops and state hashes match the baseline exactly and are
recorded at the start of each run log.

## R1-L1: an explicit mount can leave manual suppression active

Severity: medium. A volume can remain unmounted indefinitely after a user has
explicitly requested mounting it. This is one intent-bookkeeping defect with
two entry paths, not two independent findings.

The contract in [README.md](/home/christian/Documents/dev/mountd/README.md:135)
and [state.rs](/home/christian/Documents/dev/mountd/src/state.rs:138) says an
explicit mount clears manual suppression. Conditions in both reproductions
remain true throughout, so there is no later false-to-true edge to repair it.

Reproduction A, the successful-command case:

1. Mount A and explicitly unmount it. The engine records manual suppression.
2. Insert a synthetic externally mounted volume B at the same mountpoint,
   modelling Finder or another same-user tool remounting it.
3. Explicitly mount the configured share. `force_mount` returns `Ok`, correctly
   avoids stacking another mount, but leaves `user_suppressed = true`.
4. Perform pre-sleep cleanup, wake, and reconcile.

Expected: the explicit mount ended manual suppression, so the eligible share
remounts on wake. Actual: no mount occurs; status remains
`unmounted (manual, until conditions change)`. The call trace contains one
initial mount, one explicit forced unmount and one polite pre-sleep unmount.
The regression is
[explicit_mount_of_reappeared_volume_clears_manual_intent](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle.rs:126).

Reproduction B, the failed-attempt case:

1. Mount and explicitly unmount A.
2. Inject an ordinary transient helper failure and explicitly mount A.
3. Remove the injected failure, wake, and reconcile.

Expected: the new explicit intent permits automatic retry after the transient
failure disappears. Actual: the earlier manual suppression remains active;
status still reports a manual unmount and no third mount attempt occurs.
The regression is
[failed_explicit_mount_reenables_automatic_retry](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle.rs:152).
This expectation follows the documented command semantics. Even if failed
attempts were deliberately excluded by a future policy change, reproduction A
still demonstrates a successful command violating that contract.

Root cause: [force_mount](/home/christian/Documents/dev/mountd/src/engine.rs:154)
does not clear suppression when it accepts the explicit request. The existing
mount branch returns at line 175. Ordinary attempt errors return before
`note_mounted` at line 202. Only
[note_mounted](/home/christian/Documents/dev/mountd/src/state.rs:159) clears the
flag, so both paths bypass the change in user intent.

Smallest fix recommendation: after validating the name and acquiring its claim
in `force_mount`, clear that mount's manual-suppression flag before the
already-mounted shortcut and before attempting helpers. Keep shutdown, sleep,
unknown-name and claim-conflict refusals ahead of that update. Preserve the
existing mount-table check, probe gates, leases and no-stack behavior. No
production patch was applied; the parent owns fix assignment and review.

## New tests and rejected hypotheses

The new
[Rust fixture](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle.rs)
is appended to a temporary copy of `engine.rs` by the
[injector](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle.py).
It reuses `tests/portable-reconciler.py`, including its hard-failing SystemOps
replacement, and uses the distinct Cargo package
`mountd-round1-lifecycle-fixture`. Engine, state and FakeOps behavior are
unchanged. All gates use the existing `FakeChild`; no native helper or network
operation is executed. Private endpoint strings are only keys in FakeOps.

| New test | Result and observation |
| --- | --- |
| `successful_manual_sequences_match_model` | Passed all 256 sequences of four operations, 1024 actions. The model tracks mounted state and manual intent across mount, unmount, reconcile and sleep/wake. Source attribution and claim release are checked after each action. |
| `cancelled_operations_block_reload_through_removal_and_readdition` | Passed 12 cases, four held stages times three config replacements. Credential, preparation, mount and unmount cancellation retain claims and block reload until reap; status remains available. Removal, endpoint replacement and identical reload reset state only after release. Readdition creates a new claim and attributes the actual mounted source. No cancelled operation produces a late mount/unmount. |
| `reload_waits_for_successful_pending_unmount_then_uses_new_endpoint` | Passed. A pending successful unmount blocks reload; after completion, config B mounts and old manual suppression does not leak into the new runtime. |
| `removal_leaves_volume_and_readdition_adopts_actual_source` | Passed. Removal leaves the old volume mounted as documented. Readdition configured for B reports actual source A, with no extra mount/unmount. Explicit unmount/remount then uses B. |
| `queued_sleep_wake_sleep_waits_for_reap_without_intermediate_mount` | Passed. A cancelled mount keeps its claim while wake is queued behind reap. The second sleep acknowledgement waits; releasing cleanup allows ordered wake/sleep handling without an intervening mount. A final wake permits exactly one mount. |
| `shutdown_rejects_queued_wake_reload_and_final_mount` | Passed four cases, credential/preparation gates times automatic/manual operations. Shutdown rejects queued reload, keeps wake from clearing sleep, blocks drain while ownership remains, and permits no mount/unmount after release or later explicit/reconcile/sleep calls. |
| `explicit_mount_of_reappeared_volume_clears_manual_intent` | Failed, R1-L1 reproduction A. |
| `failed_explicit_mount_reenables_automatic_retry` | Failed, R1-L1 reproduction B. |

The shutdown assertion concerns starting mount/unmount effects after held
credential/preparation operations finish. It does not demand that an already
started helper's effect vanish at shutdown, or prove arbitrary descendants
have exited. The fixture does not replace the documented native acceptance.

## Commands and retained evidence

All repository commands used
`/home/christian/Documents/dev/mountd` explicitly as their working directory.
The exact first-run command was:

```sh
flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout --signal=TERM --kill-after=10s 210s \
  env TMPDIR=/home/christian/.local/share/agent-tmp \
      CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
      CARGO_TARGET_DIR=/home/christian/Documents/dev/mountd/target/portable-daemon \
  python3 tests/audit/round1-lifecycle.py \
  > tests/audit/round1-lifecycle-first.log 2>&1
```

It exited 101 with `6 passed; 2 failed; 0 ignored; 208 filtered out`.
Compilation took 18.56 seconds; the new tests took 1.15 seconds.
The formatted rerun used the identical command with output redirected to
`tests/audit/round1-lifecycle-final.log`. It also exited 101 with six passes,
the same two failures and 208 existing tests filtered out. Compilation took
19.87 seconds; the new tests took 1.21 seconds. The first run copied the fixture
before formatting; both versions have identical test logic. Each log records
its fixture hash to avoid confusing their line numbers.

The injector invokes exactly:

```sh
cargo --offline --locked test --lib round1_lifecycle -- --nocapture
```

Cargo runs inside the disposable portable crate, with a 180-second subprocess
timeout. The outer 210-second deadline bounds generation and execution after
the lock is acquired. Lock waits used yielded tool execution, allowing source
review to continue. The two test jobs are serial. No pre-existing tests were
selected, and no test failures were relabelled as passes.

Retained evidence:

- [Initial log](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle-first.log).
- [Formatted rerun log](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle-final.log).
- [Test additions patch](/home/christian/Documents/dev/mountd/tests/audit/round1-lifecycle-tests.patch), containing only the new injector and Rust fixture. The shared checkout already contains those files; do not apply it a second time.

## Scope and unresolved work

Read repository/ancestor instructions, README, ponytail, unslop,
realistic-commits, coverage, review-disposition, verification, mac-verification,
and all four original review reports. Source review covered force mount/unmount,
reload publication, per-mount claims, reconciliation bookkeeping, source
attribution, sleep/wake dispatch, shutdown gates, FakeOps and daemon/IPC drain
ordering. No finding reopens the accepted direct-child, stuck kernel/libc,
SMB argv, post-preparation pathname or pending native-Mac limits.

At entry there was no remote-tracking ref for this branch. The parent's later
push created the reliable reflog entry `update by push` at
`2026-11-12 08:50:58 +1000`, pointing to `b4f33bf`. This breaker made no commit,
push, branch/worktree change, hosting-service request or auth change.

Owned writes are limited to `tests/audit/round1-lifecycle*` and this report.
Other agents' changes, including `docs/audit/swarm-rounds.md`, were preserved.
No subagents, real mounts/unmounts, installed applications, LaunchAgents,
credentials, public services or Docker were used. The temporary crate is
removed by its generator on completion; logs and source fixtures are retained.
Both owned jobs exited, and no lifecycle injector or selected Cargo test process
remained at the final process check. Whitespace checks passed for the three new
source/report files. `git apply --numstat` parsed the test patch as two new
files, 44 Python lines and 406 Rust lines. Production sources and the original
portable generator have no working-tree changes.

R1-L1 remains unfixed and needs parent triage, a separate fixer, integration and
fresh review. Native Mac checks remain user-run. No broad build or existing
full suite was run by this breaker.
