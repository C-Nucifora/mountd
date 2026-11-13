# Round 1 independent IPC review

Fresh challenge of R1-IPC-1 and R1-IPC-2 at published checkpoint
`0be609a323c8e957bf8e4c20dc55e06d5e2bba07`, which contains fix
`2c01b863bdc24da5a94241c8b7694a3eb11574c2`. Verified branch
`hardening-audit-rounds`, origin `https://github.com/C-Nucifora/mountd.git`, both
ancestry checks and clean production paths before work. The pre-existing
`docs/audit/swarm-rounds.md` edit belongs to the parent and was preserved.

No new defect was confirmed. Both fixes withstand the three new portable
regressions below. Both negative controls remain unrun after their 600-second
lock-acquisition waits expired. This is bounded Linux evidence; native acceptance
and the broader audit remain open. No production correction is recommended.

Read repository/ancestor instructions, README, coverage, review-disposition,
round1-ipc-breaker, round1-ipc-fix, round1-installer-fix, mac-verification and the
original native/engine, integrated and RF-1 follow-up reviews. Applied ponytail,
unslop and realistic-commits under the explicit no-commit constraint. The branch
tracking reflog records its latest push at `2026-11-12 09:31:52 +1000`, checkpoint
`0be609a`. A read-only GitHub event query returned no matching branch event.
No commit timing action applies.

## New challenge and oracle

The new fixture derives the fake boundaries from `tests/portable-ipc-config.py`
and the gated-mutation approach from `tests/audit/round1-ipc-breaker.py` without
editing either. Each invocation copies production modules, the actual control
CLI and the Swift menu source into its own disposable directory. The Cargo
package is `mountd-round1-ipc-independent-review-fixture`. No native daemon or
SystemOps is built. Fake effects check the exact configured name, count entry
and completion, and optionally wait on a semaphore. Explain returns an empty
synthetic explanation list. These are IPC tests, not engine lease tests.

The oracle comes from README's named CLI commands and newline JSON protocol,
config's explicit nonempty/control policy, the shared 65,536-byte request cap,
and `serve_bound_until`'s accepted-request drain and singleton contract.

- Eight fixed generator seeds mix ASCII, quotes, backslashes, Japanese, emoji,
  composed/decomposed accents, U+2028/U+2029, ZWJ, BOM and U+10FFFF. For each seed,
  test encoded budgets one byte below, exactly at and one byte above the longest
  request limit. Compare config acceptance to actual serialization of all three
  named request variants. Accepted names must survive real status and named
  request round trips, with exact name checks inside the fake engine.
- Reject all 65 C0/C1 controls and six empty/whitespace-only inputs. Accept seven
  existing policy edge cases, including `--`, `-`, option-looking names, ZWJ,
  BOM and surrounding spaces. Format characters being accepted is existing
  policy, not a new UI usability guarantee.
- Extract both menu argv shapes and invoke the actual CLI for 13 names crossed
  with mount, unmount and named explain. Eleven short cases include `--`, `-`,
  `--help=archive`, `--json`, `-hV`, `---`, `--version`, `--config=/synthetic`,
  quotes/backslashes/Unicode with spaces, ZWJ and BOM. Two additional generated
  names are just below and exactly at the encoded limit. Also test an oversized
  CLI request and ordinary help without a daemon. Empty/control config names
  are rejected at config load; no extra CLI-level prohibition is inferred.
- Fill all 64 admission slots with two nonreading 3 MiB status peers and 62
  accepted mutations whose real client futures are cancelled. Reject four excess
  requests, then request shutdown. The listener must close, accepted mutations
  must complete after release, and the singleton must remain held through the
  blocked writers' actual ten-second deadlines. A replacement server must bind
  afterward and answer poke. This combines cases previously tested separately.

The two negative controls change only disposable copies. One disables only the
config length guard. The other removes `--` from both Swift call shapes. Each is intended
to fail the new regression for the known defect. Neither executed because the
shared lock remained unavailable for the full acquisition bound. The supplied
control fixtures therefore remain an unverified check, not failure evidence.

## Commands and results

Every repository command uses `/home/christian/Documents/dev/mountd` as its
explicit working directory. Each suite takes and releases the shared lock
separately, with no extra workers. Run each row with this command, substituting
`ARGS` and `LABEL` from the result table:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout 235s python3 -u tests/audit/round1-ipc-review.py ARGS \
  > tests/audit/round1-ipc-review-LABEL.log 2>&1
