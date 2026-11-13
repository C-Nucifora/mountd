# Process, credential, probe and mount safety audit

This is the implementation owner's historical handoff. Current integrated
results are in [verification.md](verification.md); review corrections are in
[review-disposition.md](review-disposition.md). Earlier pending statuses below
record the resource pause, not the final verification state.

Scope: `src/mountcmd.rs`, `src/cred.rs`, `src/mtab.rs`, `src/probe.rs`, `src/deps.rs`, `src/fingerprint.rs`, `src/watcher.rs`, and `tests/portable-process.py`. Baseline `b39ea7e`. No commits, pushes, remotes, installed apps, LaunchAgents, real shares, or production credentials changed. The tracking reflog has no reliable last-push record; the parent supplied the same hosting-event result. The no-commit instruction overrides commit scheduling.

## Verification status

Source implementation and regression fixtures are ready for parent integration. **The final regression suite has not been run successfully.** The resumed portable build was stopped during dependency compilation when the parent reported severe CPU/RAM pressure. Only that fixture's Cargo process tree was terminated. There are no remaining processes from that build. Its earlier temporary files were not moved or manually deleted.

Completed lightweight checks:

- `rustfmt --edition 2021 --check src/mountcmd.rs src/cred.rs src/deps.rs src/probe.rs src/fingerprint.rs src/watcher.rs src/mtab.rs`
- `git diff --check -- src/mountcmd.rs src/cred.rs src/deps.rs src/probe.rs src/fingerprint.rs src/watcher.rs src/mtab.rs tests/portable-process.py docs/audit/process.md`
- Python `ast.parse` of `tests/portable-process.py`.

Parent reported an Apple all-target compilation error from two tests referencing the removed redactor. Both tests were replaced with `command_failure_omits_argv_and_all_helper_output`; the corrected revision still needs compilation and execution. No runtime reproduction or test-pass claim is made for the resumed changes below. Each entry gives the concrete reproduction fixture and expected assertion for the serialized run.

The reproducible portable generator copies unchanged modules into an isolated temporary crate. Its sole stub supplies the watcher's engine notification field. It does not emulate macOS mount enumeration, credentials, or mount syscalls. It preserves ignored disposable-Keychain tests and binds TCP/WOL fixtures only to loopback. It sets one build job, disables incremental builds, runs tests serially, and puts temporary files on disk.

Parent's next portable check:

```sh
TMPDIR=/home/christian/.local/share/agent-tmp CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 python3 tests/portable-process.py
```

It runs `cargo test --locked --offline --lib` and `cargo clippy --locked --offline --all-targets -- -D warnings`. Run the full native suite and Apple target compilation separately through the parent. The portable suite cannot establish Darwin ABI or system-tool behavior.

## P1: operation guards could drop before owned child reaping

Severity: high. Reproduction: run a fake shell that ignores TERM, writes its PID, and stays alive; cancel its caller or expire its deadline. In the baseline, dropping `timeout(Command::output())` with `kill_on_drop` requested termination without proving reaping before engine guard release.

Expected: cancellation kills the child and explicitly waits. If the kernel delays reaping, the caller returns promptly while the mount and lifecycle guards remain held. No subsequent effect may use the same mount until cleanup finishes.

Fix: `OperationLease` carries the engine's shared owned guards into a supervisor. Caller cancellation, output failure, or deadline sends SIGKILL to the command's dedicated process group and explicitly waits for its direct child. After 100 ms of pending reaping, the caller receives typed `CleanupPending`; the supervisor retains the lease and continues waiting. Ordinary timeout errors are returned only after reaping. The unmount ladder stops on `CleanupPending`. The engine owner confirmed every mount path tries one selected endpoint and returns after errors; no same-operation fallback can overlap pending cleanup.

Verification fixtures: `cancellation_reaps_term_resistant_child_before_lease_release`, `timeout_kills_and_reaps_term_resistant_child`, and `stalled_reap_reports_promptly_but_retains_lease`. The last uses a synthetic reap gate, not an actual uninterruptible kernel process. Parent owns the engine tests that retain a fake lease through cancellation and block reload/sleep until release.

Mac gap: run these fake-process tests natively; verify process-group isolation, ESRCH after direct-child reap, and guard behavior under native engine cancellation. A real uninterruptible child cannot be safely manufactured in this audit. The synthetic gate is evidence for ownership logic, not for kernel scheduling.

## P2: output collection could exhaust memory or wait on descendants

Severity: high. Reproduction: a dependency writes indefinitely to stdout, or a shell exits after starting a pipe-inheriting descendant. Baseline `.output()` accumulated unlimited bytes and included descendant-held EOF waits without explicit reaping ownership.

Expected: bound both streams and the whole command, including pipe collection. Do not leave the direct child or its normal process-group descendants running after cancellation.

