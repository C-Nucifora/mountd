# Round 1 lifecycle correction review

Independent review of the parent's R1-L1 correction in `src/engine.rs`, in the
`hardening-audit-rounds` worktree based on `b4f33bf`. No new lifecycle defect or
unresolved objection to this correction was found. All seven new tests pass,
covering 27 cases. Removing only the correction makes the new enumeration-error
regression fail as expected. Production files and the parent's
`round1-lifecycle.py` / `.rs` fixtures were not edited.

## Oracle and scope

README's semantics and `MountRuntime.user_suppressed` document that an explicit
mount ends manual suppression. The original breaker establishes that this
includes an accepted attempt that fails. Name validation and acquiring the
operation claim define the relevant admission boundary. Sleep, shutdown,
unknown-name and conflicting-claim refusals must preserve previous intent.

The correction clears suppression immediately after that claim and before
mount-table enumeration, its already-mounted shortcut, or any helper work.
It changes intent only. The endpoint eligibility and initial/final probe gates,
credential cleanup refusal and operation/lifecycle ownership still apply.
An existing volume must not be stacked or attributed to the configured endpoint
without evidence. Status is updated by reconciliation; this review does not
invent a requirement for synchronous status refresh on a shortcut.

Failed reloads preserve config and runtime. Successful reloads publish validated
config and clear runtime while holding lifecycle ownership. Cancellation while
waiting for that ownership must not publish the new file. These expectations
come from README, the reconciler audit, and the E1/E3/E4/E5 dispositions.

Read ponytail, unslop and realistic-commits, the six requested audit reports,
README, the reconciler audit and the relevant engine/state/reconcile/FakeOps
paths. The branch's remote-tracking reflog records `update by push` at
`2026-11-12 08:50:58 +1000`, targeting `b4f33bf`. A read-only hosting-event lookup
returned no matching event. No commits, pushes, branches, worktrees, delegation,
authentication or policy changes were made. The explicit no-commit instruction
supersedes the commit skill's normal workflow.

## New synthetic challenges

The new injector reuses `tests/portable-reconciler.py` without editing it. It
appends only the review module to a disposable engine copy. The package identity
is `mountd-round1-lifecycle-review-fixture`; the cache is `target/portable-daemon`.
All SystemOps methods remain hard failures. Documentation-range addresses are
FakeOps keys only. Dependencies, credentials, mount enumeration and effects are
synthetic. Tests use owned temporary config files, with no network sockets,
real mounts/unmounts, Keychain access, LaunchAgents or app changes.

Every case starts with a successful fake mount followed by explicit unmount,
with `when_held = true`, so a condition edge cannot silently repair old intent.

| New test | Challenge |
| --- | --- |
| `accepted_failures_clear_intent_without_bypassing_gates` | Seven accepted failures: enumeration, initial probe, requires, only_if, missing credential, CleanupPending and final probe. No mount may start on failure. Restoring availability permits one reconcile mount with no wake/reload/condition edge. |
| `refusals_preserve_both_existing_intents_and_runtime` | Sleep, shutdown, unknown name and claim conflict, each with suppression set and clear. Assert exact refusal, unchanged runtime/status, no invented unknown-name runtime and no recorded Ops calls. |
| `aborted_accepted_mount_keeps_new_intent_and_cleanup_ownership` | Cancel accepted mount at credentials, preparation and helper wait after manual unmount. New intent remains; claim/lifecycle writer stay unavailable until synthetic reap. Reconcile and explicit unmount cannot overlap cleanup. Direct reconcile can mount afterward. |
| `final_sleep_or_shutdown_refusal_does_not_restore_old_intent` | Sleep/shutdown arrives during accepted preparation. The final gate refuses the effect without reviving old suppression. Wake permits a retry only in the sleep case. |
| `shortcut_preserves_actual_sources_and_allows_retry_after_external_loss` | Existing source B and an unparseable source. Shortcut clears intent, avoids helpers/probes and does not invent A. Reconcile attributes B or unknown, probes B when known, then mounts A after external disappearance without wake/reload. |
| `failed_reload_preserves_intent_and_successful_reload_resets_it` | Missing/invalid config after either an untouched manual unmount or an accepted failed mount. Failed reload preserves intent, status and claim identity. Valid replacement resets runtime, replaces the claim and mounts endpoint B. |
| `cancelled_queued_reload_cannot_reset_accepted_intent_or_publish_config` | Abort accepted credential wait, write config B and cancel reload while cleanup holds the lease. Config A and new intent survive; A mounts after reap. Later successful reload adopts actual A despite configured B. |