```

The runner uses `target/portable-daemon` only under that lock, gives Cargo a
220-second deadline, sets `TOKIO_WORKER_THREADS=1` for CLI runs, kills and reaps
its own child process group on an exception, and removes only its own fixture
directory. Each executed-suite log records source SHA-256s,
the exact Cargo command and disposable manifest path, exit status and elapsed
build/test time. Runtime fixtures use private temporary HOME directories and
Unix sockets. No TCP listener, credentials, real mount, installed application,
LaunchAgent or live service is involved.

| ARGS | LABEL | Observed result | Build/test elapsed |
| --- | --- | --- | --- |
| `names` | `names` | Pass, one new test; 24 boundaries, 16 status and 48 named round trips; policy checks pass | 12.20 s |
| `cli` | `cli` | Pass, one new test; 39 exact-name dispatches, three help calls, one oversize refusal | 7.88 s |
| `transport` | `transport` | Pass, one new test; 62 completions, four excess refusals, singleton retained through 9,978 ms shutdown drain, replacement poke succeeds | 20.48 s |
| `names --negative config` | `negative-config` | Unrun; lock acquisition expired, shell exit 1 before Cargo | No execution |
| `cli --negative menu` | `negative-menu` | Unrun; lock acquisition expired, shell exit 1 before Cargo | No execution |

The two unrun logs contain labelled reviewer annotations of the empty command
output and observed shell exit, not fabricated Cargo output. Each wait ended
within the requested bound; neither was retried.

The completed tests reject these hypotheses for the selected inputs: JSON
escaping or envelope accounting admits an oversized name; option-looking names
lose their literal value through either menu argv shape; cancelled clients
release accepted effects; saturation or stalled writers release the singleton
early or prevent bounded transport shutdown. The three test functions are the
unique new-test count. Matrix subcases are not additional tests.

## Preserved objections and limits

This review does not change the RF-1, E1-E6 or RNI-1 through RNI-4 dispositions.
RF-1's typed uncertainty and signal guards still require native launchd ordering
and recovery evidence. RNI-2 still needs native exit-policy acceptance; RNI-4
needs IOKit delivery, cancelled-sleep and timing checks. E1/E3/E4/E5 ownership and
fresh-gate contracts are not exercised by this fake engine. E2 needs native NFS
option acceptance; E6 needs case-sensitive and insensitive APFS spelling and
normalization checks. R1 installer locks still need native zsh/APFS tests and
separate fresh installer review; IPC success accepts none of that transaction.

Direct-child reap cannot certify arbitrary descendants. Trusted dependency
commands must stay foreground. Kernel-stuck helpers or libc work can retain
ownership and block shutdown indefinitely; SIGKILL, runtime destruction and OS
teardown defeat in-process retention. SMB secrets remain in argv until helper
exit. A same-user rename or external mount can race after safe preparation.
The configured TCP probe does not certify ancillary NFSv3 RPC services.

Darwin peer credentials and cross-user refusal, native helper topology and
linking, Keychain, real shares, Swift/Foundation/AppKit, VoiceOver and keyboard
focus remain unrun. Swift's command deadline starts after executable lookup and
launch, so this review supplies no bound on those calls. Reveal still depends
on completion or app restart if NSWorkspace blocks permanently. The retained
uninstall behavior after plist-removal failure is not a new rollback guarantee.
No general heap/RSS bound or native menu acceptance follows from these tests.
Provider-refused work remains stopped; no retry or alternate route was used.

## Handoff

Completed: fresh source review, the three new positive regressions, Python AST
parse, Rust formatting, source-hash/checkpoint comparisons and owned fixture
cleanup checks. The five queued commands have ended. Three executed suites
returned 0; two lock waits returned 1 before execution.

Unrun: both negative controls due to shared-lock contention; the full existing
portable suites and Clippy/Apple-target checks; all native acceptance checks.
Retained source hashes and detailed final checks are in the files below. The
parent owns any later control execution and broader integration acceptance.

Files owned by this review:

```text
docs/audit/round1-ipc-review.md
tests/audit/round1-ipc-review.py
tests/audit/round1-ipc-review-common.rs
tests/audit/round1-ipc-review-unit.rs
tests/audit/round1-ipc-review-cli.rs
tests/audit/round1-ipc-review-names.log
tests/audit/round1-ipc-review-cli.log
tests/audit/round1-ipc-review-transport.log
tests/audit/round1-ipc-review-negative-config.log
tests/audit/round1-ipc-review-negative-menu.log
tests/audit/round1-ipc-review-checks.log
tests/audit/round1-ipc-review-source-sha256.txt
```

Only this report and `tests/audit/round1-ipc-review*` belong to this reviewer.
No production edits, commits, pushes, worktrees, branches, threads or child
agents were created. Parent retains ledger and integration ownership.