Fix: shared `command_output` caps each stream at 64 KiB and includes pipe reads in the deadline. It retains the unreaped leader PID while reading, so a timeout can safely signal only the dedicated process group without PID reuse. It then reaps the direct child. Dependencies, network queries, both Keychain subprocesses, and mount/unmount use this boundary. The native owner can use the same boundary for launchd commands.

Verification fixtures: `output_flood_is_rejected_and_child_reaped`, `inherited_pipes_cannot_hold_command_open`, and dependency cancellation tests. The inherited-pipe fixture uses a short-lived synthetic sleep process in the owned group, not a network service.

Mac gap: confirm native process-group and pipe behavior with fake scripts. Dependencies are trusted executable configuration. A deliberately daemonizing dependency can escape its group with `setsid`; this supervisor is not a sandbox or a process-tree containment mechanism. It only reaps direct children; the OS adopts orphaned descendants. Native NFS backgrounding is separately prohibited by option validation.

## P3: preparation could follow symlinks and outlive cancellation untracked

Severity: high. Reproduction: point a mountpoint or its parent at a temporary symlink, or use a dead mount as an ancestor. Baseline normalization called synchronous `canonicalize`; preparation used Tokio filesystem operations whose blocking syscalls survive timeout.

Expected: status/config normalization performs no filesystem IO. Preparation refuses symlink components. A timed-out filesystem call keeps its guard until completion and does not freeze the runtime's blocking-pool shutdown.

Fix: normalization is lexical. Mount-table inspection refuses an existing mount and paths beneath known SMB/NFS mounts before directory IO. Preparation walks from an open root directory using `openat(O_NOFOLLOW | O_DIRECTORY)` and `mkdirat`, avoiding symlink following during creation. It runs in an owned plain thread with a retained lease and registered activity. A five-second caller timeout returns `CleanupPending`; the thread remains owned until its syscall returns. Unmount requires a matching mount-table entry.

Verification fixtures: lexical-normalization tests and `mountpoint_rejects_leaf_symlinks`, which also checks parent symlinks, existing directories, and new directories using canonical temporary paths.

Mac gap and limitation: confirm Darwin `openat`/`mkdirat` flags and filesystem permissions in a disposable local directory. `/tmp` and `/var` aliases are symlinks on macOS and are intentionally unsupported; use their resolved `/private/...` paths. The system mount command ultimately accepts a pathname. A concurrent external rename/symlink swap after preparation, or an unrelated process mounting at that path, remains a TOCTOU gap. No privileged atomic fd-based mount API was introduced. This is not protection against a malicious process running as the same user.

## P4: subprocess errors and Keychain disambiguation exposed or corrupted credentials

Severity: high. Reproduction: a fake helper echoes raw/escaped passwords or a partial secret to stderr and fails; a fake `security -w` returns `deadbeef` while `-g` fails or emits malformed metadata. Baseline redaction only matched URL-shaped complete secrets, `FetchOutcome` derived secret-bearing Debug output, and failed disambiguation could treat encoded data as a password.

Expected: diagnostics never include helper output, argv, or credential fragments. Ambiguous or malformed Keychain metadata fails closed. Both subprocesses share the caller's prompt deadline.

Fix: mount errors include only helper executable and exit status. The helper's stdout/stderr and argument values are omitted entirely. Credential Debug output redacts values; `security` failure diagnostics omit its output. Disambiguation validates metadata and hex before returning a password; malformed metadata and failed lookups fail closed. The caller's deadline passes to both commands and an outer deadline bounds their combined duration. There is no extra fixed 30-second prompt limit. Typed command timeouts map to `TimedOut`.

Verification fixtures: `command_failure_omits_argv_and_all_helper_output`, `metadata_is_fail_closed_and_debug_hides_secrets`, `fake_security_failure_does_not_echo_secret`, `disambiguation_failure_cannot_return_hex_as_password`, and `both_security_calls_share_one_deadline`.

Remaining exposure: the existing `mount_smbfs` CLI receives the password in argv. Other local processes may observe it while the child exists. Fifteen seconds is a requested termination deadline, not a hard exposure limit for a kernel-stuck process. The design does not write credentials to disk. Removing argv exposure needs a separate macOS credential/mount API decision.

Mac gap: run the explicitly ignored tests only against their disposable keychain. Verify missing item, literal hex, non-ASCII, trailing newline, ACL denial, and caller-budget expiry. Confirm a 90-second first-use allowance is preserved without consulting the login keychain in automated fixtures. Never use production credentials for this checklist.

## P5: explicit options bypassed soft/TCP/probe guarantees

Severity: high. Reproduction: configure `options = "bg"`, `hard`, `udp`, `proto=udp6`, `mntudp`, or a service port different from the probe. Baseline explicit options replaced every default, allowing a helper to background, filesystem IO to block, or the mount to use an unprobed service.

