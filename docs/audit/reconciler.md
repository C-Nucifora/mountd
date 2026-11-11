# Reconciler audit

This is the implementation owner's historical handoff. Current integrated
results are in [verification.md](verification.md); review corrections are in
[review-disposition.md](review-disposition.md). Earlier pending statuses below
record the resource pause, not the final verification state.

Owned files: `src/engine.rs`, `src/ops.rs`, `src/reconcile.rs`, `src/state.rs`, and `tests/portable-reconciler.py`. Changes are uncommitted for parent review. Baseline is `b39ea7e` on `origin/main`. The tracking reflog has no reliable last-push entry; the parent already checked hosting events. This is not coursework. The past-day fallback would apply if commits were authorized.

All tests use synthetic mount tables, credentials, dependencies, network snapshots, probes and child lifetimes. No real shares, credentials, installed apps or LaunchAgents were touched.

## Findings and fixes

### R1. High: cancellation releases ownership before work finishes

- Reproduction: cancel a manual mount while probing, then attempt it again. The old boolean claim remains set indefinitely. Separately, cancel a running mount command or interrupt reconciliation for sleep; dropping the caller can release its claim before the child exits.
- Expected: cancellation before an effect releases the claim; once an effect starts, the claim and config lifetime remain held through actual child reap or completion of blocking preparation.
- Root cause: a manually cleared `attempting` boolean was not cancellation safe. A caller-owned lock alone also cannot cover a detached cleanup worker.
- Fix: an owned mutex guard replaces the boolean. An `OperationLease` contains both that guard and an owned lifecycle guard. `Ops::mount` and `Ops::unmount` pass the lease to the process owner's `mount_owned` and `unmount_owned`. The worker retains it through cleanup. `CleanupPending` cannot lead to another engine effect: each attempt selects one endpoint, and every unmount error ends recovery or migration. There is no engine endpoint retry loop after a mount error.
- Verification: `cancelled_manual_attempt_releases_claim`, `unmount_during_attempt_is_rejected_without_side_effects`, `cancelled_mount_retains_both_guards_until_child_reap`, and `concurrent_attempts_for_one_mount_are_serialized`. The latter now uses an explicit child gate; its previous 200 ms fake probe exceeded the newly enforced 50 ms probe timeout.
- Mac gap: run the process owner's kill/reap and preparation tests on macOS with synthetic helpers. Confirm engine claim retention through the actual process boundary, including delayed reap. No live mount is needed.

### R2. High: reload can overlap old config effects and publish stale state

- Reproduction: hold an old mount attempt, reload a changed or removed mount with the same name, and finish the old attempt. The old task can act after reload or recreate stale runtime state.
- Expected: old effects finish or are reaped before config publication. A failed reload preserves both config and runtime. A successful reload resets state for the changed configuration.
- Root cause: config publication and name-based runtime retention did not coordinate with active operations.
- Fix: reload takes the lifecycle writer, parses before changing config, then publishes config and clears runtime together with no cancellation point between them. Operation leases hold reader ownership until cleanup completes. Status uses the short config/runtime locks and remains responsive while reload waits.
- Verification: `reload_waits_for_attempt_without_blocking_status_and_discards_old_state`, `failed_reload_keeps_runtime_and_repeated_reload_resets_changed_spec`, and retained-lease tests that assert the lifecycle writer stays unavailable until reap.
- Mac gap: repeat reload/removal during delayed synthetic mount helpers on the native runtime; verify status stays responsive and no old effect follows successful reload.

### R3. High: stale probes can authorize a mount

- Reproduction: allow endpoint selection, remove reachability during credential retrieval, or remove reachability while migration unmounts the old endpoint. Previously the earlier probe still authorized mounting.
- Expected: the selected endpoint must remain eligible and pass a new probe after credentials and migration unmount complete. Normal attempts must still satisfy `when`; explicit force-mount may bypass only `when`.
- Root cause: the selection probe was reused after long intervening work.
- Fix: immediately before invoking mount, reevaluate dependencies/network conditions and endpoint eligibility, probe the chosen endpoint again, and check sleep/shutdown gates. Migration failure clears stale mounted attribution and reports an error.
- Verification: `credential_wait_invalidates_old_probe`, `dependency_change_during_credentials_prevents_mount`, `migration_reprobes_after_unmount_and_reports_failure`, and existing force-mount/credential tests.
- Mac gap: synthetic delayed credential/helper integration under macOS. The probe cannot guarantee a server remains alive after the TCP handshake; normal mount timeouts still apply.

### R4. High: sleep events wait behind reconciliation; timeout can release live work

