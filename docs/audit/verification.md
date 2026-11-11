# Integrated verification

This is the first audit batch's verification record. Follow
[extended audit rounds](swarm-rounds.md) for subsequent checks and publication.

Verified on Linux x86_64, 2026-11-12 Australia/Brisbane, against the hardening
sources based on `b39ea7ed42b418faa870f74f8edf282f60797fde`.
[Source hashes](results/source-sha256.txt) identify the checked source and
fixtures independently of later documentation commits. Run
`sha256sum -c docs/audit/results/source-sha256.txt` from the repository root.

All builds ran serially with:

```sh
export TMPDIR=/home/christian/.local/share/agent-tmp
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1
export CARGO_TARGET_DIR=/home/christian/Documents/dev/mountd/target/portable-daemon
```

| Command | Result | Retained output |
| --- | --- | --- |
| `python3 tests/portable-reconciler.py` | 47 engine, 25 reconcile, 5 state and 3 Ops tests passed; Clippy passed | [reconciler.log](results/reconciler.log) |
| `python3 tests/portable-process.py` | 103 passed, 5 real-Keychain tests ignored; Clippy passed | [process.log](results/process.log) |
| `python3 tests/portable-ipc-config.py` | 27 config, 10 IPC, 3 CLI wire and 1 actual CLI cleanup-drain test passed; Clippy passed | [ipc-config.log](results/ipc-config.log) |
| `python3 tests/portable-daemon.py` | Actual entry point with fake effects passed duplicate-start, responsive status, accepted IPC drain, SIGTERM and owned-child disappearance checks | [daemon.log](results/daemon.log) |
| `python3 tests/portable-native.py` | 16 unit tests, Clippy, build and synthetic launchd transaction checks passed | [native.log](results/native.log) |
| `python3 menubar/test-build.py` | Synthetic install/update rollback, XML, config preservation and recovery cases passed | [installer.log](results/installer.log) |
| `python3 docs/audit/results/negative-controls.py` | Passed: seven deliberately restored faults each failed their regression; the runner requires those seven failures | [negative-controls.log](results/negative-controls.log) |
| `python3 docs/audit/results/reproduce-signalled-client.py` | Reproduced the old lost-plist branch after removing only the signal guard from a temporary copy | [signal-negative.log](results/signal-negative.log) |
| `python3 docs/audit/results/reproduce-signalled-menu.py` | Reproduced premature file restoration and a second bootstrap after removing only the menu signal guards from a temporary copy | [menu-signal-negative.log](results/menu-signal-negative.log) |
| `cargo clippy --locked --target aarch64-apple-darwin --all-targets -- -D warnings` | Passed, with CARGO_TARGET_DIR unset for the repository's Apple cache | [apple-clippy.log](results/apple-clippy.log) |
| `cargo fmt --check` and `git diff --check` | Passed | Parent command output |
| `bash -n tests/verify-macos.sh` and `zsh -n menubar/build.sh menubar/test-command.sh` | Passed | Parent command output |
| Python AST parse of all fixture scripts | Passed | Parent command output |

These suites overlap. Their counts must not be added into a unique-test total.
Apple-target Clippy typechecks native bindings but does not link or execute Mac
binaries. The full Linux native build remains inapplicable because it cannot
link IOKit/CoreFoundation. No production platform behavior was replaced.

The new network regression initially used an array where endpoint `only_if`
expects a string; the fixture was corrected and rerun. Sharing a Cargo target
also exposed a collision between fixtures with the same package identity.
Each generator now uses a distinct package name. Reconciler, IPC and daemon
fixtures were rerun after that correction. The logs above contain the passing
runs, not the earlier failures.

## Negative controls

The retained [negative-control runner](results/negative-controls.py) derives its
isolated crate from the reconciler fixture, preserving the hard-failing
SystemOps boundary. It restores the reviewed faulty behavior only in temporary
copies: unowned credential work, preparation after the gate, pre-dependency
network snapshots, unregistered nested producers, NFS alias acceptance and
literal-only path comparison, and inspection that discards pending launchd
cleanup. It selects seven corresponding regressions and requires Cargo to
report exactly seven failures. Production files are unchanged.

The first control run exposed a weak network regression: its transition occurred
during initial evaluation, so the final gate masked the restored bug. The test
now waits at the existing fake credential gate and changes networks during the
final dependency check. All six controls then failed as expected; the corrected
reconciler suite and Apple-target Clippy were rerun successfully. After the
integrated review raised RF-1, a seventh control restored that launchd branch;
all seven failed as required. Native fixtures now defer server-side mutation
until after the client timeout, and an actual CLI test holds cleanup after an
install error. The affected native/CLI suites and Apple-target Clippy passed. The focused
review then identified a signal-terminated client variant. The prescribed Rust
status check and matching menu guards were added. Separate disposable controls
reproduce the prior failures; current native fresh/update and menu bootstrap/bootout
signal cases pass, as do the affected native, installer and Apple-target checks.

## Isolation

No real shares, production credentials, installed applications or LaunchAgents
were accessed or changed. Tests used owned temporary configs and synthetic
helpers. TCP/UDP fixture listeners bound explicitly to 127.0.0.1; IPC used
isolated Unix sockets. No task daemon or helper remains after verification.

The final listener inspection found no mountd service. Existing unrelated
public host services were left unchanged. Final rootless Docker inspection through
`unix:///run/user/1001/docker.sock` found no running containers. Earlier unrelated
containers published only on 127.0.0.1. No container, proxy or firewall change was made
for this audit. No home-machine SSH was attempted.

## Review and remaining acceptance

Two original reports identified ten findings. The integrated recheck accepted
those responses and raised RF-1, which is now corrected and tested. Responses
and the resolver-cap fix are recorded in [review-disposition.md](review-disposition.md).
The focused recheck confirmed the main RF-1 correction and identified the signal
variant documented above. Its small correction was reviewed and tested by the
parent. All recorded findings have a disposition; native acceptance remains
open. Implementation commits are listed in [the handoff](README.md). Nothing has
been pushed or published.

Swift compilation, Foundation child reaping, VoiceOver, APFS aliases, Darwin
peer authentication, IOKit sleep/wake, native launchd behavior, disposable
Keychain and real-share acceptance remain unverified. The exact commands,
expected results and artifacts are in [mac-verification.md](mac-verification.md).
The separate account/VM and disposable share/Keychain inputs remain unavailable.
