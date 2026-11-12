# Extended audit rounds

The user resumed the approved audit after checkpoint `0be609a`. The deferred IPC and installer challenges completed without a new confirmed
defect; round 2 now targets process, credential and filesystem boundaries. The checkpoint and its evidence remain intact. Round 1 is complete within the recorded portable scope. Rounds 2 and 3 and
the two-consecutive-clean-round requirement remain in force.

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

Status: complete within portable coverage after four confirmed fixes and fresh
lifecycle, IPC and installer challenges. It is not a clean round because the
breakers found substantive defects. Consecutive clean rounds: zero. Native
acceptance and explicitly unrun supplemental cases remain separate.

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

## Resumption after checkpoint

The latest instruction revokes the pause. Start with two Astra/high/priority
reviewers, disjoint report/fixture ownership and no production edits. Current
host headroom permits this small swarm; all heavy jobs still acquire and release
the shared lock individually. No new top-level thread or checkout is created.
The tool-created worktree mismatch described above remains a reason to use the
explicit mountd path and isolated disposable test crates.

The IPC reviewer must challenge the option terminators and encoded request
boundary with new bounded cases, then inspect nearby transport/admission
behavior. The installer reviewer must challenge transaction exclusion and
rollback lifetime with new controlled interleavings and retain RF-1 uncertainty
semantics. Both receive the earlier findings, responses and native limits.
Confirmed findings need reproducible evidence; speculative concerns are coverage
gaps rather than defects. Fresh review results will determine integration and
the next round. No refused testing payload is being retried.

### Resumed IPC review

The fresh [independent IPC review](round1-ipc-review.md) found no new defect in
24 generated encoding boundaries, 65 control-character rejections, 39 actual
CLI dispatches and a combined cancellation/saturation/shutdown case. Three new
test functions passed. Parent reviewed the fixture logic and verified its source
hashes, then signed and published `b400b9d`, `challenge control names and shutdown
admission`. Production files are unchanged.

The reviewer's two negative controls did not execute: each exhausted its
600-second shared-lock acquisition wait. Their logs state that limit rather
than claiming regression failures. Parent queued separate 900-second bounded
acquisitions for those controls, releasing the lock between them. This is a
resource retry, not a retry of refused testing. Installer review remains live;
round 1 is not yet closed and no clean-round count advances.

### Resumed installer review and round-1 disposition

The [independent installer review](round1-installer-review.md) passed ten new
cases: second-lock refusal/release, simultaneous cold starts, four malformed
lock-path cases and four rollback inspection/mutation failures. The new
signal-guard removal control failed as required. Parent reviewed the source,
oracles, hashes and cleanup evidence and published signed commit `0818999`,
`challenge installer locks and uncertain rollback`. Only a blank EOF line in the
finished checks log was removed before commit. Production is unchanged.

The owner's TERM/HUP observations did not run because their 600-second lock
wait expired. This is recorded as a supplemental gap, not a discovered product
defect. Native zsh/APFS and launchd semantics remain open. The earlier passing
full suites remain applicable because these reviews changed only fixtures and
reports. No substantive round-1 finding or independent objection remains within
the exercised portable coverage. Round 1 closes with four fixed findings and
zero consecutive clean rounds; it does not satisfy the clean-round rule.

## Round 2: process and credential boundary compositions

Status: process/drain task stopped after provider refusal. The independent
credential/path task remains active within its original scope. Neither may edit
production. Round 2 is incomplete and cannot count as a clean round.
This is a new round, not a relabelling of the round-1 review results.

| Owner | New hypotheses and bounded coverage |
| --- | --- |
| Process/drain | Combine exact per-stream output limits, simultaneous stdout/stderr pressure, inherited-pipe EOF, receiver cancellation and lease/drain lifetime; exercise resolver saturation, cancellation, release and admission together |
| Credentials/paths/options | Compose both credential reads with malformed metadata, deadline exhaustion and cancellation; challenge path preparation with local non-directory/symlink errors and compare option validation to argv without executing mounts |

Each breaker should select a small nonredundant matrix from source review and
run one focused bounded suite before considering extras. The shared queue is
busy, so keep build/test parallelism at one and release the lock between suites.
No new native acceptance claim follows from these fixtures. Any confirmed issue
requires separate fix ownership, integration checks and a fresh challenge;
otherwise preserve rejected hypotheses and exact evidence without inventing work.

### Round-2 provider refusal

The process/drain delegation was refused with: "This content was flagged for
possible cybersecurity risk." That entire task is stopped. It will not be
retried, rephrased, performed by the parent or reassigned through another
provider/model. The task is terminal with no pending child runs. A previously
queued shell command remained after the failure: parent verified PID 1351760
was the mountd `round2-process-runner.py` flock waiter with no children, then
sent SIGTERM before test execution. Other projects' similarly named processes
were left untouched.

Partial `tests/audit/round2-process*` artifacts are preserved locally. No
completed execution or review acceptance has been established, and these files
are not being published. No round-2 process result is claimed. The existing credential/path delegation
continues within its original separate scope; it will not take over the refused coverage. This is an explicit
coverage blocker, not evidence of a product defect or a clean round.

### Parent supplemental IPC control

The config guard-removal control acquired the shared lock near the 900-second
bound and ran. It exited 101 at the intended regression assertion: seed 1,
offset +1 serialized named frames of 65,535 / 65,537 / 65,537 bytes while the
mutated config validator accepted the name. The new test therefore detects the
known length defect. Exact source hashes, command and output are retained in
[`round1-ipc-controls-config.log`](../../tests/audit/round1-ipc-controls-config.log).
Only the disposable copy changed; production files remain unchanged. The menu
control subsequently ran under its separate acquisition and exited 101 at the
intended CLI assertion. With only the menu terminators removed, the literal
name `--` was parsed as an option terminator, leaving NAME missing and producing
CLI exit 2 instead of a successful dispatch. The log is
[`round1-ipc-controls-menu.log`](../../tests/audit/round1-ipc-controls-menu.log).
Both parent controls therefore failed as required, and the parent runner
completed with exit 0. Earlier reviewer lock-expiry logs remain historical
evidence, not overwritten results. No production edit or additional full-suite
rerun was needed for these disposable-copy controls.
