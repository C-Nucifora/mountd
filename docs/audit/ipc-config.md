# IPC and configuration audit

This is the implementation owner's historical handoff. Current integrated
results are in [verification.md](verification.md); review corrections are in
[review-disposition.md](review-disposition.md). Earlier pending statuses below
record the resource pause, not the final verification state.

Scope is `src/ipc.rs`, `src/config.rs`, `src/bin/mountctl.rs`, `tests/ipc_cli.rs` and `tests/portable-ipc-config.py`. Changes remain uncommitted. Baseline is b39ea7e. This is not coursework; the tracking reflog has no reliable last-push entry, consistent with the parent's past-day fallback. No real share, Keychain item, installed app or LaunchAgent was accessed.

## Findings and fixes

| ID | Severity | Reproduction and expected behavior | Root cause | Fix and regression evidence |
| --- | --- | --- | --- | --- |
| IPC-1 | High | Start a second server at an occupied isolated socket. The first socket inode and control channel must survive; the second daemon must fail before reconciliation. A regular file or symlink at the socket path must also survive. | `serve` unconditionally removed the path before bind, so two daemons could run and shutdown could remove a successor's socket. | Persistent owner-only `flock` acquired before stale detection; only an owned socket with a refused connection is replaceable. `bind_socket` returns readiness before effects start. `singleton_and_private_permissions`, `unsafe_paths_and_live_legacy_socket_are_preserved`, `rejects_symlinked_parent_and_lock`. Native owner wired startup and removed shutdown unlink. |
| IPC-2 | Medium | Put a symlink or unsafe file at the isolated lock path, or make the socket directory writable by other users. Startup must fail without modifying targets. A different UID must not control or impersonate the daemon. | Socket chmod was best effort, parent directory trust was inherited, and neither peer was authenticated. | Require owned non-symlink leaf directory without group/other write permission, then mode 0700; lock is O_NOFOLLOW/O_NONBLOCK, regular, single-link, owned, mode 0600; socket chmod failure is fatal. Both client and server require peer UID equal effective UID. Existing path-preservation and mode fixtures cover same-user Unix behavior; cross-UID Darwin acceptance remains pending. |
| IPC-3 | Medium | Send a valid JSON prefix without newline, a cap-exceeding frame, or connect without sending bytes. Reject incomplete/oversized frames and expire idle transport. A client that accepts but never answers must not hang mountctl indefinitely. | A capped reader did not distinguish EOF/cap exhaustion from a complete frame. The client accepted missing payloads as empty lists. Connection timeout covered the whole request, including mutable engine work. | Shared framing reader requires newline and reads cap+1 to detect overflow. Request cap 64 KiB; response cap 4 MiB on both sides; server returns an error for an oversized generated response. Ten-second read/write deadlines, 120-second total client deadline, at most 64 connection tasks. Engine operations complete under their own effect deadlines instead of transport cancellation. `framing_rejects_truncation_and_oversize`, `idle_request_is_closed_at_the_transport_deadline`, `client_deadline_bounds_an_idle_daemon`; CLI valid/error/malformed/truncated/oversized fixtures. |
| IPC-4 | High | Signal shutdown while an accepted request is incomplete or an operation is active. A replacement instance must remain excluded until connection work and owned effects finish. | Detached connection tasks outlived the accept task and its singleton file. Aborting the server could release the singleton before mutations finished. | `serve_bound_until(engine, listener, lock, shutdown)` tracks connections in a JoinSet, closes accepts, and joins requests without cancelling mutations. Server and connections retain shared lock ownership. Native owner additionally retains a cloned file through engine/process drains. `graceful_shutdown_drains_accepted_requests_before_unlocking` verifies an accepted partial request blocks shutdown and replacement bind until completion. Actual-bin failure/cancellation and child-reap acceptance belongs to parent/engine/process fixtures. |
| CFG-1 | Medium | Parse zero poll/probe values, extreme timers, invalid endpoint authorities, empty required mount strings, NULs, dot paths, root/home mountpoints, or dependency `equals` without `json_path`. Reject before effects, including reload. | Several settings were accepted until process/time arithmetic failed, and `equals` alone was silently ignored. Bare `~` was accidentally resolved as a name before home-path validation. | Bounded timers, positive failure thresholds, NUL/nonempty validation, bare-host/IP endpoints, absolute NFS exports, lexical dot/root/home checks, mount-root validation, dependency timeouts and required comparison paths. Home aliases are checked after pure lexical normalization. Regression tests include invalid timers/values, home spellings and ignored comparisons. Existing collision, nesting and reference checks remain. |
| CFG-2 | High | NFS `options="bg,hard"` previously passed through while replacing `soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp`. `udp`, a changed `port`, or SMB probe port other than 445 could also defeat the TCP service gate. Ordinary `vers=4,ro` must remain usable. | Custom options replaced all safe defaults; config accepted options unrelated to the probe service. Darwin bg forks an unsupervised retry child. | Config calls the process owner's `mountcmd::validated_options`, also called by `mount_argv`. It merges safe defaults and rejects known contradictory background/hard/UDP/interrupt/retry/dead-time options and unprobed service overrides. NFS port is permitted only when it matches the effective probe; custom NFS gate requires matching port. SMB custom gate is rejected while custom SMB ports are unsupported. Parse regression `config_rejects_mount_options_that_escape_the_probe_and_timeout_contract`; process owner owns argv regressions. |

## Darwin option evidence and limits

