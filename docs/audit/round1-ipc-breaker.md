# Round 1 IPC breaker

Independent source review and new bounded Linux fixtures against `a66778e`.
The shared branch is `hardening-audit-rounds`. The checkout initially had exactly
that HEAD; the parent's documentation-only `b4f33bf` arrived during this run.
The IPC, config, CLI and menu sources used by these fixtures still match
`a66778e`. A concurrent edit to `src/engine.rs` appeared during final checks.
It is outside this breaker's ownership and is not exercised by the fake engine;
no review or test acceptance of that new edit is claimed here.

Only `tests/audit/round1-ipc*` and this report belong to this breaker. No production
edits, commits, pushes, branch changes, worktrees or subagents were made.

## Confirmed findings

### R1-IPC-1, medium: menu treats a mount name as a CLI option

Config accepts `--help`, `-h`, `--` and `-archive` as mount names at
`src/config.rs:552`. The menu passes `["mount", s.name]` and
`["unmount", s.name]` at `menubar/Sources/main.swift:227` and `:232`.
Clap parses those names as syntax rather than positional data.

The new `menu_argument_shape_addresses_every_accepted_name` regression invokes
the actual `src/bin/mountctl.rs` with the menu's argument shape and an owned fake
Unix server. All eight action/name combinations send zero requests. Both actions
with `--help` or `-h` exit 0 and print help; the other four combinations exit 2.
Expected: every accepted name reaches the intended IPC command unchanged.
Actual: neither mount nor unmount is dispatched. The help cases also appear
successful to `runCommand`, whose exit-zero branch returns no error. The menu
ignores stdout in `action` at `main.swift:148`, so these no-ops produce no alert.
This is an argv parsing defect established on Linux, not a claim to have run AppKit.

Smallest fix: insert the option terminator in both menu call sites,
`["mount", "--", s.name]` and `["unmount", "--", s.name]`. Preserve ordinary CLI
help behavior. The separate `option_terminator_preserves_config_names` control
already dispatches seven names exactly, including all four failing names,
a quote and Japanese text. The regression extracts either original or corrected
menu call shape; it fails explicitly if the call is refactored beyond those forms.

### R1-IPC-2, low: config can accept a name that the control transport cannot send

The new `config_acceptance_implies_transportable_name` regression supplies a
synthetic config whose name contains 32,768 quote characters. Config validation
succeeds and a real IPC status round trip returns the exact name. An unmount
request for that same name serializes to 65,564 bytes including its newline,
exceeding the 65,536-byte request cap. `client_request` returns
`IPC request too large`; the fake engine records zero mutations.

Expected: config either rejects an unaddressable name or every name it accepts
can be used by the control protocol. Actual: status advertises a valid configured
mount that mount/unmount/named-explain requests cannot address. The same escaped
name fits comfortably in the 4 MiB response budget, so successful status does not
establish control usability. This is a low-severity validation defect requiring
an unusually large name, not a remote attack or measured memory-exhaustion issue.

Root cause: name validation at `src/config.rs:552` only checks emptiness and
control characters. It has no bound corresponding to the client check at
`src/ipc.rs:308` or server frame limit at `:139`. Counting unescaped characters
alone also misses JSON expansion.

Smallest fix: reject names at config load when their JSON-encoded bytes, the
largest named-command envelope and the newline would exceed the shared request
limit. Keep that budget consistent across mount, unmount and explain. The
regression permits early config rejection, so it can pass after this validation
fix without allowing oversized IPC frames.

## New fixture boundaries

`round1-ipc-breaker.py` reuses the fake boundaries from
`tests/portable-ipc-config.py`. A distinct package,
`mountd-round1-ipc-breaker-fixture`, prevents the documented cache collision.
It copies `ipc.rs` only to attach the new unit module and compiles the actual CLI.
Config, IPC framing/admission, client, serialization and CLI parsing stay real.
Fake mutation methods only increment counters, await a semaphore, and increment
completion counters. The status fixture can pad one field to exercise response
limits. Mounts, native mount enumeration, credentials and probes remain forbidden.

The new mutation tests establish transport ownership around a deliberately held
engine future. They do not establish native mount completion, launchd ordering,
engine lease behavior or direct-child reap. Those are separate existing contracts.
The `/proc/self/fd` measurements are Linux-only. Test sockets and configs are
owned temporary files; no TCP listener, public service, Docker, installed app,
LaunchAgent or production credential is involved.

## Commands and retained evidence

All repository commands used `/home/christian/Documents/dev/mountd` as their
explicit working directory. Both suite invocations used this exact shell command:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 600 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout 500s python3 tests/audit/round1-ipc-breaker.py \
  > tests/audit/round1-ipc-runner.log 2>&1
