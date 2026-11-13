# Independent native and IPC review

Reviewed the integrated working tree against `b39ea7e` on 2026-11-12. Changes remain in progress, so the line references below identify the source inspected during this review, not a frozen commit. Request changes on the lifecycle findings below. Portable fixtures do not establish native acceptance.

## Findings

### RNI-1, high: a process-group kill does not establish completion of the mount effect

Locations: `src/mountcmd.rs:231-243`, `src/mountcmd.rs:338-359`, `src/bin/mountd.rs:156-160`.

The supervisor sends SIGKILL to the process group but waits only for its direct child. Once that child is reaped, it releases the operation lease and activity registration. NFS still invokes `/sbin/mount -t nfs`, which delegates to the filesystem-specific helper on macOS. If the wrapper exits while its mount helper remains in a kernel operation, reaping the wrapper does not prove that the mount effect ended. The engine can admit another operation or finish shutdown and release its singleton while that helper still exists. This defeats the new retained-lease guarantee in exactly the kernel-delay case it is intended to cover.

There is also a source-demonstrable successful-exit case: a synthetic shell can start `sleep 60` in its existing process group with all standard streams redirected, then exit successfully. EOF and the leader's exit satisfy the success branch immediately; no group cleanup occurs and the lease drops with the descendant alive. This reproduction needs no `setsid` or malicious PID reuse. It is a proposed fixture, not an executed result.

Minimal fix: invoke `/sbin/mount_nfs` directly with the validated foreground options so the supervised child is the NFS effect owner. Check the remaining effect commands for intermediary processes and state the ownership boundary accurately. If supervision claims completion for a whole group, it needs evidence that the relevant descendants ended, not just successful signalling and leader reap. Do not extend a direct-child guarantee to arbitrary trusted dependency process trees.

The process ledger correctly says only direct children are reaped, but that limitation must be reconciled with the engine/native ledger's stronger mount-effect and singleton guarantees. `inherited_pipes_cannot_hold_command_open` asserts return timing and an error, not descendant completion. The actual-bin fixture uses `exec /bin/sleep`, so it cannot expose the wrapper/helper distinction. Native confirmation of the system helper topology and delayed cleanup remains required; no real mount was attempted.

### RNI-2, high: the generated LaunchAgent can force termination before drain completes

Locations: `src/launchd.rs:43-67`, `src/bin/mountd.rs:142-160`, `src/mountcmd.rs:314-319`.

The daemon now deliberately waits indefinitely for owned cleanup while holding the singleton. The generated plist does not set `ExitTimeOut`, leaving launchd's finite SIGTERM-to-SIGKILL policy in effect. During bootout, update, or managed shutdown, launchd can therefore kill the daemon before its drain finishes. The in-process lock and leases then disappear. Supervised commands use separate process groups, so launchd's ordinary cleanup of the daemon's process group is not a replacement for the daemon's supervisor.

Native reproduction: use a disposable synthetic LaunchAgent with a controlled drain held beyond launchd's exit timeout, then boot it out. Observe whether the daemon is killed and its singleton becomes available before releasing the synthetic effect. The existing actual-bin fixture sends SIGTERM directly and completes promptly; it does not exercise launchd's escalation policy.

Minimal fix: make the generated agent's exit policy agree with retained ownership, such as an explicit `ExitTimeOut` of zero for indefinite managed drain, and coordinate installer behavior with that policy. Do not substitute another finite timeout while claiming indefinite ownership. Document that explicit SIGKILL and OS teardown remain outside this guarantee.

This is a platform-contract blocker identified from the generated plist and shutdown code. The exact launchd behavior on supported macOS versions has not been executed here.

### RNI-3, medium: rollback treats a failed launchctl response as proof that no service change occurred

Locations: `src/launchd.rs:79-80`, `src/launchd.rs:118-119`, `src/launchd.rs:194-224`; related menu rollback at `menubar/build.sh:81-82`, `menubar/build.sh:94-96`, `menubar/build.sh:133`.

The new ten-second timeout can terminate launchctl after it has submitted a state-changing request to launchd. Killing the client cannot undo that request. If bootout changes service state and then times out, `install` returns at line 194 without entering recovery. If bootstrap registers the new job and then times out, recovery writes the old plist and attempts bootstrap without first removing the newly registered job. A fresh install instead deletes the plist while the registered job can remain running. These are observable partial states, not rollback to the prior service.

Concrete synthetic reproduction: extend the existing fake launchctl so `bootout` removes its `loaded` marker before delaying its response beyond ten seconds. Start from a loaded installation; the installer reports failure and leaves it unloaded. Separately, have `bootstrap` create the marker before delaying. A failed fresh install then removes the plist but leaves the marker loaded. For updates, make duplicate bootstrap fail as real launchd does. The same post-mutation failure mode should be considered for the menu installer.

