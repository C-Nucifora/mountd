# Independent engine and process review

Reviewed the integrated working tree against `b39ea7e`, including the parent's direct `mount_nfs` change and 16-slot resolver cap added during review. This is a source-only review. I did not build, run tests, start services, open browsers, touch credentials or shares, spawn agents, or commit. Only this report was written.

Read README, baseline/process/reconciler audits, the requested Rust modules and portable fixtures, and ponytail/unslop/realistic-commits. The branch is `main`; HEAD remains `b39ea7e`. The origin/main reflog has no reliable last-push record. The baseline records the same result from hosting events. No commit timing action applies to this review.

Parent reports the prior process snapshot passed 97 tests with five ignored and Clippy, and that the daemon fixture and reconciler checks passed. The changes made during review still need the parent's serialized rerun. Those results are supplied evidence, not checks I ran.

## Disposition

Do not call the hardening complete yet. Four high-severity gaps remain in credential ownership, option validation, the placement of the final mount gate, and shutdown accounting for nested producer tasks. There is also a stale-network observation gap and a Mac pathname-identity gap described below. The resolver accumulation concern is fixed in the updated source.

### E1. High: credential subprocesses do not retain the operation lease

Evidence: [engine.rs:885](/home/christian/Documents/dev/mountd/src/engine.rs:885), [ops.rs:46](/home/christian/Documents/dev/mountd/src/ops.rs:46), [ops.rs:96](/home/christian/Documents/dev/mountd/src/ops.rs:96), [cred.rs:86](/home/christian/Documents/dev/mountd/src/cred.rs:86), [cred.rs:116](/home/christian/Documents/dev/mountd/src/cred.rs:116), and [cred.rs:197](/home/christian/Documents/dev/mountd/src/cred.rs:197).

The engine holds its claim and lifecycle reader while fetching a password, but `Ops::fetch_password` has no lease parameter. Both `security` commands call `command_output` with `Arc::new(())`. Cancelling reconciliation during a Keychain prompt drops the engine lease immediately. The supervisor only then notices its receiver closed and starts kill/reap. A new attempt or reload can therefore acquire ownership while the old `security` process still exists. Kernel-delayed reap makes this an unbounded overlap, but the ordering violation does not require a stuck kernel.

The outer credential timeout returns `TimedOut` without awaiting cleanup, and `command_failure` also collapses `CleanupPending` to `TimedOut`. With `allow_passwordless`, the same attempt can advance to mounting while credential cleanup is still pending. A global activity count protects final daemon shutdown once registered; it does not protect this mount's claim or reload ordering.

Smallest complete fix: pass the same operation lease through the credential boundary and into both supervisors. Preserve a cleanup-pending outcome that always ends the attempt, including passwordless configurations. Use one absolute credential deadline and let supervisor cleanup report its state, instead of an outer timeout that erases whether cleanup finished. Keep the 90-second first-use allowance.

Regression needed: cancel or expire a fake credential child with reap held behind a gate. Assert the mount claim and lifecycle writer remain unavailable, no passwordless mount starts, and both become available only after reap. Existing engine fake credentials return immediately; the credential deadline test checks elapsed time, not ownership.

### E2. High: native NFS aliases bypass endpoint and address-family validation

Evidence: [mountcmd.rs:139](/home/christian/Documents/dev/mountd/src/mountcmd.rs:139), [mountcmd.rs:157](/home/christian/Documents/dev/mountd/src/mountcmd.rs:157), and [mountcmd.rs:193](/home/christian/Documents/dev/mountd/src/mountcmd.rs:193). Unknown options are appended unchanged. Only `proto=tcp6` gets an IPv6-literal restriction.

Concrete accepted configurations include `options = "localhost"`, `"inet6"`, `"noinet"`, or `"netid=tcp6"`. Apple's native parser sets `force_localhost` for `localhost` and resolves `LOCALHOST_ADDR` instead of the configured hostname. Thus a successful probe of a remote NAS can authorize an attempted mount of loopback. The address-family options can similarly make a successful IPv4 probe authorize an IPv6-only mount attempt. `proto=tcp` and `inet` also force IPv4 while a hostname probe can fall through to IPv6.

Native source checked read-only during review:

