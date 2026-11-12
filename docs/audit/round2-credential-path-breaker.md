# Round 2 credential, path and option breaker

One medium finding is confirmed: malformed or contradictory credential
metadata can produce a found credential instead of failing closed. The one
focused suite completed with five passing tests and two deterministic failing
regressions. No production changes were made. Native acceptance remains open.

Reviewed `/home/christian/Documents/dev/mountd`, branch `hardening-audit-rounds`,
base `08189996c009a3538d7eec1ba7e2680975505d5a`, descended from `0be609a`.
Origin is `https://github.com/C-Nucifora/mountd.git`. The tracking reflog records
`update by push` at 2026-11-12 10:17:16 +1000. The read-only GitHub repository
push-event query returned no matching event. No commit timing action applies. During this review the parent advanced HEAD
to `4ef4824485963172cb471f420765d1fc77a0b156`, pushed at 10:22:54 +1000.
That commit changes audit evidence only; every captured source hash still
matches the starting checkpoint.

Read the supplied ancestor/repository instructions. No additional AGENTS.md
was found in the repository or ancestor directories. Applied ponytail, unslop
and realistic-commits under the explicit no-production-edit/no-commit rule.
Read README, coverage, review-disposition, process, ipc-config,
review-engine-process, review-final, round1-ipc-review, round1-installer-review,
mac-verification and the parent ledger. Current README and integrated
review-disposition take precedence over historical statements that permitted
proto=tcp or described uncapped resolver accumulation.

Source inspected: `src/cred.rs`; option validation, argv construction, lexical
comparison, preparation and credential-supervisor interfaces in
`src/mountcmd.rs`; mount validation and collision checks in `src/config.rs`;
credential dispatch in `src/engine.rs` and `src/ops.rs`; `src/mtab.rs`; and
`tests/portable-process.py`. Dependency modules copied by the generator are
hashed but are not new resolver, dependency or watcher review claims.

## Focused oracle and fixture

The credential contract is process P4's explicit requirement that ambiguous or
malformed metadata fail closed, together with README's literal-hex versus
encoded-password distinction and no-secret diagnostic promise. E1 requires one
absolute caller deadline, retained operation ownership until child reap and
preserved CleanupPending. README and E2/E6 specify additive safe options,
matching probe/service ports, lexical alias/nesting refusal and no symlink walk.
The tests do not impose a new exhaustive native-option allowlist.

The generator derives its module list and crate setup from portable-process.py.
It copies production module bytes unchanged and appends two test modules only
in its private crate. Its unique package identity is
`mountd-round2-credential-path-breaker-fixture`. The watcher notification target
is the same isolated stub as the existing generator. Production mount-table
code remains Unsupported on Linux. No native behavior is replaced to pass.

A private Python executable handles only synthetic `-w` and `-g` invocations,
records each direct-child PID, and emits bounded configured stdout/stderr.
Neither `/usr/bin/security` nor a mount/unmount executable is invoked. All
credentials are distinctive fake strings. Error details, reason strings and
Debug are checked for raw, hex, URL-encoded and partial secret fragments.
Helpers have no descendants, listeners or network access. Each delay is at most
three seconds. Every executed case drains registered work and checks its recorded
PIDs are gone before its temporary directory is removed.

The seven test functions cover these selected combinations:

- Eight normal/rejected metadata shapes, plus separate failing oracles for a
  truncated lone quote and contradictory literal/encoded records in both orders.
- Nine first/second-call failure and output-cap cases. A first-call error must
  never start `-g`; second-call exit 44 must remain a disambiguation failure.
  Caps are exercised independently on each stream at the credential boundary.
  Five literal, empty and trailing-newline outputs verify interpretation without
  inventing rejection rules for odd-length or uppercase literal passwords.
- Zero budget, first-call exhaustion, two individually short reads that exceed
  one total budget, and cancellation after the second helper publishes its PID.
  A Drop observer checks both helper PIDs at lease release, followed by drain
  and weak-reference checks. These call the real credential boundary; they do
  not inject supervisor cleanup or resolver implementation behavior.
