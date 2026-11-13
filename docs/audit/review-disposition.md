# Independent review dispositions

The original reports remain unchanged in `review-native-ipc.md` and
`review-engine-process.md`. This records the integrated response.
The first integrated recheck is in `review-final.md`. It accepted the other
responses and raised RF-1 below. The focused recheck in
`review-launchd-followup.md` confirmed that correction and identified its
signal-termination variant. The parent applied the requested classification,
checked the equivalent menu wrappers, and verified both with fault reproductions
and passing regressions. No further independent pass followed that small change.
Native acceptance remains open.

| Finding | Change | Regression evidence |
| --- | --- | --- |
| RNI-1, high, wrapper reaping | Invoke `/sbin/mount_nfs` directly, with background options forbidden. Describe direct-child ownership accurately. | NFS argv tests, synthetic supervisor cancellation and delayed-reap tests. Native helper topology remains a Mac check. |
| RNI-2, high, launchd forced exit | Generate `ExitTimeOut = 0` for the daemon so managed shutdown can drain with the singleton held. | Plist assertion and native fixture. Actual launchd escalation remains a Mac check. |
| RNI-3, medium, mutation followed by failed response | Inspect service state after bootout/bootstrap. Treat observed desired state as success, settle attempted registration before rollback, and preserve recovery files when state is unknown. Apply the same rule to the menu installer. | Fake launchctl rejects duplicate bootstrap, injects errors after mutation and delays replies past the command deadline. Installer fixtures cover recovery failures. |
| RNI-4, medium, sleep acknowledgement budget | Validate cleanup budgets in 0..20 seconds; use a fixed 25-second callback ceiling independent of reload. Cancelled sleep emits wake. | Config boundaries, extracted acknowledgement logic and cancellation cases. Real IOKit delivery remains a Mac check. |
| E1, high, credential ownership | Pass the mount/lifecycle lease through Ops and both security subprocesses. Use one absolute deadline. Preserve CleanupPending and refuse passwordless continuation while cleanup is pending. | Engine fake-child cancellation holds claim and lifecycle writer; passwordless cleanup refusal; real synthetic cancellation of either security call asserts reap before lease drop; shared-deadline regression. |
| E2, high, NFS redirection aliases | Reject localhost/nolocalhost, proto/netid and inet/inet6/noinet/noinet6. Keep the protocol-neutral TCP safety default and ordinary vers/ro/rsize additions. Validation applies at config load and argv construction. | Config rejection and argv cases, including hostname and literal endpoints, contradictory selectors and matching service ports. Native option acceptance remains a Mac check. |
| E3, high, preparation after the gate | Split Ops preparation from execution. Retain ownership during bounded preparation, then refresh conditions, dependency and network observations, probe reachability and check sleep/shutdown. Execution only checks the nonblocking mount table and starts the helper. | Gated preparation followed by sleep, shutdown or disconnection must produce no mount call. |
| E4, high, nested producer/drain race | Register activity before spawning each dependency or endpoint producer. Keep registration until the producer completes or is dropped, so children cannot register after drain observes zero. | Multi-thread probe test holds a producer inside its poll before child creation, aborts and joins the parent, and proves global drain waits. Dependency cancellation separately checks child reap. |
| E5, medium, network snapshot before dependencies | Gather dependency results before the network snapshot in the shared evaluation path. | After initial selection and a credential gate, the final dependency check changes the network while reachability remains true; automatic when and forced only_if both refuse mounting. Restoring the old ordering fails the negative control. |
| E6, medium, Mac pathname aliases | Reject possible case or Unicode collisions/nesting conservatively. Refuse possible aliases of existing mounts before directory traversal. On Mac, check each opened directory's stored spelling through F_GETPATH before continuing. Never select an unmount by a folded path. | Pure comparison and config regressions; Mac-only local-directory fixture checks distinct-case identity and alias refusal. APFS case/normalization behavior remains unexecuted here. |
| RF-1, medium, unsettled launchd mutation | Preserve a typed uncertain outcome for command-supervisor errors and failed post-mutation inspections. Never promote timeout or CleanupPending from a presence snapshot. Skip ordinary rollback on uncertainty; stage previous plist bytes before mutation and retain attempted files. The actual CLI drains supervisors before runtime teardown on both success and failure. | Inject pending cleanup with desired state visible; defer server-side bootstrap/removal until after the client times out; check retained plists/backups and no contradictory mutation. An actual CLI fixture holds registered cleanup behind a gate after install fails. A seventh negative control restores the lost-uncertainty branch and must fail. |
| RF-1 signal follow-up, medium | Treat a mutating client with no exit code as unsettled before inspection. Menu wrappers likewise treat shell signal statuses as unsettled and skip rollback while retaining the staged app/plist backups. | Actual SIGTERM of a deferred synthetic client: fixed Rust fresh/update cases and menu bootstrap/bootout cases preserve recovery state. Removing the respective guards reproduces lost attempted files and conflicting recovery bootstrap. Native and installer fixtures pass. |
| Resolver accumulation | Limit uncancellable libc lookups to 16 retained slots; saturation fails closed. | One-slot blocked lookup survives caller cancellation, refuses another lookup, then permits work after release. |

## Remaining limits

- A supervisor waits for its direct child and signals its dedicated process
  group on failure. That does not prove arbitrary descendants have finished.
  Dependency commands must be trusted foreground predicates without background
  work. Direct NFS invocation removes the known wrapper gap.
- Explicit SIGKILL, runtime destruction and OS teardown defeat in-process guard
  retention. Managed daemon shutdown waits for registered work and holds the
  singleton. Permanently blocked kernel/libc work can keep it pending.
- SMB passwords remain in helper argv until exit. A kernel-stuck helper can
  exceed the normal 15-second deadline. Error messages omit argv and helper
  output, including partial secret fragments.
- The final path-based mount syscall still permits a same-user rename or
  external mount after safe preparation. No synchronous canonicalization or
  stat of configured mount paths was added. Conservative Unicode comparisons
  can reject distinct names, even on case-sensitive volumes.
- The TCP gate covers the configured service endpoint and port. It does not
  certify NFSv3 ancillary mountd/RPC services or replace native protocol
  negotiation. Native acceptance cannot be inferred from mock results.