Seven tests contain 27 cases. These are new hypotheses and assertions, not a
rerun of the parent's eight regressions. A negative control removes only the
new suppression-clearing block inside the disposable crate, then runs the
accepted-failures test. It fails on the enumeration case's suppression assertion, as expected. The repository's engine is never rewritten for this control.

## Commands and results

Commands run from `/home/christian/Documents/dev/mountd`. Each invocation takes
and releases the shared lock separately, with one bounded Cargo suite per
acquisition. Nothing holds the lock during review or report writing.

```sh
flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout --signal=TERM --kill-after=10s 210s \
  env TMPDIR=/home/christian/.local/share/agent-tmp \
      CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  python3 tests/audit/round1-lifecycle-review.py \
  > tests/audit/round1-lifecycle-review-fixed.log 2>&1

flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout --signal=TERM --kill-after=10s 210s \
  env TMPDIR=/home/christian/.local/share/agent-tmp \
      CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  python3 tests/audit/round1-lifecycle-review.py --without-fix \
  > tests/audit/round1-lifecycle-review-negative.log 2>&1
```

The injector runs `cargo --offline --locked test --lib round1_lifecycle_review
-- --nocapture` with a 180-second timeout. The negative control selects only
`round1_lifecycle_review::accepted_failures_clear_intent_without_bypassing_gates`.
Each log records the exact command, temporary crate path and SHA-256 hashes of
all source modules, Cargo manifests, generator, test module and injector.
Fixed result: exit 0, `7 passed; 0 failed; 208 filtered out`. Compilation took
19.70 seconds and the selected tests took 0.16 seconds. No existing tests were
selected. Negative-control result: exit 101, `0 passed; 1 failed; 214 filtered
out`. Compilation took 17.86 seconds and the selected test took 0.01 seconds.
The failure is the assertion that suppression must be clear after an accepted
mount-table enumeration error. This is expected negative-control evidence,
not a defect in the corrected engine. Both suites completed within their
execution bounds; shared-lock waits preceded compilation.

Python syntax, Rust formatting and owned-file whitespace checks pass. The engine,
state, Ops, reconcile, generator and new fixture hashes match across both logs
and the final checkout check. Both disposable crates were removed by the runner.
The commands above are sufficient to reproduce the evidence. Retained files are:

- `tests/audit/round1-lifecycle-review.rs`, seven new tests.
- `tests/audit/round1-lifecycle-review.py`, bounded injector and negative control.
- `tests/audit/round1-lifecycle-review-fixed.log`, passing suite and source hashes.
- `tests/audit/round1-lifecycle-review-negative.log`, expected failure and hashes.
- `tests/audit/round1-lifecycle-review-checks.log`, static checks and source identity.

No new audit scope or workers followed the checkpoint instruction. Review work
is complete and stopped for the parent's serial checkpoint review and push.

## Dispositions and limits

No additional production correction is proposed. This independent challenge
accepts R1-L1 within the synthetic lifecycle scope. Parent-reported eight regressions,
47 engine / 25 reconcile / 5 state / 3 Ops tests and Clippy are prior evidence,
not reruns or independent full-suite acceptance by this reviewer.

E1 through E6, RNI-1 through RNI-4, resolver accumulation and RF-1 including
its signal follow-up retain their recorded dispositions. This review does not
reopen or strengthen their acceptance. In particular:

- Direct-child reap and process-group signalling do not prove arbitrary
  descendants have finished. Dependency commands remain trusted foreground work.
- SIGKILL, runtime destruction and OS teardown defeat in-process retention.
  Permanently blocked kernel/libc work can retain ownership and delay shutdown.
- SMB passwords remain visible in helper argv until exit; helper output and
  argv must remain absent from error messages.
- A same-user rename or external mount can race the final path-based syscall.
  Conservative case/Unicode checks can reject distinct paths.
- TCP reachability covers the configured service endpoint, not ancillary
  NFSv3 mountd/RPC services or native protocol negotiation.
- Synthetic FakeChild reap does not prove native helper topology, Darwin
  enumeration, Keychain, APFS, IOKit, launchd or Swift/Foundation behavior.
  Mac acceptance remains pending exactly as in `mac-verification.md`.

The concurrent menu option parsing and escaped-name work in config/IPC/Swift,
and installer exclusion in `menubar/build.sh`, are outside this review. Their
completion and independent checks belong to their current owners. Source hashes
identify the compilation snapshot without accepting those changes. The parent
owns serial integration, broad verification and publication. Repository writes by this reviewer are limited to this report and
`tests/audit/round1-lifecycle-review*`.
