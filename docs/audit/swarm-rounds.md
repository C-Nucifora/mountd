# Extended audit rounds

The user requested a checkpoint followed by a pause during round 1. No further
workers or rounds will start. Signed topic-branch commits preserve the reviewed
changes and verification evidence. Round 1 is not a completed clean round;
rounds 2 and 3 and the two-consecutive-clean-round requirement remain deferred.

The user reopened the audit on 2026-11-12 and authorized topic-branch pushes.
Baseline: `a66778e`. The prior findings and acceptance limits remain recorded in
[review dispositions](review-disposition.md) and [coverage](coverage.md).
Mac acceptance remains user-run; no native result has been supplied.

## Completion rule

Run at least three new rounds, then continue until two consecutive independent
rounds report no substantive unresolved defects in the coverage matrix. A round
must exercise new hypotheses with bounded synthetic evidence. Existing passing
tests alone do not count. Confirmed findings require separate fix ownership,
serial integration, focused and full applicable checks, and a fresh independent
challenge. A clean round does not require invented fixes.

## Resource and publication controls

Start with two breakers and no nested delegation. Other project owners also use
small swarms. Every build, test, fuzz or browser job takes
`/home/christian/.local/state/agent-swarms/heavy.lock`. Use disk TMPDIR, one Cargo
job, no incremental compilation and one test thread. No real shares, credentials,
installed apps or LaunchAgents are used.

Work branch: `hardening-audit-rounds`, based on the four signed local commits.
Fetched `origin` before starting; `origin/main` remains `b39ea7e`. The only
workflow runs checks on main pushes and pull requests; no deployment or release
trigger exists in the checkout. Topic pushes therefore do not run that workflow.
No merge, PR, release or deployment is planned. Recheck remote and triggers before
publication. The initial last-push lookup found no reliable timestamp, so the first batch
used the realistic-commits past-day fallback. Subsequent push reflog entries
provide exact checkpoints: `b4f33bf` at 08:50:58, `2c01b86` at 09:15:04 and
`674de98` at 09:16:59, all 2026-11-12 +10:00. Later commits retain chronological
author/committer timestamps after their published parents and configured SSH
signing. No published history was rewritten.

## Round 1: cancellation sequences and IPC admission

Status: checkpointed after four confirmed fixes. This round does not count as
complete. Fresh independent IPC and installer fix challenges remain deferred.

Initial tool-created worktrees belonged to the unrelated default project. Both
workers refused that checkout before editing or running tests. The delegation
tool also rejected a different local project target. Replacement breakers use
the explicit mountd path with disjoint report/fixture ownership. They inject
tests into disposable portable crates and cannot edit production files.

The baseline source hashes identify the original audit snapshot. Round-1
changes have a separate checkpoint manifest below. Published
the four original signed commits plus ledger commit `b4f33bf` to
`origin/hardening-audit-rounds` at 2026-11-12 08:50:58 +10:00. All five signatures,
message rules and chronological timestamps verified. Remote HEAD matches the
push, and remote main remains `b39ea7e`. No new round is claimed by that
publication. The ledger commit adds only audit tracking and clarifies the
historical verification record. No PR, merge, release or deployment was created.

| Area | Hypotheses and new coverage | Evidence and result |
| --- | --- | --- |
| Engine state | Interleaved explicit unmount, reload, sleep cancellation and shutdown can expose stale claims or wrong source attribution beyond the prior individual regressions | [R1-L1](round1-lifecycle-breaker.md) confirmed twice, six new tests pass and two fail; 256 modeled sequences pass. Parent applied the prescribed small intent fix; all focused/full checks and a fresh independent challenge pass |
| IPC/config | Slow or disconnected clients, connection saturation and response size limits can leave retained work or inconsistent command outcomes; configuration boundaries can interact with wire encoding | [R1-IPC-1/2](round1-ipc-breaker.md) confirmed; nine new tests pass and two fail. Menu option terminators and exact encoded-name budget applied; focused and full checks pass; fresh independent challenge deferred |
| Installer overlap | One failed install can roll back another successful update | Confirmed by the parent's new bounded fake-tool fixture; [R1-INSTALL-1](round1-installer-breaker.md), separate fix and positive/negative checks pass; fresh independent challenge deferred |

