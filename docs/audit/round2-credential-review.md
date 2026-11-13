# R2-CRED-1 independent parser review

Independent parser review accepts the correction within the exercised scope.
All six new pure-parser tests passed, covering 427 matrix cases and 24 negative
control comparisons. No substantive unresolved parser finding remains in this
review. Native metadata compatibility remains open.

## Source and scope

Reviewed `/home/christian/Documents/dev/mountd`, origin
`https://github.com/C-Nucifora/mountd.git`, branch `hardening-audit-rounds`, HEAD
`171eab5b7d8d8d183f2cb2cb39df745a5f32631f`. The fixed, uncommitted
`src/cred.rs` SHA-256 is
`460fdeedc8bd07c5cc452f726e1bd6bab23de718dac98d234f45b2e459fe049d`.
The unrelated default course-planner directory was not used for repository
operations. The branch's local tracking reflog records its last push at
`2026-11-12 10:30:07 +1000`. No unpublished commits were present at review start.
No hosting query was made because this review forbids network access.

Read the supplied instructions and checked ancestor and repository AGENTS.md
paths; no additional file was found. Applied ponytail, unslop and
realistic-commits. The explicit no-commit instruction overrides commit creation
and scheduling. Read README credential semantics, process P4,
review-disposition, round2-credential-path-breaker, round2-credential-fix,
swarm-rounds and mac-verification. No branch, worktree, thread, subagent,
production edit, commit or push was created.

Only the parser correction and its immediate interpretation were challenged.
The parent owns the retained seven-case breaker rerun, portable-process suite
and Apple-target Clippy. Those checks are not duplicated or claimed here.

## Disposition and source reasoning

R2-CRED-1 remains a valid medium finding against the old parser. The original
seven-test breaker reported five passes and two failures. Successful synthetic
`-w` hex followed by `-g` containing a lone quote returned `Found`. Conflicting
literal and encoded records selected the first record, in either order. These
violated P4's existing malformed/ambiguous fail-closed requirement, without
establishing that native security emits these bytes.

The fix checks for a second recognized record before interpreting the first.
That covers identical duplicates, conflicting records, malformed records in
either position and a third record separated by unrelated lines. Counting a
second match suffices to reject any larger count. Missing records still fail.
Recognition remains a line whose leading whitespace is removed before the
exact `password:` prefix; unrelated lines do not become records merely because
they contain that word elsewhere.

Removing the opening quote before looking for a closing quote requires two
distinct characters. A single quote fails even with surrounding whitespace;
two quotes remain an allowed empty literal indicator. The parser returns
`None` for literal metadata, leaving the caller's original hex-looking password
intact. An encoded value passes the unchanged even-length check, hex decoder
and UTF-8 validation. The actual empty first response still becomes Missing in
the caller; accepting an empty literal indicator does not change that rule.

The production diff changes only record selection, quote recognition and two
source tests. The parser's errors are static string literals. Its only
production caller maps them to Failed without interpolating stderr. Both hex
helpers are byte-identical to HEAD. Deadline, lease, subprocess and platform
code are unchanged. This is source evidence, not an independent runtime check
of those callers.

The fixer's two source regressions cover the original failures and neighboring
forms. Its runtime attempts did not acquire the shared lock within 900 seconds;
formatting passed. Parent source acceptance and queued broader checks do not
constitute completed native or runtime results in this report.

## Independent matrix and negative control

The new fixture has six test functions. Five contain 427 parser cases:

| Challenge | Cases | Oracle |
| --- | ---: | --- |
| All sequences of zero through three records over literal, encoded, lone-quote, empty and invalid-UTF-8 forms, with LF and CRLF layouts | 312 | Zero is missing, one is interpreted or rejected, every larger count is ambiguous |
| Every truncation of one complete quoted value, with three padding forms, plus missing or displaced boundaries | 30 | Only a complete pair of distinct boundaries succeeds |
| Seven unrelated lines with a valid record inserted at every position | 9 | Noise alone is missing; one recognized record remains literal |
| Accented text, CJK, emoji, tab, repeated newline and mixed values; uppercase/lowercase encoded hex, optional existing display suffix and line endings; literal-hex indicators | 54 | Encoded UTF-8 survives exactly; literal hex returns None |
| Odd/non-hex values, invalid UTF-8, truncated sequences, overlong encoding, surrogate and out-of-range encoding, with/without a synthetic marker suffix | 22 | Exact static `invalid password hex` error |

These combinations extend the fixer's tests with exhaustive short record
sequences, record placement and truncation boundaries. Counts describe matrix
rows, not separate Rust test functions or native formats. Assertions follow
P4, README and existing supported forms; they do not define an exhaustive
macOS grammar.

The sixth test compares the same rejection oracles against a disposable old
parser. It restores only first-record selection and the old starts/ends quote
predicate, then verifies that body byte-for-byte against HEAD's parser. The
control requires four padded lone-quote cases and twenty two-record cases to
violate the oracle while the fixed parser satisfies it. This is an explicit
comparison test, not a claim that a separate test process exited nonzero.

The generator extracts unchanged complete `looks_like_hex`, `decode_hex` and
`parse_password_metadata` function bytes. It verifies the full production hash
before extraction, logs individual function, fixture, generator and generated
source hashes, and verifies source identity after execution. One Rust test
binary contains both the exact fixed functions and isolated old parser. It has
no dependencies, Cargo build, async runtime, helper programs, native API calls,
network or credential access. Only synthetic strings enter the parsers.
Generated source and binary remain in its unique owned scratch directory;
there is no active scratch deletion.

## Commands and execution evidence

Each acquisition uses the same bounded command:

