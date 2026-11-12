# Round 2 credential parser fix

Scope is R2-CRED-1 in `src/cred.rs`. Parent review and integration remain
required. No commits or pushes are authorized for this implementation task.

## Baseline and contract

Checkout: `/home/christian/Documents/dev/mountd`, branch
`hardening-audit-rounds`, HEAD
`171eab5b7d8d8d183f2cb2cb39df745a5f32631f`, with `0818999` verified as an
ancestor. Origin fetch and push URLs are
`https://github.com/C-Nucifora/mountd.git`. The branch tracking reflog records
`update by push` at `2026-11-12 10:30:07 +1000`; a read-only GitHub push-event
query returned no matching event. The explicit no-commit instruction overrides
commit scheduling.

Read the supplied instructions, ancestor/repository instruction paths, ponytail,
unslop and realistic-commits, README's credential contract, process P4,
review-disposition, round2-credential-path-breaker and mac-verification. No
additional AGENTS.md was found. Before editing, searched all
`parse_password_metadata` callers. The only production caller is
`disambiguate_hex`; the other callers are parser tests.

P4 requires ambiguous or malformed credential metadata to fail closed. The
retained breaker log records five passes and two failures, with source bytes
unchanged. Its exact failing tests are
`truncated_single_quote_must_fail_closed` and
`contradictory_records_must_fail_closed`. A successful synthetic `-w` hex
response followed by a successful `-g` lone quote returned `Found`; conflicting
literal/encoded records were accepted in both orders. This is defensive parser
behavior, not evidence that native security emits these records.

Baseline `src/cred.rs` SHA-256:
`bc985636127d2b7cf9217934e490ee9cf728081ffd57cce4e3a2d2cfc3446215`.
The original `tests/audit/round2-credential-path-suite.log` is preserved, SHA-256
`c28f2995da17a656c3c2b73c1ef129a254d7cfa639c96541d2c0d664edf6f540`.

## Change and edge cases

The parser now iterates over password records and refuses a second record
before interpreting the first. This rejects conflicting records in either
order, identical duplicates and a valid record accompanied by a malformed or
empty record. It retains the existing indented `password:` recognition and
ignores unrelated metadata lines. It does not collect records or allocate
additional secret strings to count them.

Literal detection strips an opening quote, then requires a closing quote in
the remainder. A lone quote therefore fails; two distinct quotes still
represent a valid literal indicator. Errors remain static and contain no input.
The hex decoder, literal-hex behavior, empty first-response behavior, one-newline
removal, shared deadline and lease handling are unchanged. No caller changes
are needed.

Two source-level tests next to the existing parser test cover the exact
truncation and conflicting-order failures, missing opening/closing quotes,
identical literal and encoded duplicates, malformed/empty extra records,
indented records, unrelated metadata, CRLF, empty literal metadata, literal
hex, encoded UTF-8 and an encoded trailing newline. These assertions have been
added but not executed; formatting is not test acceptance.

Native grammar uncertainty remains. This fix preserves existing handling of
embedded literal quotes/escapes and the optional display text after encoded
hex. It neither validates a new native grammar nor compares the contents of
the two successful subprocess responses. No new rejection or acceptance rule
for those shapes is introduced. A disposable-Mac check must establish native
format compatibility.

Final `src/cred.rs` SHA-256:
`460fdeedc8bd07c5cc452f726e1bd6bab23de718dac98d234f45b2e459fe049d`.

## Verification and resource limit

Every shell command used explicit cwd `/home/christian/Documents/dev/mountd`.
The baseline rerun used the retained, unmodified fixture:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 900 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round2-credential-path.py \
  > tests/audit/round2-credential-fix-baseline.log 2>&1
```

The outer command exited **75** after the 900-second lock acquisition limit.
The suite never started and produced zero bytes. The baseline log now records
that command, status and lack of execution. No fixture, Cargo invocation or
synthetic child was created by this attempt. No lock acquisition was retried.
The retained earlier log remains the only executed reproduction: five passes,
two failures, Cargo exit 101, unchanged baseline source. No new baseline
reproduction is claimed.

The fixed-source breaker, new source regressions, focused credential tests and
`tests/portable-process.py` are **not run** because the resource slot was not
acquired. `tests/audit/round2-credential-fix-focused.log` records that status,
the fixed hash and the intended commands. There is no fixed-source Cargo exit
code and no runtime pass claim. Parent verification must use separate lock
acquisitions, with `TMPDIR=/home/christian/.local/share/agent-tmp`,
`CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `RUST_TEST_THREADS=1` and
`CARGO_TARGET_DIR=/home/christian/Documents/dev/mountd/target/portable-daemon`.
Release the lock between suites; bound each execution to 235 seconds plus at
most five seconds of termination cleanup. The retained breaker has its own
distinct fixture identity and source-byte checks. The portable generator uses
its existing separate package identity and disposable source copies.

Lightweight checks are recorded with exact argv, exit codes and hashes in
`tests/audit/round2-credential-fix-checks.log`:

- `rustfmt --edition 2021 --check src/cred.rs`, exit 0.
- `git diff --check -- src/cred.rs docs/audit/round2-credential-fix.md`, exit 0.
- Baseline source manifest verification, exit 0 before editing.
- Final non-owned source and retained breaker input/log identity checks,
  exit 0. Only `src/cred.rs` differs from the original source manifest.

## Handoff files

- `src/cred.rs`
- `docs/audit/round2-credential-fix.md`
- `tests/audit/round2-credential-fix-baseline.log`
- `tests/audit/round2-credential-fix-focused.log`
- `tests/audit/round2-credential-fix-checks.log`

Only the source file is a tracked product change. The pre-existing parent
ledger edit and breaker artifacts are preserved. No branch, worktree,
top-level thread, subagent, commit or push was created. Parent review and
runtime verification are still required.

## Native limits and scope

No `/usr/bin/security`, ignored native credential integration test, real share,
production config, installed app or LaunchAgent is used. The breaker fixture
crosses the real subprocess boundary with isolated synthetic helpers only.
Native Keychain metadata, escaping, ACL and prompt behavior remain Mac
acceptance requirements. Apple-target and broader integration checks, and a
fresh independent review, belong to the parent.

Existing argv exposure, direct-child ownership, arbitrary-descendant limits,
kernel/libc retention, path races and native acceptance limits remain as
documented in review-disposition and mac-verification. The separate
provider-refused process/drain audit remains stopped. None of its artifacts or
implementation is read, changed, adapted, run or reassigned by this task.