The reports below preserve reproduction commands, fixes, checks and review
objections. On restart, finish the deferred round-1 independent challenges
before counting a completed round. Proposed round 2 targets process, credential
and filesystem failures; proposed round 3 targets UI recovery and integrated
behavior. Later work should follow evidence rather than repeat identical checks.

### Round 1 triage and implementation

- R1-L1, medium: accepted against the documented explicit-mount intent contract.
  The parent applied the breaker's small correction after name/claim validation
  and before the mounted shortcut. No helper, probe, lease or unmount behavior
  was changed. New lifecycle tests and the full portable reconciler suite
  passed: eight new lifecycle tests, all 47 engine/25 reconcile/5 state/3 Ops
  tests and fixture Clippy. A fresh independent review passed seven additional
  tests containing 27 cases, with its negative control failing as required and
  no unresolved objection. Signed commit `674de98`, `honor explicit mount intent`,
  was pushed normally to the topic branch.
- R1-INSTALL-1, medium: accepted after the controlled overlap reproduction.
  A separate Sol/high/priority fixer added fail-fast build/install locks retained
  through EXIT rollback. Parent source/fixture review found no further defect.
  Existing recovery cases, overlap at four transaction phases, retry, directory
  permissions and child descriptor checks pass. Both the original reproduction
  and lock-removal negative control fail as expected. See the
  [fix report](round1-installer-fix.md). Signed commit `c367823`,
  `serialize menu installer transactions`, was pushed at 09:24:24 +10:00 and
  records the fix and evidence. Native
  lock availability and behavior remain Mac acceptance items.
- R1-IPC-1, medium: accepted. Menu mount/unmount now insert `--` before the
  configured name. Ordinary CLI help behavior is unchanged.
- R1-IPC-2, low: accepted. Config and IPC share the request cap. Config counts
  JSON-escaped name bytes plus the longest named-command envelope and newline.
  Six new exact-boundary cases compare validation with actual serialization of
  mount/unmount/explain, including quotes, backslashes and multibyte Unicode.
  All 12 focused tests pass: the breaker's 11 plus the exact-boundary test.
  Full IPC/config/CLI checks pass, including 27 config, 10 IPC, three wire tests,
  actual CLI cleanup-drain and Clippy. Prior failing logs are retained separately.
  Parent review and limitations are in [the correction record](round1-ipc-fix.md).
  Signed commit `2c01b86`, `keep mount names addressable`, was pushed normally to
  `origin/hardening-audit-rounds`. The process suite subsequently passed 103 tests
  with five Keychain tests ignored, plus Clippy.

The global queue grew during these checks. Subsequent jobs release the shared
lock between suites, with a bounded execution deadline after acquisition.

## Coverage gaps selected for subsequent breakers

Parent source review identified these untested combinations, not confirmed
defects. Independent breakers must reproduce or reject them before triage.

- Process round: combine simultaneous stdout/stderr pressure with receiver
  cancellation and child exit; test exact output limits and EOF ordering. Existing
  supervisor tests do not by themselves establish every ordering.
- Process round: resolver saturation followed by release, cancellation and new
  lookup admission; verify activity drain and capacity recovery together.
- Installer round: inject uncertainty during rollback itself. Round 1 now covers
  overlap during write, rollback restore/restart and cleanup; do not count those
  same cases again as new coverage.
- UI round: extend the native command fixture with inherited pipes, exact output
  boundaries and repeated timeout/recovery. The current Swift fixture covers
  ordinary output/error, missing executable, one resistant timeout and flooding.
  Swift/Foundation execution remains a Mac check; Linux must not substitute its
  process implementation and claim native acceptance.