- Thirty-eight config/argv option pairs across NFS and SMB. Duplicate tokens,
  empty comma fields, surrounding/internal whitespace, mixed-case safety keys,
  numeric port spellings, port conflicts and invalid numbers are compared with
  the shared validator. Accepted options retain defaults and remain one argv
  element; URLs and exports preserve argument boundaries. Explicit empty config
  strings remain rejected while direct option merging retains defaults.
- Six local file/symlink/non-directory preparation failures, unchanged target
  bytes and directory entries, repeated successful private-directory creation,
  lexical normalization without symlink traversal, relative/dot-path refusal,
  and five collision/nesting pairs including composed/decomposed Unicode and
  distinct non-ASCII names conservatively rejected by the existing policy.

These are seven test functions, not a sum of matrix rows as independent tests.
No actual mount-table traversal, native parser, APFS identity, delayed kernel
reap, engine reload or passwordless mount is claimed by this fixture.

## R2-CRED-1, medium: malformed metadata succeeds

`src/cred.rs:250` selects the first password record with `find_map`, then
accepts literal metadata whenever the trimmed value starts and ends with a
quote. A one-character quote satisfies both predicates. `disambiguate_hex`
returns None and `fetch_inner` then returns the original hex string as a found
literal credential. Additional conflicting password records are ignored.

The regression asserts Failed after a successful synthetic `-w` hex response
followed by successful `-g` containing only `password: "`. A second regression
asserts Failed for literal and encoded records together in either order.
The expected result follows the existing malformed/ambiguous fail-closed
contract. It makes no claim that native security normally emits these bytes.

The lone-quote regression failed with `Found([redacted])`. Both contradictory
record orders failed the required Failed-outcome assertion. The source explains
the returned values: a literal first record preserves the original hex string;
an encoded first record decodes it. All diagnostic-fragment assertions and
owned-child cleanup checks completed before those failing assertions.

This can turn an unreadable or ambiguous credential response into an attempted
SMB authentication with a misinterpreted password. It is a failure of the
existing defensive parsing contract, not evidence of native Keychain compromise
or a claim that real security normally emits malformed successful output.

Smallest correction: require
one unambiguous password record and distinct opening/closing literal quotes;
reject truncation or conflicting records with a static non-secret failure.
Preserve valid literal hex, encoded values, shared deadlines and lease passing.
Do not replace the platform credential API. A separate fixer owns production
changes and native grammar compatibility remains a Mac acceptance requirement.

## Commands and resource bounds

All commands run with explicit cwd `/home/christian/Documents/dev/mountd`.
The one focused suite ran with:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 900 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round2-credential-path.py \
  > tests/audit/round2-credential-path-suite.log 2>&1
