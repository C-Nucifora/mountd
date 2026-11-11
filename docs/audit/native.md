# Native lifecycle and menu bar audit

This is the implementation owner's historical handoff. Current integrated
results are in [verification.md](verification.md); review corrections are in
[review-disposition.md](review-disposition.md). Earlier pending statuses below
record the resource pause, not the final verification state.

Scope: `src/launchd.rs`, `src/power.rs`, `src/bin/mountd.rs`, `menubar/`, and the reproducible native-area fixture. Baseline `b39ea7e`. No commits, pushes, installed apps, real LaunchAgents, credentials or shares were changed. The last push could not be established from the original tracking reflog or available hosting events. The parent retains the non-coursework past-day commit fallback.

## Issue ledger

### N1, high, effects started before IPC ownership

- Reproduction: start two daemons with the same isolated socket path, or make binding fail. Previously the detached IPC task logged its error while the engine, routing watcher and power watcher continued running.
- Expected: socket and singleton acquisition must succeed before any effect starts. A server failure must stop the daemon and preserve ownership through cleanup.
- Root cause: binding happened inside an unobserved spawned server; the reconciler had no readiness dependency. Shutdown unconditionally removed the pathname, which could remove a replacement server's socket.
- Fix: await `bind_socket` before constructing the engine or starting watchers. Keep an extra cloned singleton file. Select on IPC and engine task failures. Use `serve_bound_until` to stop accepts and drain accepted requests. Set the engine's permanent shutdown gate, abort and join reconciliation and routing producers, await IPC, then await `Engine::drain` and `mountcmd::drain_commands` before releasing the extra file. Leave socket and lock inodes for the next locked bind.
- Verification: source review against the IPC, engine and process owners' contracts. Parent owns the actual-binary duplicate-start and SIGTERM fixture. Final integrated execution is pending.
- Mac gap: exercise duplicate startup, server failure and SIGTERM during fake in-flight effects on macOS. Confirm no effects begin in the rejected process, no shutdown unmount occurs, and replacement startup remains rejected until cleanup completes.

### N2, high, installer could stop unrelated processes and lose the previous app

- Reproduction: the old menu installer used a process-name kill before copying over the installed bundle. A copy or bootstrap failure left the prior job stopped. The first rollback revision also failed to restart the prior service when moving the old app failed after bootout.
- Expected: target only this user's exact `gui/<uid>/org.mountd.menubar` service. A failed update restores the previous app, plist and loaded state, preserving configuration.
- Root cause: process-name targeting, replacement without a transaction, and a disk-change flag incorrectly doubling as service-stop state.
- Fix: stage the new app and back up the previous plist before bootout. Track service-stop and disk-change state separately. Restore both on failure. Preserve the unloaded state when the old service was already unloaded. Report incomplete recovery and retain its staging directory instead of deleting recovery files.
- Verification: `menubar/test-build.py` uses a temporary HOME and fake uname, compiler, signing, copy/move and launchctl boundaries. Completed checks cover install/update, XML paths, configuration preservation, copy, plist backup, old-app move, new-app move, plist write, bootout, bootstrap, permission failure, failed fresh installs, unloaded rollback and rollback restart failure. Added recovery-copy and recovery-move failure injections have syntax validation but await the parent's serialized run.
- Mac gap: run the same fake-tool fixture with native zsh. Separately verify launchctl diagnostics and real service lifecycle in a disposable Mac user or VM containing only synthetic fixture apps. Linux fake launchctl results are not launchd acceptance.

### N3, high, launchd errors and custom configuration were lost