Minimal fix: distinguish inspection from mutation and treat a timeout after mutation as an unknown outcome. Reinspect the exact service, settle or remove the attempted registration before restoring the prior job, and retain recovery information if the state cannot be established. Include bootout in the recovery transaction. A longer timeout alone does not remove the ambiguity.

Evidence gap: `tests/portable-native.py:65-74` injects failures before modifying `loaded`; its bootstrap also accepts duplicate registration. The menu fake likewise fails before mutation. Their current passing cases, when executed, cannot establish recovery from a lost or delayed response after a side effect.

### RNI-4, medium: validated sleep budgets can outlive the system acknowledgement window

Locations: `src/config.rs:508-513`, `src/power.rs:111-114`, `src/power.rs:132-155`, `src/bin/mountd.rs:105-116`; integration at `src/engine.rs:989-1040`.

Validation accepts `sleep_unmount_budget_secs = 86400`. The power callback uses that value directly while blocking its CFRunLoop before calling `IOAllowPowerChange`. With a retained operation preventing the engine's lifecycle writer from acquiring, both engine cleanup and the callback can wait far beyond IOKit's roughly thirty-second acknowledgement window. The OS can proceed without the promised acknowledgement, and the blocked callback also delays delivery of subsequent wake notifications. This is a retained baseline defect that the new validation currently certifies as valid.

The callback budget is also captured only at startup, while the engine uses the reloaded value. Starting with zero and reloading a nonzero budget permits the callback to acknowledge immediately while the engine continues pre-sleep work. Starting with a large value and reloading a small one does not repair the callback bound.

Concrete synthetic reproduction: parse the accepted large budget, keep the engine's sleep acknowledgement sender alive without completing it, and observe that `acknowledge_sleep` uses the full supplied duration. For reload, inspect or inject differing startup and current budgets. No real sleep or long-running reproduction was performed.

Minimal fix: cap the configured engine budget below the OS acknowledgement deadline and independently enforce a small fixed upper bound in the callback. The callback can wait for the engine's completion signal up to that bound, so reload need not leave it with a stale shorter configured timeout. Test zero, maximum, rejected oversized values, and reload in both directions using synthetic acknowledgements.

## Assessed scope

Read README and the baseline, IPC/config, native, and reconciler audit ledgers, plus the process ledger when it became available. Applied ponytail, unslop, and realistic-commits instructions within the explicit no-commit constraint. Checked the branch and tracking reflog; no reliable last-push entry was available, consistent with the baseline's recorded hosting-event check.

Reviewed the requested IPC, config, CLI, daemon, launchd, power and menu sources and the IPC/portable fixture generators. Traced relevant engine lifecycle guards, process supervision, mount argv/preparation and watcher cleanup. No unrelated feature changes are recommended.

Within the inspected IPC code, I found no additional blocker in same-UID checks, persistent socket locking, refusal to replace unsafe paths, newline/frame bounds, or accepted-request drain ordering. The daemon retains its extra singleton descriptor across its own cleanup. These conclusions assume the documented trusted home ancestors and same-UID boundary; they are source observations, not cross-UID Darwin results.

Config validation now shares option policy with argv construction, rejects lexical path hazards, and avoids filesystem resolution during status/reconciliation. Native option parsing and the remaining pathname race described in the process ledger are not proved by parser tests. Swift moves helper lookup/execution off the main thread, bounds pipe reads, guards refresh/Reveal and adds explicit accessibility labels. Compilation, Foundation reaping, VoiceOver, keyboard navigation and recovery remain unverified. In particular, the command deadline begins after executable lookup and `Process.run`; it does not establish a hard bound on blocked filesystem lookup or launch itself.

## Remaining verification

- Parent must run the serialized integrated checks. This reviewer ran no builds, tests, browser sessions or containers and makes no new pass claim.
- Add targeted synthetic coverage for the findings before treating their contracts as complete. Current actual-bin coverage verifies duplicate startup, a partial accepted frame, direct SIGTERM and direct-child disappearance. It does not cover launchd escalation, an IPC server task failure, a delayed descendant after leader exit, or replacement startup while such cleanup remains pending.
- Native acceptance still needs Darwin peer credentials, launchctl/plutil behavior, native option parsing, Swift compilation and child cleanup, accessibility, sleep/wake delivery and reload-budget behavior. Keep those separate from source findings and portable results.
- Uninstall currently stops the job before deleting the plist and does not restore the loaded state if removal fails. Its fixture tests bootout failure, not plist removal failure. This is retained behavior, not a new rollback guarantee established by this change.

Only this review document was written. No implementation or test files, credentials, real mounts, installed applications, LaunchAgents or services were changed. No commits or pushes were made.