- [Apple mount_nfs.c](https://github.com/apple-oss-distributions/NFS/blob/main/mount_nfs/mount_nfs.c), lines 1633-1673, treats `netid` as an alias of `proto`; lines 1927-1931 and 2049-2053 implement `inet`, `inet6`, `noinet`, and `noinet6`; lines 1987-1988 set `force_localhost`; lines 2754-2755 substitute the loopback address.
- [Apple mount_nfs.8](https://github.com/apple-oss-distributions/NFS/blob/main/mount_nfs/mount_nfs.8), lines 139-162, documents family selection and that `proto=tcp` means TCP over IPv4.

Smallest complete fix: reject endpoint-redirection options such as `localhost`; validate `netid` using the same rules as `proto`; reject conflicting family selectors and require matching literal endpoints for explicit family restrictions. Alternatively, reject these optional selectors altogether while retaining ordinary options such as `vers=4,ro,rsize`. Apply the rule at config load and argv construction.

Add parser/argv regressions for the named aliases, contradictory selectors, IPv4-only and IPv6-only options against hostname endpoints, and allowed matching literals. Do not infer native semantics solely from the existing string-based argv tests.

During the earlier wrapper revision, `-o=hard` and `-b,-R=100` also bypassed validation because `/sbin/mount` converts dash-prefixed comma options into helper arguments. The direct-helper change removes that particular execution path. Do not report it as an active exploit in the current direct-helper revision.

### E3. High: blocking preparation runs after the final probe and sleep/shutdown checks

Evidence: [engine.rs:915](/home/christian/Documents/dev/mountd/src/engine.rs:915), [mountcmd.rs:400](/home/christian/Documents/dev/mountd/src/mountcmd.rs:400), and [mountcmd.rs:427](/home/christian/Documents/dev/mountd/src/mountcmd.rs:427).

The engine runs its final probe and checks `sleeping`/`stopping`, then calls `Ops::mount`. The real implementation enumerates mounts and waits up to five seconds for directory preparation before spawning the helper. No probe or engine gate is checked after that wait.

Concrete interleaving: an IPC force-mount passes the final checks, then preparation stalls for four seconds. `WillSleep` sets `sleeping=true`; its lifecycle writer waits for this manual request, which is not part of the cancelled reconciler task. When preparation returns, the request starts a new mount helper while the sleep gate is closed. A short configured sleep budget permits this even after the engine acknowledges sleep. Shutdown has the same window because accepted IPC work is drained rather than immediately cancelled. A server that disappears during preparation also receives a mount attempt based on the old probe.

Smallest complete fix: split bounded mountpoint preparation from helper execution at the effect boundary. Retain the lease through preparation, then perform final eligibility/probe and sleep/shutdown checks immediately before spawning the helper. Keep the mount-table refusal check close to execution and avoid another blocking preparation stage afterward. A callback at the prepared boundary is another option, but do not introduce a second unowned worker.

Regression needed: gate preparation, start a manual mount, deliver sleep or shutdown, release preparation, and assert no helper starts. Separately remove reachability during preparation and assert the mount is refused. The current daemon fixture invokes `command_output` directly and bypasses preparation, so its success cannot cover this gap.

### E4. High: nested producers can outlive the shutdown drain's zero count

Evidence: [deps.rs:72](/home/christian/Documents/dev/mountd/src/deps.rs:72), [probe.rs:100](/home/christian/Documents/dev/mountd/src/probe.rs:100), [engine.rs:328](/home/christian/Documents/dev/mountd/src/engine.rs:328), and [mountcmd.rs:270](/home/christian/Documents/dev/mountd/src/mountcmd.rs:270). Integration ordering is at [mountd.rs:142](/home/christian/Documents/dev/mountd/src/bin/mountd.rs:142).

Dropping a `JoinSet` requests task cancellation; it does not join tasks currently being polled on other runtime workers. Dependency and endpoint-selection child tasks carry neither an activity registration nor the engine lifecycle guard. Activity registration happens later, inside `command_output` or resolver creation.

Concrete interleaving on a multi-thread runtime: a dependency child is polling `check` and is descheduled just before `command_output` registers activity. Another worker aborts and joins the outer engine task. That drops the nested JoinSet and the lifecycle reader, but the dependency task has not finished its current poll. `eng.drain()` and `drain_commands()` can now observe no ownership and return. The dependency resumes its poll, registers activity, and spawns after that drain. The analogous window exists between starting a probe task and registering its resolver. Aborting does not interrupt synchronous code inside an in-progress poll.

Smallest complete fix: create an activity registration before each nested producer is spawned, move it into that task, and retain it until the task is dropped or completes. This keeps the count nonzero while the producer can create a command/resolver, including during cancellation. Explicitly joining nested producer shutdown is also valid, but requires a cancellation-safe owner; merely changing the comment to say JoinSet aborts is insufficient. Cover dependency and probe producers together.

Regression needed: use a multi-thread runtime and a barrier that holds a nested producer before child registration. Cancel and join its parent, begin drain, and prove drain stays pending until the producer finishes cancellation. The existing drain test manually registers an activity first; the daemon fixture supplies immediate fake deps/probes. Neither exercises this interleaving.

### E5. Medium: final condition evaluation retains a network snapshot across slow dependency checks

Evidence: [engine.rs:274](/home/christian/Documents/dev/mountd/src/engine.rs:274), [engine.rs:915](/home/christian/Documents/dev/mountd/src/engine.rs:915), and [config.rs:535](/home/christian/Documents/dev/mountd/src/config.rs:535). Dependency timeouts may reach 120 seconds.

`eval_ctx` captures and matches the network before awaiting every dependency. Even the final evaluation can therefore use a home-network match collected up to two minutes earlier. A slow unrelated dependency is enough. If the machine changes networks during that check and the same private address answers TCP on the new network, `when=network:home` or `only_if=network:home` still permits the mount. The final port probe establishes reachability, not network identity.

Smallest fix for this avoidable stale interval: gather dependency results before the final network snapshot, then evaluate the selected endpoint against that refreshed snapshot. If a network-change generation is available, reject a final probe whose observation spans a known change. This is separate from the unavoidable race between the final check and the actual syscall.

Regression needed: a fake dependency check changes the network snapshot before returning, while the address remains reachable. Both automatic `when` and forced-mount `only_if` must use the new network and refuse the attempt. Current tests change dependencies during credential fetching, not the network during dependency evaluation.

### E6. Medium: pathname equality does not cover default Mac filesystem aliases

Evidence: [mountcmd.rs:43](/home/christian/Documents/dev/mountd/src/mountcmd.rs:43), [mountcmd.rs:409](/home/christian/Documents/dev/mountd/src/mountcmd.rs:409), [mountcmd.rs:468](/home/christian/Documents/dev/mountd/src/mountcmd.rs:468), and [mtab.rs:64](/home/christian/Documents/dev/mountd/src/mtab.rs:64). Configuration overlap checks also compare normalized strings at [config.rs:746](/home/christian/Documents/dev/mountd/src/config.rs:746).

On a case-insensitive APFS volume, `~/mnt/NAS` and `~/mnt/nas` name the same directory. They pass the string collision check, receive separate per-name claims, and can both pass empty mount snapshots before preparing the same directory and starting helpers. `O_NOFOLLOW` prevents symlinks; it does not distinguish case aliases. A differently cased path below an existing network mount can also miss the ancestor refusal. This is not the documented adversarial pathname-swap residual and needs no malicious peer process.

Smallest safe direction: enforce one filesystem spelling/identity for prepared local mountpoints and reject aliases, rather than treating two spellings as distinct effect targets. Use the already-open directory descriptors for any native identity check and keep it inside bounded preparation. Conservatively refuse possible aliases of mounted network paths before walking them. Do not fix this by adding synchronous `canonicalize` to reconciliation or globally lowercasing paths, which can select the wrong mount on a case-sensitive volume.

The exact native implementation needs a disposable Mac check, including APFS normalization behavior. Add a no-share fixture with two differently cased local paths and assert that only one logical target can be admitted. Linux lexical tests cannot establish this property.

## Resolved during review

### Resolver concurrency cap

The original implementation started one thread per hostname lookup and retained timed-out threads indefinitely. Repeated reconcile or explain calls could exhaust process resources. This was a substantive hardening defect, not an acceptable uncancellable-libc residual.

The current [probe.rs:19](/home/christian/Documents/dev/mountd/src/probe.rs:19) and [probe.rs:58](/home/christian/Documents/dev/mountd/src/probe.rs:58) fix that accumulation. A static semaphore admits at most 16 workers, `try_acquire` fails closed before thread creation, and the permit lives inside the thread until the lookup returns. Caller cancellation cannot release it. Spawn failure drops the closure and releases its permit/activity. Numeric probes bypass DNS. The one-slot fake lookup test at [probe.rs:146](/home/christian/Documents/dev/mountd/src/probe.rs:146) checks cancellation retention, saturation without another lookup, and recovery after release. Source review accepts this fix; execution remains the parent's job.

If all slots become permanently stuck, hostname probes fail closed and shutdown intentionally remains pending. Numeric endpoints remain usable. A resolver subprocess or cache is not needed to fix accumulation. E4 still applies to producer registration before this capped worker is created.

### Direct NFS helper

The initial reviewed argv used `/sbin/mount -t nfs`. [Apple mount.c](https://github.com/apple-oss-distributions/diskdev_cmds/blob/main/mount.tproj/mount.c), lines 882-935, forks the filesystem helper and waits for it. Killing and reaping that wrapper did not prove the effect-owning helper had exited. A kernel-stuck `mount_nfs` could survive the group signal while the wrapper reaped and released the lease.

The updated [mountcmd.rs:236](/home/christian/Documents/dev/mountd/src/mountcmd.rs:236) invokes `/sbin/mount_nfs` directly. With background options rejected, the supervisor now waits for the effect-owning process. This is the smallest appropriate fix for that concrete defect. The native reviewer separately owns the changed launchd and power integration; I have not treated those pending checks as completed.

## Owned descendants and stated residuals

- The supervisor at [mountcmd.rs:337](/home/christian/Documents/dev/mountd/src/mountcmd.rs:337) signals a dedicated process group and explicitly waits for the direct child. It never waits for all descendants. Even an ordinary descendant that stays in the group may remain kernel-stuck after SIGKILL. Also, a successful shell can close both inherited pipes and exit after starting background work, causing the success path to return while that work runs. Do not claim general process-tree completion from direct-child reap. Direct foreground effect helpers address the known NFS case; trusted dependency commands must be foreground predicates. If the requirement covers arbitrary descendant effects, this remains unsatisfied and needs explicit descendant tracking or a narrower executable contract. A `setsid` escape is a separate, accepted trust boundary, not the only descendant limitation.
- SMB argv exposure beyond 15 seconds is an honest kernel-stuck-process limitation. Keeping the lease until reap is the correct response. Removing argv exposure requires a separate native credential/mount API decision; this review does not recommend that platform change. Helper output and arguments are omitted from failure diagnostics, and credential Debug hides secrets.
- The fd walk rejects symlinks during preparation. A same-user rename or external mount after preparation remains a pathname API race. That is acceptable only as the documented trust boundary, not a claim of atomic path security. E6 concerns ordinary aliases before any attacker-controlled swap.
- Ancillary NFS mountd/RPC negotiation is not certified by a TCP probe of the NFS service port. This documented limitation does not require removing NFSv3 or replacing platform APIs. Keep the claim specific to the configured service endpoint and preserve foreground/time-bounded helpers. E2 is different because options redirect that primary endpoint or its family.
- The resolver cap bounds retained lookup threads. It does not make libc cancellation possible or guarantee finite shutdown. Retaining ownership is preferable to allowing another daemon to overlap surviving work.

## What the existing regressions establish

The tests cover useful state transitions: cancellation of a fake mount holds both guards, reload waits without holding the status locks, enumeration failure produces no mount effects, migration stays polite when busy, health evidence resets on changed source, and wake settling suppresses recovery/migration. The production lease clones across mount/unmount and preparation are consistent with those assertions. Unmount escalation stops on `CleanupPending`; recovery/migration stop after unmount errors. Reload publication and runtime reset have no cancellation point between them.

Process tests check bounded output, direct-child kill/reap, a synthetic delayed-reap gate, deadline sharing, output suppression, symlink refusal, and watcher EOF cleanup. The inherited-pipe test proves prompt return, not descendant termination. The delayed-reap test directly exercises `finish_cleanup`, not a native kernel-stuck helper. The watcher has registered cancellation cleanup and explicitly waits after EOF/error.

The portable process fixture copies the process modules and stubs only the watcher notification target. The reconciler fixture replaces SystemOps with panics and uses FakeOps. The daemon fixture retains the real entry point and process supervisor but substitutes immediate network/dependency/probe results and a direct synthetic child. This separation is sound for portable checks, but leaves E1, E3 and E4 between the tested boundaries. No production Linux empty mount-table fallback should be added to make these tests pass.

Native enumeration retains `MNT_NOWAIT`, checks both syscall failures, and refuses a full/truncated second snapshot. Engine, explain and sleep paths fail closed on its errors. The source is consistent with that contract; native ABI and churn behavior remain unexecuted here.

## Remaining Mac acceptance gaps

After the parent's serialized checks, use a separately authorized disposable Mac environment for native process groups/direct-helper reap behavior, mount-table churn with `MNT_NOWAIT`, APFS case/normalization aliases, native mount-option semantics, adopted SMB/NFS source formatting, and IOKit sleep/cancelled-sleep/wake delivery. The Keychain integration tests remain explicitly ignored and need a disposable keychain, including ACL denial, first-use delay, literal hex, non-ASCII and trailing-newline passwords. Do not use production credentials or shares.

Dead-share nonblocking behavior, busy real mounts and network transitions during actual mount negotiation cannot be certified by synthetic Rust tests or Apple cross-compilation. No such acceptance work was performed in this review.