[Apple's mount_nfs manual](https://github.com/apple-oss-distributions/NFS/blob/main/mount_nfs/mount_nfs.8) documents background forking and retry behavior at lines 91-130, TCP/UDP transport and proto netids at 132-165, service port and ancillary mountd port at 168-172, interruptible/soft behavior at 194-208, and deadtimeout at 448-456. This is Darwin evidence, not Linux mount behavior. The manual was fetched read-only; no mount utility was invoked.

`proto=tcp6` requires every configured endpoint to be an IPv6 literal. DNS names are rejected with this option because a probe could otherwise connect over IPv4 while the mount forces IPv6. `proto=tcp` remains allowed. Option tokens containing internal whitespace or control characters and conflicting duplicate NFS service ports are rejected, avoiding dependence on Darwin parser precedence.

Other options remain user-controlled, including protocol version, read-only flags, caching and transfer sizes. This is a targeted safety policy, not an exhaustive option allowlist. Their Darwin compatibility and latency effects still need native checking. NFS v2/v3 ancillary mountd/locking RPCs are not individually covered by the NFS service-port probe. No claim is made that a successful TCP probe proves authentication, export access or mount success.

Only the immediate socket directory and lock/socket paths are checked for symlinks and ownership. Trusted home ancestors and same-UID filesystem activity remain outside an adversarial boundary. The lock inode is deliberately persistent; administrators must not unlink it beneath a running instance. Socket files remain after clean shutdown and are safely replaced after lock acquisition and stale proof. A custom home path too long for Darwin sockaddr_un still yields a clear bind/connect error; production path layout is unchanged.

Transport deadlines use Tokio's monotonic clock and can pause during macOS sleep. They bound active runtime time, not elapsed wall time across suspend. Draining an effect whose kernel child cannot be reaped deliberately retains ownership and keeps shutdown pending; it must not release the singleton and allow another instance to race it.

## Verification status

Completed on the final owned source:

- `rustfmt --edition 2021 --check src/config.rs src/ipc.rs src/bin/mountctl.rs tests/ipc_cli.rs` passed.
- `git diff --check -- src/config.rs src/ipc.rs src/bin/mountctl.rs` passed.
- Python `ast.parse` accepted `tests/portable-ipc-config.py`.
- Source review confirmed `DirBuilderExt` is imported and native startup uses the pre-bound readiness API and graceful drain contract.

No final Rust test or Clippy result is claimed. The original `/tmp` fixture was stale after the mount-table Result signature changed. The replacement generator uses current source files and a deliberately fake engine/mount table; forbidden operations fail rather than emulate macOS. Builds were interrupted after the provider/host stall, then this owner's two remaining fixture process trees were stopped at the resource-pause request. Their temporary files were left untouched. The parent's earlier Apple-target check passed before the latest option/shutdown changes; it is not final acceptance.

The portable generator is preserved in `tests/portable-ipc-config.py`. It builds only mountctl, uses real config/IPC/CLI/parser code and a fake engine, runs the config and IPC unit filters plus CLI integration tests, then Clippy. It does not execute unrelated imported modules' tests. The CLI server uses a unique short temporary HOME on disk beneath the current user's home, avoiding the 104-byte Darwin socket-path limit. All listeners are local Unix sockets; no TCP service is started. Future builds default to disk TMPDIR, one Cargo job, disabled incremental compilation and one test thread.

Parent's serialized verification commands, when resource limits permit:

```sh
TMPDIR=/home/christian/.local/share/agent-tmp CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 python3 tests/portable-ipc-config.py
TMPDIR=/home/christian/.local/share/agent-tmp CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 cargo check --locked --offline --target aarch64-apple-darwin --all-targets
TMPDIR=/home/christian/.local/share/agent-tmp CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 cargo clippy --locked --offline --target aarch64-apple-darwin --all-targets -- -D warnings
```

## Remaining Mac checks

Use a disposable macOS 13+ account with a short isolated HOME and empty/synthetic config. Do not change installed LaunchAgents or use production credentials.

1. Run `cargo test --locked --lib ipc::tests`, `cargo test --locked --lib config::tests` and `cargo test --locked --test ipc_cli` natively. Run repository formatting/lint gates on the integrated tree. Confirm malformed and missing payloads fail with empty CLI stdout, and state/status JSON remains compatible.
2. Run the parent's actual-bin lifecycle fixture. Confirm a second instance fails before any effect; the first socket inode and control channel survive; stale socket restart works; regular files, symlinks, unsafe parents and unsafe lock files survive rejection. Check directory 0700 and socket/lock 0600 under permissive umask.
3. Exercise same-UID authentication and rejection of another disposable UID at both client and server. Linux SO_PEERCRED testing does not establish Darwin LOCAL_PEERCRED/getpeereid behavior.
4. With synthetic delayed operations/children, send shutdown or fail the server while commands are active. Confirm accepts stop, connections join, mount/lifecycle/singleton leases persist until child reap, and replacement startup remains excluded. Repeat with an idle accepted client; its transport deadline must let graceful shutdown proceed. Do not infer this from source alone.
5. On a disposable Mac, verify generated NFS/SMB argv and the host version's documented option parser with synthetic helpers. Confirm `vers=4,ro` preserves the safety defaults, equal-port NFS configuration passes, conflicting options fail before helper execution, and `proto=tcp6` accepts IPv6 literals while rejecting DNS or IPv4 endpoints. Any real-share option/latency acceptance requires separately approved disposable endpoints; none was performed here.
6. Suspend/resume while an isolated client/server is idle and while a synthetic command runs. Confirm paused monotonic deadlines, bounded active-time recovery and responsive status; record wall-time behavior explicitly.

No known source defect remains intentionally unfixed in this owned area. Final Rust compile/test/lint, independent integrated review, Darwin peer credentials and native shutdown/option acceptance remain unverified. These are acceptance gaps, not passing results.