- Reproduction: update an existing daemon plist containing `run --config /isolated/custom.toml`; the old installer replaced its arguments with `run`. A bootout permission failure was ignored during install and uninstall. A failed bootstrap left the replacement plist in place.
- Expected: preserve the configured argument tail, refuse unreadable or incompatible existing launch configuration, and distinguish an absent service from operational failure. Uninstall must leave configuration files alone.
- Root cause: unconditional plist rendering, ignored bootout errors, unescaped XML paths and no rollback.
- Fix: read ProgramArguments through plutil before changing files; retain arguments following `run`; XML-escape values; inspect only the exact per-user service; propagate errors other than explicit service absence. Restore previous plist bytes and restart only a previously loaded service after a failed apply. Create the local log directory. Reuse the process owner's bounded, supervised command runner for launchctl and plutil.
- Verification: Rust tests cover escaped paths, argument retention, sibling binary targeting, missing binaries, command failure and deadline. `tests/portable-native.py` preserves a fixture generator that copies launchd code and replaces only the two native executable paths with synthetic tools. Its integration cases cover fresh-install bootstrap failure, loaded/unloaded update rollback, inspection/bootout errors, custom config retention, uninstall and repeated uninstall. The expanded fixture's execution was interrupted for resource limits and remains pending.
- Mac gap: validate `/usr/bin/plutil -extract ProgramArguments json -o -` and absence diagnostics on the minimum supported macOS version and current macOS. Confirm crash restart and clean-exit behavior for both generated LaunchAgents in an isolated account.

### N4, high, menu subprocess deadlines were not hard bounds

- Reproduction: use a synthetic mountctl that ignores TERM or exits after spawning a descendant that keeps stdout open. The previous implementation sent TERM and then waited indefinitely for process exit; pipe drain workers could also remain blocked after the caller returned. Launch failure started drain workers before a child existed.
- Expected: a failed helper cannot freeze menu refresh or accumulate blocked workers. Stdout and stderr must have finite limits and deadlines.
- Root cause: blocking pipe reads, an advisory termination signal, shared mutable worker state and process waits beyond the deadline.
- Fix: move bounded child execution to `menubar/Sources/Command.swift`. Drain both streams with nonblocking reads and a 1 MiB limit per stream, bound each read pass, use a monotonic deadline, close pipe handles on all paths, and send SIGKILL to a running overdue helper. Resolve mountctl off the main thread too.
- Verification: `menubar/test-command.sh` is executable and generates synthetic success, stderr failure, launch failure, TERM-resistant timeout and output flood cases. Shell syntax passes. No Swift toolchain is available here, so compilation, execution and Foundation child-reaping behavior remain unverified.
- Mac gap: run the command fixture, add a controlled inherited-pipe child if needed, and inspect owned child exit/reaping. Check polling and actions while the fake helper hangs, floods either stream, exits nonzero, disappears and later recovers.

### N5, medium, repeated refresh and Reveal could exhaust workers

- Reproduction: open the menu repeatedly during a slow status request, or repeatedly choose Reveal while the workspace call is blocked. The old client spawned another worker each time.
- Expected: at most one refresh and one Reveal operation at a time, while menu navigation and other UI remain responsive.
- Root cause: no in-flight guards for either entry point.
- Fix: main-thread refresh and Reveal guards. Refresh completion clears its guard on success and failure; a returning Reveal clears its guard and refreshes state. Preserve the existing deferred menu rebuild while the menu is being tracked.
- Verification: source review only. The helper fixture covers bounded subprocess failure; AppKit scheduling and UI recovery need macOS.
- Remaining limit: a permanently blocked NSWorkspace call still occupies one worker and prevents another Reveal until it returns or the app restarts. The change bounds worker accumulation; it cannot cancel an AppKit workspace call.
- Mac gap: repeatedly open and navigate menus during slow responses; confirm one refresh, stable item identity while tracking, continued keyboard navigation, and recovery after the fake daemon returns. Test Reveal against a synthetic local fixture and report failed workspace opens.

### N6, medium, accessibility depended on symbols and emoji

- Reproduction: inspect the status item and per-share rows using VoiceOver or Accessibility Inspector. The old client supplied an image description but no explicit control label; rows began with status emoji.
- Expected: assistive technology can read the daemon state, share name and share state without interpreting color or glyphs.
- Root cause: no explicit status-button or per-share accessibility labels.
- Fix: set status-button accessibility label and tooltip whenever the icon changes; give each share row a textual name and state label.
- Verification: source review only. AppKit typechecking and assistive-technology behavior are pending.
- Mac gap: inspect each status kind with VoiceOver, navigate into its submenu by keyboard, and verify action names and focus while refresh completes.

### N7, regression coverage, power acknowledgements