- Reproduction: deliver `WillSleep` during a long probe or mount. Previously the main loop only consumed power events after the pass finished. Separately, let pre-sleep unmount exceed its budget while its child remains alive.
- Expected: sleep interrupts reconciliation promptly, acknowledgement stays bounded, no new mounts start before wake, and surviving cleanup retains ownership.
- Root cause: power events were read after reconciliation; timeouts dropped caller-local guards. Wake settling also failed to prevent health recovery/migration.
- Fix: the loop selects power events against reconciliation. Sleep sets a gate before bounded cleanup. Each pre-sleep unmount holds its own claim plus the lifecycle writer through its process lease. Wake waits for lifecycle ownership and resets backoff, health, condition-loss, migration and WOL timing. Settling suppresses health recovery and migration as well as condition-loss unmounts.
- Verification: `sleep_interrupts_probe_and_blocks_mounts_until_wake`, `sleep_interrupt_keeps_cancelled_child_claim_until_reap`, `sleep_unmount_budget_keeps_writer_until_child_reap`, `wake_settling_does_not_recover_a_suspect_mount`, and existing polite pre-sleep tests.
- Mac gap: IOKit callback-to-engine acknowledgement, cancelled sleep, actual sleep/wake clock behavior, and queued sleep/wake notifications. The portable fixture replaces only the power module with the event type; it does not test IOKit.

### R5. High: failed mount enumeration looks like an empty table

- Reproduction: inject a mount-table error with a reachable endpoint. Previously an empty result could permit another mount over an existing one.
- Expected: enumeration failure never authorizes automatic mount or sleep cleanup.
- Root cause: the original system boundary could not represent enumeration failure.
- Fix: propagate the process owner's fallible mount-table contract through `Ops`. Manual mount and explain return an error; reconciliation records the error and performs no effects; sleep skips cleanup on enumeration failure.
- Verification: `enumeration_failure_never_permits_a_mount` checks reconciliation, manual mount, explain and sleep with no effect calls.
- Mac gap: native `getfsstat` error/race handling belongs to the process owner's tests. No Linux empty-table substitute was introduced.

### R6. Medium: migration and health evidence outlives its endpoint

- Reproduction: replace an adopted mount's source address after old health failures, or fail a migration because the share is busy and then make it available.
- Expected: a new mount source starts a new health streak. Busy migration remains polite and needs fresh confirmations before retrying.
- Root cause: attribution was only initialized when empty; migration confirmations survived busy/error paths or ineligible periods.
- Fix: synchronize attribution from each successful mount snapshot and reset evidence when the source changes. Clear migration evidence when migration is unavailable, after a busy unmount, and on failed remount. Changed network snapshots clear retry backoff even without named network conditions.
- Verification: `changed_mount_source_starts_a_new_health_streak`, `network_snapshot_change_clears_backoff_without_named_conditions`, extended `a_busy_share_is_not_migrated`, and existing disappearing-server/failover tests.
- Mac gap: native mount-source formatting and adopted SMB/NFS attribution, using synthetic mount records before any separately authorized acceptance work.

### R7. High: shutdown must retain singleton ownership through effects

- Reproduction: stop the engine while a command is still cleaning up, then allow another daemon instance to start.
- Expected: shutdown rejects new effects and retains the daemon singleton until existing commands are reaped.
- Root cause: cancelling engine futures does not establish completion of the work they started.
- Fix: `begin_shutdown()` permanently closes the engine effect gate. `drain()` waits for lifecycle ownership after producers stop. Wake cannot reopen the shutdown gate. The native owner integrates these with IPC accept/connection drain and the process owner's `drain_commands()` while retaining the singleton.
- Verification: `cancelled_mount_retains_both_guards_until_child_reap` checks drain blocks before synthetic reap, completes afterward, and rejects mount/unmount even after wake. The parent owns the actual-bin duplicate-instance and SIGTERM fixture.
- Mac gap: native executable signal delivery, singleton release ordering and synthetic child reap on macOS. A kernel-stuck worker intentionally keeps drain pending; releasing ownership would permit overlapping effects.

## Verification record

- Passed after final source edits: `rustfmt --edition 2021 --check src/engine.rs src/ops.rs src/reconcile.rs src/state.rs` and `git diff --check` for those files. Python `ast.parse` accepts `tests/portable-reconciler.py`.
- Earlier portable snapshot: 35 of 36 engine tests passed. The sole failure was the old concurrency fixture's 200 ms fake probe under a 50 ms timeout. It has been replaced by a deterministic child gate. That run predates the final lease/shutdown changes and is not final acceptance evidence.
- Final Rust tests and clippy remain pending. The user stopped heavy work because the shared VPS was overloaded. The active owned fixture build and its child processes were stopped; active temporary files were not moved or deleted.
- Native full tests/clippy were blocked at baseline by Darwin mount APIs and IOKit framework linking on Linux. This fixture keeps production platform behavior intact, disables binary targets, retains the power event type, and makes every `SystemOps` method panic. Only synthetic engine/reconcile/state/ops tests run.

Parent's serialized command when resources permit:

```sh
TMPDIR=/home/christian/.local/share/agent-tmp CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 python3 tests/portable-reconciler.py
```

The generator creates and removes its own disposable crate, honors an optional `CARGO_TARGET_DIR`, runs the four owned test groups, then `cargo --offline --locked clippy --lib --tests -- -D warnings`. It defaults to the resource limits above. It does not run the daemon or test native acceptance.

No known additional owned-area defect is being deferred. Final compilation, test outcomes and the listed Mac integration checks remain unresolved; source inspection alone cannot certify them.