## Checkpoint verification and publication

These checks ran for round 1 on Linux, separately from the first audit's logs.
Suites overlap; do not sum their counts into a unique-test total. Every heavy
suite used the shared `heavy.lock`, disk TMPDIR, one build job and one test
thread. The final batch released the lock between suites, with a 900-second
acquisition limit and a 240-second execution limit per suite.

| Check | Result | Evidence |
| --- | --- | --- |
| New lifecycle breaker cases | Eight pass, including 256 modeled sequences | [Focused log](../../tests/audit/round1-lifecycle-fixed.log) |
| Fresh lifecycle fix challenge | Seven tests covering 27 cases pass; removing only the fix fails | [Independent review](round1-lifecycle-review.md) |
| Full reconciler fixture and Clippy | 47 engine, 25 reconcile, 5 state and 3 Ops tests pass | [Full log](../../tests/audit/round1-reconciler-full.log) |
| New IPC/config/menu cases | Ten unit and two actual CLI tests pass | [Fix record](round1-ipc-fix.md) |
| Full IPC/config/CLI fixture and Clippy | 27 config, 10 IPC, 3 wire and 1 actual CLI cleanup-drain test pass | [Full log](../../tests/audit/round1-ipc-full.log) |
| Full process fixture and Clippy | 103 pass; five real-Keychain tests ignored | [Full log](../../tests/audit/round1-process-full.log) |
| Actual daemon entry point with fake effects | Singleton refusal, accepted IPC drain and child disappearance pass | [Full log](../../tests/audit/round1-daemon-full.log) |
| Portable native fixture | 16 unit tests, Clippy, build and fake launchd transaction checks pass | [Full log](../../tests/audit/round1-native-full.log) |
| Apple-target Clippy | `cargo clippy --locked --target aarch64-apple-darwin --all-targets -- -D warnings` passes | [Full log](../../tests/audit/round1-apple-clippy.log) |
| Formatting and fixture syntax | Cargo/Rust formatting, shell syntax, Python parsing and whitespace checks pass | Parent checkpoint command output |
| Sequential and overlapping installer fixtures | Recovery, permissions, descriptor ownership and overlap pass; two negative controls fail as required | [Results](../../tests/audit/round1-installer-results) |

The [round-1 source manifest](results/round1-source-sha256.txt) identifies the
checkpoint's production files and fixtures. Verify it from the repository root
with `sha256sum -c docs/audit/results/round1-source-sha256.txt`. Historical
execution-snapshot hashes in breaker logs and the first audit manifest remain
unchanged. Rust fixture formatting after execution changed whitespace only.
Finished logs were normalized only for trailing whitespace and blank EOF lines.

Published fix commits on `origin/hardening-audit-rounds`:

- `2c01b86` keep mount names addressable
- `674de98` honor explicit mount intent
- `c367823` serialize menu installer transactions

No main-branch update, PR, merge, release or deployment occurred. The existing
workflow runs on main pushes and PRs, so these topic pushes do not trigger CI.
Native Mac execution, Swift build/accessibility, Keychain and real-share
acceptance remain unverified. The user will run the
[Mac checklist](mac-verification.md); disposable credentials/shares still need
separate authorization. The known SMB argv, descendant-process, kernel-work,
pathname-race and NFS ancillary-service limits in the original audit remain.

No temporary mountd service or test child remained after its fixtures. Listener
inspection found no mountd listener, and the rootless Docker daemon had no
running containers. Existing unrelated host services were left unchanged.
No production config, credentials, app or LaunchAgent was accessed or changed.

Checkpoint checks completed successfully. The final evidence commit is titled
`record round one checkpoint evidence`; its hash is available in the branch log.
All fix signatures were verified against the configured public signing key.
Further audits are paused at the user's request. Resume with the deferred
independent IPC/installer challenges, then the remaining new rounds. No clean
round or native acceptance is claimed by this checkpoint.
