# Round 3 policy decision sequences

No new substantive policy defect was confirmed. All six new tests passed,
including 625 modeled migration sequences with 2500 reconcile transitions.
This is a scoped portable result, not Mac acceptance or a completed global
clean round. No production correction is proposed.

## Source and review boundary

Reviewed `/home/christian/Documents/dev/mountd`, origin
`https://github.com/C-Nucifora/mountd.git`, branch `hardening-audit-rounds`.
Entry HEAD was `2224097`, with `171eab5` verified as an ancestor. During review
the parent advanced HEAD to `e38bd63`, `record credential parser verification`.
The production sources, Cargo files and portable reconciler generator have no
diff between those commits. The local tracking reflog initially recorded the
last push at 2026-11-12 10:30:07 +10:00, then the parent's `e38bd63` push at
11:23:52. No hosting request was made because this task forbids network access.

Read the supplied instructions and checked ancestor/repository AGENTS.md
locations; no additional file was found. Applied ponytail, unslop and
realistic-commits. Explicit review-only instructions override local commit
creation and timing. Read README, coverage, review-disposition, reconciler,
round1-lifecycle-breaker, round1-lifecycle-review, swarm-rounds,
round2-credential-review and mac-verification.

Source review covers `reconcile::decide`, eligibility, `MountRuntime` evidence,
and engine bookkeeping, health/migration decisions, polite unmount outcomes,
source attribution and validated config publication. The existing 47 engine
tests and round-1 review establish the comparison baseline. This suite adds
combinations across passes rather than repeating their simple success cases or
manual-suppression and held-operation reload cases.

No production edit, commit, push, branch, worktree, thread or subagent was
created. Other-owner changes are preserved. Status/menu schema and native UI
review belong to the other round-3 owner; broad verification and integration
belong to the parent.

## Oracles and new tests

README requires continuous condition loss before policy unmount, polite policy
and migration unmounts, ordered eligible endpoints, a fresh successful probe
before mounting, and validation before config replacement. State and engine
comments require endpoint-specific evidence and consecutive confirmations.
Failed reload preserves the running config and runtime; successful reload
resets runtime. These are the oracles, not additional policy preferences.

All six tests are in `tests/audit/round3-policy.rs`:

| Exact test | New challenge and expected result |
| --- | --- |
| `migration_sequences_match_consecutive_winner_model` | All 625 length-four sequences over A eligible/reachable, B eligible/reachable, neither target eligible, `when` false, and A eligible/unreachable. A separate consecutive-winner model checks all 2500 reconcile transitions. A busy source C remains mounted, only eligible targets are probed, target switches and gaps discard evidence, and every attempted migration is polite and needs two new confirmations after failure. |
| `interrupted_grace_and_busy_policy_retry_require_continuous_loss` | Mature loss, restoration, fresh loss, two busy policy failures, restoration again, fresh loss and successful polite removal, then restoration/remount. Earlier elapsed grace cannot leak across a true observation. Busy retries never force and preserve actual source C. |
| `source_replacement_discards_partial_migration_evidence` | Start with one A confirmation while mounted via C. Replace C with configured B, an unconfigured NFS-format address, or an unparseable local-device source. Preserve actual attribution, discard the previous source's evidence, then replace it with NFS-format C and require two fresh A confirmations before migrating. Three cases. |
| `invalid_reload_preserves_confirmations_valid_reorder_restarts_them` | Reject both malformed TOML and a zero confirmation threshold during a partial migration streak. The next pass uses retained evidence. Then reload reordered priorities B/A/C while A remains mounted. Adopt A at its new rank and require two fresh B confirmations. Two cases. |
| `invalid_reload_retains_grace_valid_policy_change_discards_it` | Reject an excessive grace setting during mature condition loss and verify the old polite busy retry still occurs. Valid reload either lengthens grace or pins the volume. Old elapsed evidence cannot authorize immediate unmount; after synthetic expiry only the unpinned case unmounts. Two cases. |
| `health_success_breaks_failure_streak_before_confirmed_migration` | C health results fail/fail/succeed/fail alongside four A migration confirmations. One success breaks the health failure streak, so the fourth pass migrates politely instead of forcing recovery. No earlier mount or unmount is permitted. |

Grace tests move only the runtime's monotonic evidence timestamp past a
configured boundary. They do not sleep, change a clock, test timer delivery or
claim exact wall-clock timing. The migration model uses two confirmations and
four-step sequences, not an exhaustive model of every configuration or engine
operation. Dependency names are FakeOps map keys. There are no dependency
executions, network probes, sockets, credentials or real mount effects.

## Fixture and execution