```

Inside the lock the generator uses `target/portable-daemon`, one Cargo job,
serial tests and `cargo test --locked --offline --lib round2_credential_path --
--nocapture`, with a 215-second subprocess timeout. On timeout it kills and
waits only for its own Cargo process group, allows the finite helper bound, and
removes only its owned temporary root. The outer timeout permits five seconds
of cleanup within the 240-second execution ceiling. The shared lock is released
when the command finished. The lock was acquired within the 900-second bound.
Cargo and the outer command returned 101 for the two expected regression
failures. No second suite or retry was started.

## Completed evidence and unrun checks

The complete output is in `tests/audit/round2-credential-path-suite.log`.
Cargo compilation took 9.23 seconds, test execution 2.41 seconds, and the
runner recorded 11.733 seconds overall after acquiring the lock.

| Test | Result |
| --- | --- |
| metadata_valid_and_invalid_shapes | Pass, eight selected shapes |
| truncated_single_quote_must_fail_closed | Fail at the intended Failed-outcome assertion; Found is observed |
| contradictory_records_must_fail_closed | Fail at the intended assertion in both record orders |
| first_call_interpretation_and_cross_call_failure_caps | Pass, nine failure/cap cases and five literal/empty/newline cases |
| total_deadlines_and_cancelled_second_call_keep_ownership | Pass, four budget/cancellation cases, reap-before-release and post-drain release |
| local_preparation_errors_and_lexical_aliases | Pass, six refused walks, preserved targets, creation and alias cases |
| option_config_and_argv_matrix | Pass, 38 pairs and two explicit-empty boundaries |

The passing checks reject the selected hypotheses that first-call failures
proceed to `-g`, second-call failures become passwords, stream caps leak secret
diagnostics, the second call gets a fresh budget, cancellation releases the
credential lease before its direct child reaps, local refused paths alter their
targets, or option merging drops safety defaults or splits argument values.
These results establish only the selected inputs and portable caller behavior.
The documented direct-empty/config-empty distinction and conservative Unicode
rejection are preserved behavior, not new defects. Numeric and uppercase option
acceptance is observed at the Rust/argv boundary; Darwin interpretation remains
unverified.

All 108 pre-existing tests were filtered out. No existing full suite, Clippy,
Apple-target build, engine integration or native acceptance check was rerun.
No synthetic delayed-reap injection or resolver/supervisor implementation test
was added; the refused process/drain task remains stopped. No production fix or
fixed-source acceptance is claimed. The parent owns integration and a fresh
review after a separate fixer addresses R2-CRED-1.

The checks log records Python AST parsing, successful Rust fixture formatting,
ancestry, source identity and whitespace checks. An initial lightweight checker
mistakenly required exit 0 from `git diff --no-index --check` on a new file.
Git returned 1 without diagnostics; the corrected check accepts that ordinary
difference status and requires empty diagnostics. This was a checker error,
not a test or production failure. No heavy run was repeated for it.

The suite log includes source and fixture SHA-256 values. The separate source
manifest covers 14 source/generator inputs. Production bytes were compared
before and after the run. The unique temporary root
`/home/christian/.local/share/agent-tmp/mountd-round2-credential-path-q7e0j1qu`
was removed. Every credential case checked its recorded direct-child PIDs after
drain; the intentional failing assertions occur after that cleanup. No other
owner's scratch or process was touched. The shared Cargo cache remains as
permitted and is not a live fixture or process.

## Preserved objections and acceptance limits

Round 1's four fixes and fresh review dispositions remain accepted within their
recorded portable scope. This scoped review does not reopen or replace
R1-L1, R1-IPC-1/2 or R1-INSTALL-1. Historical P1-P7, CFG/IPC, E1-E6, RNI-1 through
RNI-4 and RF-1 retain their documented corrections and objections. In particular,
launchd presence cannot settle timed-out/signalled mutations, installer recovery
files must survive uncertainty, the final gate follows preparation, dependency
observations precede the network snapshot, and producers retain drain ownership.

Direct-child reap does not certify arbitrary descendants. Dependency commands
must stay trusted foreground predicates. Kernel/libc work can retain ownership
and block shutdown indefinitely; explicit SIGKILL, runtime destruction and OS
teardown defeat in-process retention. SMB passwords remain in helper argv until
exit. Safe preparation does not eliminate a later same-user rename or external
mount. Conservative Unicode comparison may reject distinct directories. A TCP
service gate does not certify NFSv3 ancillary RPC services.

Unrun native acceptance remains unchanged: disposable Keychain metadata, ACL,
encoding and total-prompt timing; the five ignored integration tests; native
helper topology and option semantics; APFS case/spelling/normalization and Mac
descriptor APIs; IOKit sleep/wake; Darwin peer authentication; launchd ordering,
exit policy and zsh locks; Swift/Foundation/AppKit, VoiceOver and UI recovery;
and separately authorized disposable real shares. No fake result closes these.
The parent ledger records the separate process/drain task as stopped after
provider refusal, "This content was flagged for possible cybersecurity risk."
That coverage remains unrun and is not taken over here. This credential/path
task continues only within its original independently assigned scope.
Provider-refused tasks remain stopped with no retry or alternate route.

The parent owns broader checks and integration. No production code, prior
fixture, ledger, branch, worktree, top-level thread, subagent, commit or push was
created or changed by this reviewer. Pre-existing edits and parallel-owner
artifacts were preserved.

Owned files:

- `docs/audit/round2-credential-path-breaker.md`
- `tests/audit/round2-credential-path.py`
- `tests/audit/round2-credential-path-cred.rs`
- `tests/audit/round2-credential-path-mount.rs`
- `tests/audit/round2-credential-path-suite.log`
- `tests/audit/round2-credential-path-source-sha256.txt`
- `tests/audit/round2-credential-path-checks.log`