- Reproduction: synthetic engine completion, dropped acknowledgement after failure, closed engine channel, or an engine that never acknowledges within its budget.
- Expected: invoke the sleep acknowledgement exactly once on every path and keep waiting bounded. Ordinary wake and sleep-cancel notifications need no permission acknowledgement.
- Root cause: no new missing-ack defect was established. The original callback's contract needed direct coverage without registering real power notifications.
- Change: factor acknowledgement delivery into one helper accepting a fake allow closure. Replace the test that registered actual IOKit notifications with synthetic acknowledgement tests.
- Verification: portable fixture extracts the event, constants, acknowledgement helper and tests unchanged. It does not replace or emulate IOKit. Final execution is pending after the resource pause.
- Mac gap: compile the real FFI and confirm actual callback delivery, notification IDs, acknowledgement timing, forced/idle sleep and wake hints on a disposable Mac test machine. Synthetic closure counts do not prove native sleep behavior.

## Checks and limits

Completed in this resumed run:

- Verified package `mountd` and baseline/branch; preserved concurrent owners' files.
- `rustfmt --edition 2021` on the two changed Rust files.
- `cargo fmt --check` before Apple cross-check startup emitted no format errors.
- `zsh -n menubar/build.sh menubar/test-command.sh` passed.
- The installer fixture passed through the restart-failure recovery cases described above.
- Owned-file whitespace and Python source syntax checks passed.

Pending final integration checks, serialized by the parent:

- Re-run the final installer fixture, including recovery-copy and recovery-move failures.
- Run `tests/portable-native.py` for synthetic launchd integration and extracted power/helper tests plus clippy. Its prior cargo invocation was interrupted, so it is not a pass.
- Run the parent's actual-binary startup/shutdown fixture and remaining portable suites.
- Run final `cargo fmt --check`, Apple-target clippy/check and native `cargo test`. This run's Apple check printed `Finished`, but exited 130 after interruption and is not counted as a completed check.
- On macOS, typecheck both Swift source files, execute `menubar/test-command.sh`, and run the non-installing menu build.

Use a disk-backed TMPDIR and one Cargo build job for subsequent builds. No active temporary files were moved or deleted to reduce resource use. No test in this area opened TCP listeners or used real credentials, probes or shares.

Shutdown deliberately remains pending if an owned child, resolver or filesystem syscall cannot finish. The singleton stays held. The process owner tracks command supervisors, routing-monitor cleanup and plain filesystem/resolver threads; the engine owner retains effect leases until completion. A timeout that discards those guards would reintroduce overlapping effects.

## Mac acceptance checklist

All items remain open. Run only in a separately authorized disposable macOS account or VM with synthetic configuration and fixture binaries. Do not repoint or replace the user's installed app or LaunchAgents.

1. Run format, clippy and Rust tests with real macOS frameworks, then Swift typecheck, command fixtures, fake installer fixtures and a build without `install`.
2. Parse both generated plists with native plutil. Include paths containing spaces, ampersands and angle brackets, plus a custom daemon config path. Confirm per-user labels, sibling daemon path, argument preservation and `SuccessfulExit = false`.
3. With disposable fixture apps only, check fresh registration, update while loaded, update while unloaded, crash restart, deliberate clean exit and uninstall. Verify that other users' services, unrelated processes and fixture config bytes stay unchanged. Inject each documented install and rollback failure and inspect retained recovery files.
4. With fake mount effects, hold one operation during SIGTERM and during simulated IPC failure. Confirm accepts close, new effects are rejected, current work drains, owned child waits complete, and a replacement cannot acquire the singleton early. Repeat with duplicate startup and an unsafe socket path. The stale socket and persistent lock inode should remain safely reusable.
5. Exercise real IOKit delivery on the disposable machine. Record sleep acknowledgement timing when synthetic unmount work succeeds, fails and exceeds its budget. Confirm wake settling and no new synthetic mount effects between WillSleep and DidWake. This requires real macOS sleep/wake observation and is not covered by a Linux fixture.
6. Use a fake mountctl for valid, malformed, missing and delayed status; stderr errors; excessive output; TERM-resistant hangs; and recovery. Confirm one outstanding refresh, live menu navigation, stable tracked menu rows and useful error state. Verify VoiceOver labels, keyboard actions and Reveal responsiveness.
7. Real SMB/NFS dead-share behavior and native credential/Keychain prompts remain outside this area's synthetic verification. They require a separately authorized isolated share/credential environment and the process/engine owners' acceptance checklists. No production share or credential should be used.