```

The runner sets `CARGO_TARGET_DIR` to
`/home/christian/Documents/dev/mountd/target/portable-daemon`. It invokes these
serially, each with a 240-second subprocess deadline, using an owned temporary
manifest whose exact path is recorded in each log:

```sh
cargo test --offline --locked --manifest-path "$fixture/Cargo.toml" --lib round1_ipc -- --nocapture
cargo test --offline --locked --manifest-path "$fixture/Cargo.toml" --test round1_ipc_cli -- --nocapture
```

The lock waits ran in yielded command sessions. No build ran outside that lock.
The runner deliberately returns nonzero for failing regression assertions.

The initial unit run had a fixture error. Rust temporary directories inherited
permissions that failed the production private-directory check. Eight tests
stopped at socket setup; the direct socket-pair writer test passed. The initial
CLI run independently reproduced R1-IPC-1. The fixtures now explicitly set their
temporary directories to `0700`. Initial output is preserved in
`tests/audit/round1-ipc-initial-{unit,cli,runner}.log`; it is not product-failure
evidence except for the independently completed CLI reproduction.

Final result: 11 new tests, 9 passed and 2 failed on the confirmed regressions.
No existing tests were selected. The unit invocation reports 8 passed, 1 failed
and 105 unrelated tests filtered out in 21.74 seconds. The CLI invocation reports
1 passed and 1 failed in 0.94 seconds. Both Cargo commands exit 101; the runner
exits 1. Compilation succeeded for both.

| Hypothesis | New measured result | Disposition |
| --- | --- | --- |
| Completed connections retain admission slots or fds | 1,024 requests in 268 ms; fd count 12 before and 12 after | Rejected for this bounded churn |
| Saturation keeps growing or never recovers | 64 gated requests admitted, 32 excess requests rejected, 64 completions; fds 12 to 140 to 12; subsequent status succeeds | Rejected |
| Disconnect, half-close or nonreading peer cancels accepted mutation or releases singleton early | Mount/unmount/reload crossed with three peer modes; 9 completions, 0 premature unlocks; shutdown waits for each gate | Rejected at the IPC/fake-engine boundary |
| EOF permits partial mutation frames | All 32 proper prefixes of the request rejected; 0 mutations | Rejected |
| Idle saturation pins shutdown forever | 64 accepted idle peers; lock held during wait; shutdown takes 10,000 ms; fds 12 before and 10 after server cleanup | Rejected |
| A stalled response writer never expires | 1 MiB payload, requested 4 KiB send buffer, no reader; write deadline returns after 10,123 ms | Rejected |
| Request/response framing disagrees about newline or cap | 17 frame-boundary cases pass; 98,304-byte and 4,194,303-byte JSON responses succeed; 4,194,304 and 4,194,305 return explicit oversize error | Rejected for framing; config acceptance defect remains separate |
| Escaped/Unicode names change in JSON transport | 8 names preserved through status and named request; each mutation counter reaches one | Rejected |
| Menu names remain data at the CLI boundary | 8 calls send no request, including 4 exit-zero help no-ops; 7 calls with `--` dispatch correctly | Confirmed R1-IPC-1 |
| Every accepted name fits a named IPC request | 32,768-byte name accepted and listed; 65,564-byte request rejected before engine entry | Confirmed R1-IPC-2 |

Retained final logs are `tests/audit/round1-ipc-unit.log`,
`tests/audit/round1-ipc-cli.log` and `tests/audit/round1-ipc-runner.log`.
The 11 test count is the unique new test count. Matrix rows contain subcases,
which must not be added to it. No native result is inferred from these numbers.

## Parent integration artifacts

All paths below are relative to `/home/christian/Documents/dev/mountd`:

- `tests/audit/round1-ipc-breaker.py`, generator and bounded runner.
- `tests/audit/round1-ipc-unit.rs`, nine new IPC/config tests.
- `tests/audit/round1-ipc-cli.rs`, two new actual-CLI tests.
- `docs/audit/round1-ipc-breaker.md`, this report.
- `tests/audit/round1-ipc.patch`, add-file patch for those four source/report files.
- `tests/audit/round1-ipc-source-sha256.txt`, checked source and fixture hashes.
- `tests/audit/round1-ipc-checks.log`, formatting, Python parse, patch whitespace,
  source identity and owned-process cleanup checks.

The patch adds tests/report only. The logs and hash manifest remain available
alongside it in this shared checkout. Production fixes remain unimplemented.
The direct writer test checks `serve_conn`'s timeout; the idle and mutation tests
separately exercise `serve_bound_until` shutdown. This suite does not claim an
integrated saturated large-response shutdown test or general heap/RSS bound.

## Source review and remaining limits

Read coverage, review-disposition, verification and mac-verification, all four
original review reports, the IPC/config ledger, relevant daemon/engine lifecycle
paths and both portable IPC/daemon generators. No additional source claim is made
about RF-1. The latest signal guards and the accepted direct-child, stuck
kernel/libc, SMB argv and post-preparation pathname limits remain accepted.

Applied ponytail, unslop and realistic-commits under the explicit no-commit
instruction. Initially the branch had no remote-tracking ref, and an authenticated
read-only GitHub event lookup returned no matching branch PushEvent. Rechecking
after the parent's publication found the reliable `update by push` reflog entry
for `b4f33bf` at `2026-11-12 08:50:58 +1000`. No commit timing action is applicable.

Mac runtime acceptance remains pending the user. This round does not run Swift,
AppKit, Darwin peer credentials, Keychain, real shares, LaunchAgents or a full
repository test/build suite. The parent owns integration, full checks and fresh
independent review. Production fixes are recommendations only.