The generator derives its disposable crate setup directly from
`tests/portable-reconciler.py`, stopping before that generator's broad test and
Clippy loop. It changes only the disposable package identity, temporary path
and appended test module. Package identity is `mountd-round3-policy-fixture`.
All nine SystemOps methods remain hard panics. The runner asserts that FakeOps
is byte-identical to the source. No FakeChild gates are configured.

The crate includes existing source test modules for compilation, but the exact
filter selects only the six new policy tests. No existing process, supervisor,
resolver, credential or lifecycle tests are selected. Production native
implementations are not edited to make Linux pass. The disposable power and
SystemOps replacements are exactly the existing portable fixture boundary.

One acquisition and one Cargo invocation, from the explicit mountd directory:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 3600 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round3-policy.py \
  > tests/audit/round3-policy-suite.log 2>&1
policy_status=$?
printf 'outer_exit=%s\n' "$policy_status" >> tests/audit/round3-policy-suite.log
exit "$policy_status"
```

Under the lock, the generator uses `target/portable-daemon` and invokes:

```sh
cargo --offline --locked test --lib engine::round3_policy:: -- --test-threads=1 --nocapture
```

The lock wait is bounded at 3600 seconds with exit 75. Execution after
acquisition is bounded at 235 seconds plus five seconds of termination grace.
There are no retries or parallel heavy jobs. The generator removes only its
own disposable crate after Cargo exits. It records all copied Rust source,
Cargo, generator and test hashes, generated file hashes, exact argv, exit
status and source identity after execution. The separate source manifest
records the policy source and fixture hashes before execution.

The single invocation completed with Cargo and outer exit 0:

```text
6 passed; 0 failed; 0 ignored; 0 measured; 210 filtered out
compile: 20.25 seconds
selected tests: 7.11 seconds
runner after lock acquisition: 27.450 seconds
```

All six exact tests listed above ran and passed. The suite rejects the tested
hypotheses that interrupted conditions retain elapsed grace, migration targets
share confirmation evidence, failed polite unmounts permit force or premature
retry, source replacement retains old migration evidence, failed reload loses
policy evidence, valid reordered/pinned config inherits old evidence, or
intermittent health failures accumulate across a successful probe.

The run log is `tests/audit/round3-policy-suite.log`. It contains source and
generated hashes, exact argv, every test result and all exit statuses. The
temporary crate at
`/home/christian/.local/share/agent-tmp/mountd-round3-policy-tzhjurn1` was removed
by its owner after Cargo completed. The shell invocation exited and released
the shared lock. No second acquisition or heavy suite was started.

Source identity passed inside the runner and again at handoff against
`tests/audit/round3-policy-source-sha256.txt`. HEAD remained `e38bd63`; the
reviewed production files, Cargo files and original portable generator have
no working-tree diff. Python AST parsing, Rust fixture formatting and owned
source/report whitespace checks passed; exact checks are retained in
`tests/audit/round3-policy-checks.log`.

All 210 existing source tests were filtered out. No broad suite, Clippy,
Apple-target check, negative-control mutation or native Mac check was run by
this reviewer. There is no failing regression to report because this review
confirmed no substantive issue. The bounded model does not cover every grace
duration, threshold, multi-mount interleaving or change between observations.

## Preserved limits and stopped scope

Prior E1-E6, RNI-1 through RNI-4, RF-1 and its signal follow-up, and the round-1
findings retain their existing dispositions. These policy tests do not reopen
or strengthen their native acceptance. The parent's earlier 7-case/105-test
credential checks and Clippy are separate evidence, not reruns here.

Native mount-source spelling, actual busy volumes and helper behavior, Darwin
enumeration, APFS, IOKit, Keychain, launchd and Swift/UI remain subject to
`mac-verification.md`. TCP gating still does not prove NFS ancillary-service
availability. The recorded SMB argv, pathname-race, kernel/libc retention and
shutdown limits remain. Synthetic results cannot establish Mac acceptance or
the required two consecutive clean rounds.

The separate round-2 process/drain task was provider-refused and remains
stopped. No `tests/audit/round2-process*` content was inspected, executed,
adapted or changed. No supervisor/resolver/cancellation, command/pipe/reaping
or drain coverage was added or taken over. This independent policy audit is
not a replacement route for that work. No part of this policy request was
refused.

## Owned files

- `docs/audit/round3-policy-breaker.md`
- `tests/audit/round3-policy.py`
- `tests/audit/round3-policy.rs`
- `tests/audit/round3-policy-suite.log`
- `tests/audit/round3-policy-source-sha256.txt`
- `tests/audit/round3-policy-checks.log`

Parent evidence, the Apple check and all other files remain with their owners.
