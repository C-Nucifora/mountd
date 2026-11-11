# Extended audit rounds

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
publication. The prior last-push lookup found no reliable timestamp; use the
realistic-commits past-day fallback for new signed commits.

## Round 1: cancellation sequences and IPC admission

Status: breakers starting. This round does not count as complete.

Initial tool-created worktrees belonged to the unrelated default project. Both
workers refused that checkout before editing or running tests. The delegation
tool also rejected a different local project target. Replacement breakers use
the explicit mountd path with disjoint report/fixture ownership. They inject
tests into disposable portable crates and cannot edit production files.

The baseline source hashes still match all retained passing evidence. The four
original signed commits are ready for the first authorized topic push; no new
round is claimed by that publication. This ledger's commit adds only audit
tracking and clarifies the historical verification record.

| Area | Hypotheses and new coverage | Evidence and result |
| --- | --- | --- |
| Engine state | Interleaved explicit unmount, reload, sleep cancellation and shutdown can expose stale claims or wrong source attribution beyond the prior individual regressions | Pending independent bounded sequence tests |
| IPC/config | Slow or disconnected clients, connection saturation and response size limits can leave retained work or inconsistent command outcomes; configuration boundaries can interact with wire encoding | Pending independent fault fixtures |

Confirmed findings, reproduction commands, separate fixes, focused/full checks,
fresh review objections and publication hashes will be recorded here as each
stage completes. Round 2 will target process/credential/filesystem failures;
round 3 will target installer/UI recovery and integrated behavior. Later rounds
will follow evidence rather than repeat identical checks.