```sh
cd /home/christian/Documents/dev/mountd
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 1800 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 175s python3 -u tests/audit/round2-credential-review.py \
  > tests/audit/round2-credential-review-suite.log 2>&1
review_status=$?
printf 'outer_exit=%s\n' "$review_status" >> tests/audit/round2-credential-review-suite.log
exit "$review_status"
```

Under that single lock the runner invokes `rustc --edition=2021 --test` on the
generated source, then the resulting binary with `--test-threads=1 --nocapture`.
Each subprocess has a 60-second timeout. The outer execution limit is 175
seconds with five seconds of termination grace, at most 180 seconds. The one
suite releases the lock on completion. Exact paths and argv are logged.

The first acquired attempt exited 1 before compilation. The control generator
restored both old decisions but retained a newline before the opening brace;
its byte-identity assertion correctly rejected the mismatch against HEAD.
This was a fixture error, not a product failure. The original output is retained
in `tests/audit/round2-credential-review-preflight.log`. No scratch directory,
compiler or test binary was created by that attempt. Its generator SHA-256 was
`767fa2e7de3ed73c3bc26296482b9269c29553f4179cd93764ebccfaea0012fd`.

Corrected only the control text replacement to include the opening brace, then
verified the extracted functions and restored HEAD parser byte-for-byte in a
lightweight source-only check. The corrected fixture used a separate bounded
acquisition; neither attempt expired its bound. No acquisition was retried after
a lock timeout. Only the corrected attempt compiled and ran tests.

The corrected run passed all six tests, with zero failures, ignored tests or
filtered tests. All 427 matrix cases passed. The old parser violated all 24
selected rejection oracles while the fixed parser satisfied them. These results
reject the tested hypotheses that later or malformed duplicates escape counting,
padding makes a truncated literal valid, unrelated lines hide or create records,
valid encoded UTF-8/newlines change meaning, or invalid hex produces success or
input-bearing diagnostics. They do not settle the native grammar objections.

Rust compilation, the test binary and the outer command each exited 0. The
test binary reported 0.01 seconds; the runner reported 0.445 seconds after lock
acquisition, including source extraction and compilation. Source identity
passed after execution and again at handoff. Branch and HEAD remained as stated
above. The shared lock was released and the command completed; no test child
remains. No further heavy checks were started.

Completed lightweight checks: Python AST parsing, `rustfmt --edition 2021
--check tests/audit/round2-credential-review.rs`, owned-file whitespace checks,
`git diff --check -- src/cred.rs`, and extraction/control byte identity. Broader
Cargo, portable-process and Apple-target checks belong to the parent and were
not run by this reviewer. Neither the two new production tests nor the five
ignored native credential tests were executed by this extracted-function suite.

The final generator SHA-256 is
`905dcab262299a2d8da4fb59a33ffb60904c57a8209a2376143d32a079a62792`;
the Rust fixture SHA-256 is
`a52dcb0294841ef6041f392380cd11a1efff360de7d215dc5d2d24c294c206db`.
The generated Rust SHA-256 is
`17f827c6017d5db0a8eec5fcf62ebae3e24524e214b39b4bb0abe420733dfe6d`.
The complete final log records each extracted function hash, the old-parser
hash, exact compiler/test argv and all results. The inactive generated source
and binary are retained at
`/home/christian/.local/share/agent-tmp/mountd-round2-credential-review-dzfxom4x`.

## Preserved objections and limits

No further substantive parser finding is confirmed. The following native
grammar objections remain unresolved, as recorded by the fixer: embedded
literal quotes and escaping, optional display text after encoded hex, and
whether two successful reads should be compared for consistency. The parser
still tests only literal boundaries and only the first hex token. It does not
validate the entire display suffix. Rejecting an invented suffix or escape
grammar would require native evidence; the mock matrix cannot supply it.
Lossy stderr conversion and exact record recognition are existing caller/parser
behavior, not a validated specification of every native byte sequence.

The five ignored Keychain tests remain unrun. Native metadata, ACL, prompts,
encoding and caller-budget behavior still require the separately authorized
disposable-Mac checks. No successful synthetic case establishes native output
grammar or compatibility.

Prior P1-P7, E1-E6, RNI-1 through RNI-4, RF-1 and round-1 dispositions are
preserved. E1's shared deadline and retained credential ownership remain
requirements. RF-1's uncertain launchd outcomes cannot be settled from presence
alone. The reviewed correction does not alter preparation/final-gate ordering,
dependency/network observation ordering, producer ownership or installer
recovery. This review neither reopens nor independently re-verifies those fixes.

SMB argv exposure, direct-child versus arbitrary-descendant limits,
kernel/libc retention, shutdown limits under SIGKILL/runtime teardown, pathname
races, conservative Unicode rejection and NFS ancillary-service limits remain.
APFS, Darwin credentials and helper options, IOKit, launchd/zsh, Swift/UI and
real-share acceptance remain open under the existing Mac checklist.

The separate process/drain task was provider-refused with "This content was
flagged for possible cybersecurity risk." It remains stopped. No
`tests/audit/round2-process*` content was inspected, executed, modified or
adapted, and no refused supervisor/resolver work was taken over. No provider or
model route changed. No new portion of this parser review was refused.
Round 2 remains incomplete and cannot count as a clean round.

## Owned files

- `docs/audit/round2-credential-review.md`
- `tests/audit/round2-credential-review.py`
- `tests/audit/round2-credential-review.rs`
- `tests/audit/round2-credential-review-suite.log`
- `tests/audit/round2-credential-review-preflight.log`

All pre-existing production and other-owner changes are preserved. The parent
still owns integration and publication.
