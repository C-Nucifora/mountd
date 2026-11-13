# Round 1 control-name corrections

Parent reviewed the fixes for R1-IPC-1 and R1-IPC-2 before checkpointing them.
The independent breaker's original report and failing results are retained.
Trailing line whitespace in archived logs was normalized for Git checks.
The user's checkpoint/pause request deferred a fresh independent post-fix review.

Menu mount and unmount actions now insert `--` before the configured name.
Names such as `--help`, `-h` and `--` reach IPC as data; ordinary command-line
help remains available. The actual CLI fixture extracts the menu argument shape
and verifies the dispatch. This does not execute AppKit.

Config and IPC now share the 64 KiB request cap. After rejecting control
characters, validation counts name bytes plus the extra JSON escaping for quotes
and backslashes. It includes the longest named-command envelope and newline.
Oversized names fail validation without embedding the enormous name in the
error. No transport bound was increased. Six boundary cases compare validation
with actual serialization of mount, unmount and named explain, including exact
fits, one-byte excess, escaped characters and multibyte Unicode.

## Checks

- New IPC suite: 10 unit tests and two actual-CLI tests pass. This includes the
  breaker's 11 tests plus the six-case encoded-boundary test.
- Full portable suite: 27 config tests, 10 IPC tests, three CLI wire tests,
  actual CLI cleanup-drain and all-target fixture Clippy pass.
- Formatting and whitespace checks pass.

Logs are `tests/audit/round1-ipc-unit-fixed.log`, `round1-ipc-cli-fixed.log`,
`round1-ipc-fixed-runner.log` and `round1-ipc-full.log`. The earlier logs without
the `fixed` suffix retain the original failure evidence. Initial setup failures
are labelled as fixture errors in the breaker report, not product findings.

Run the focused checks with `MOUNTD_AUDIT_LOG_SUFFIX=-checkpoint` to preserve
earlier logs, then run `python3 tests/portable-ipc-config.py`. Each suite must
take the shared heavy lock separately, with disk TMPDIR, one Cargo job and one
test thread. Both commands already ran under those controls. The focused runner
now derives the repository path from its location and passes it to the CLI test
at compile time; the fd-count cases remain explicitly Linux evidence.

Swift execution, Darwin socket authentication and native acceptance remain open.
The lifecycle and installer changes have separate evidence and are not accepted
by these fake-engine tests. The multi-round audit is unfinished at this checkpoint.