Expected: custom options retain the safe defaults, remain foreground/TCP/soft, and use the service port that the gate probed.

Fix: shared `validated_options` is called by config validation and argv construction. It merges defaults, rejects conflicting safety settings and background/UDP aliases, rejects mountd port overrides, requires the configured NFS service port to match the TCP gate, and rejects an unsupported custom SMB gate. Whitespace/control characters within an option and contradictory duplicate service ports are rejected. `proto=tcp6` requires IPv6 literal endpoints so the gate cannot succeed over IPv4 while the mount forces IPv6. Options such as `vers=4`, `ro`, and `rsize` remain available.

Evidence: Apple's `NFS/main/mount_nfs/mount_nfs.8`, reviewed by the config owner, documents `bg`, `hard`, explicit TCP versus fallback, `proto`, `mntudp`, `port`, and `mountport`: https://raw.githubusercontent.com/apple-oss-distributions/NFS/main/mount_nfs/mount_nfs.8

Verification fixtures: `explicit_options_keep_safety_defaults`, `explicit_empty_options_keep_soft_mounts`, `unsafe_options_and_mismatched_ports_are_rejected`, plus config-owner parse regressions.

Mac gap and limitation: verify generated SMB/NFS argv against the supported Darwin versions without mounting real shares during this audit. Remaining non-safety options pass through to the native parser. The NFS mountd ancillary RPC service remains outside the NFS service-port probe; rejecting an explicit `mountport` does not make a single TCP probe certify every negotiation dependency. Real-share acceptance needs a separate authorized disposable Mac environment.

## P6: mount enumeration failures looked like an empty mount table

Severity: high. Reproduction: force a synthetic enumeration error through FakeOps, or make the table outgrow the allocated snapshot between getfsstat calls. Baseline error paths returned an empty vector, permitting state changes based on missing information.

Expected: enumeration failure is an error, not proof that every share is unmounted. Listing must not stat or contact a dead share.

Fix: `mtab::list` returns `io::Result`, checks both `getfsstat` calls and truncation, and preserves `MNT_NOWAIT`. Non-macOS returns Unsupported rather than inventing a mount table. Engine/CLI callers were coordinated with their owners to fail closed. NFS bracketed IPv6 source extraction is corrected.

Verification: source review of both syscall return paths; source-address unit fixture; engine owner's scripted enumeration-error tests. Portable tests never replace production enumeration semantics.

Mac gap: native `getfsstat` ABI/build and enumeration under concurrent mount-table churn. Dead-share nonblocking acceptance remains Mac-only and must be separately authorized; no dead real share was created here.

## P7: cancellation left probe/dependency work and monitor cleanup detached

Severity: medium. Reproduction: cancel endpoint selection or dependency collection while injected operations remain pending; close a route monitor's stdout while its process stays alive; return plausible route output with a failing exit code.

Expected: selection cancellation drops outstanding probe futures, dependency cancellation reaches the command supervisors, a closed monitor pipe cannot indefinitely prevent restart, and failed queries cannot supply network identity.

Fix: probe/dependency collection uses JoinSet and injected probe deadlines. Network queries reject nonzero exits. The watcher kills and waits after EOF/error; its drop guard registers and owns cancellation cleanup through the global drain. Hostname lookup uses the same libc-backed `ToSocketAddrs` semantics in a registered plain thread; numeric addresses bypass resolution. Resolver calls remain uncancellable, but no longer hide in Tokio's shutdown-blocking pool.

Verification fixtures: `hung_injected_probe_is_bounded`, `cancelled_selection_drops_probe_futures`, `cancellation_kills_and_reaps_dependency_child`, `failed_query_cannot_supply_network_identity`, `closed_stdout_does_not_prevent_restart`, plus loopback TCP and injected priority tests.

Mac gap: fake route-monitor cancellation and restart with native process APIs, and resolver completion during synthetic shutdown. Real sleep/wake and routing-socket integration belong to the native acceptance checklist. A permanently stuck resolver may retain its thread; repeated attempts can accumulate such threads. No resolver sandbox or cache was added.

## Shutdown integration contract

Stop/abort **and join** all producers before calling `mountcmd::drain_commands().await`. Keep the runtime alive until it finishes. The activity counter covers shared subprocess supervisors, watcher cleanup, preparation threads, and resolver threads. `shutdown_drain_waits_for_registered_work` is the source regression fixture.

A kernel-uninterruptible child, filesystem call, or libc resolver may leave shutdown pending. This is deliberate ownership retention; a bounded caller/UI response is separate from successful cleanup. Force-dropping the runtime or externally killing the daemon defeats the async reaping guarantee. Parent/native owner owns actual-bin startup/shutdown integration and its fixture.
