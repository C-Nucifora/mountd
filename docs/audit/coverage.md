# Acceptance evidence map

Use `verification.md` for command results and retained logs,
`review-disposition.md` for review fixes, and `mac-verification.md` for native
acceptance. The area ledgers preserve the original owner handoffs.

| Requirement | Current evidence | Remaining acceptance |
| --- | --- | --- |
| Concurrent operations and reload | Fake child gates retain per-mount and lifecycle ownership; status remains available; failed/repeated reload cases preserve or reset the right state | Native child behavior |
| Probe gating and network changes | Final gate follows credentials and filesystem preparation; dependencies precede the network snapshot; disconnect and network-change tests reject stale eligibility | Actual network changes during native negotiation |
| Sleep/wake | Synthetic interruption, cleanup timeout, fixed callback ceiling, reload budget limits, wake settling and permanent shutdown tests | IOKit delivery, cancelled sleep and real suspend timing |
| Disappearing servers | Fake health streaks and recovery; fallible MNT_NOWAIT enumeration in source | Dead-share enumeration and native ABI |
| Migration and busy mounts | Confirmation, fresh target probes, polite busy-unmount failure and source attribution regressions | Busy disposable shares and native source formatting |
| Cancellation and subprocesses | Synthetic cancellation/deadlines, direct-child reap, retained lease, cleanup-pending refusal, pre-spawn producer registration and drain regressions | Native helper topology and kernel-delayed completion |
| NFS/SMB options | Config/argv rejection of background, hard, UDP, endpoint/family redirection and mismatched ports; safe defaults remain additive | Native option semantics on supported macOS versions |
| Credentials | Synthetic failures, secret-fragment suppression, shared deadline and both security-call cancellation paths | Disposable Keychain ACL and encoding checks; five tests remain ignored |
| Paths | Lexical normalization, symlink refusal, conservative alias/nesting rejection and Mac descriptor spelling checks | Case-sensitive and insensitive APFS, Unicode normalization, pathname race limits |
| Unix socket and CLI | Private modes, same-user authentication, singleton, bounded framing, idle/oversized/malformed messages and actual control CLI | Darwin peer credentials with separate disposable users |
| Daemon lifecycle | Actual entry point with fake effects rejects duplicate startup before work, remains responsive, drains accepted IPC and reaps synthetic children on SIGTERM | Native launchd exit policy |
| Install/update/uninstall | Fake launchctl/plutil and menu installer rollback, post-mutation failures, delayed replies, preserved config and recovery files | Disposable per-user LaunchAgent registration |
| Menu bar | Source review, bounded helper, refresh/Reveal guards, deferred menu rebuild and accessibility labels; Swift fixture supplied | Swift compilation, Foundation reap, VoiceOver and AppKit recovery |
| VPS exposure | Loopback-only TCP/UDP fixtures and isolated Unix sockets; no task container, proxy or persistent listener | No mountd listeners remain; unrelated services unchanged |
| Independent review | Original audits, integrated recheck and focused RF-1 follow-up; recorded corrections and negative controls | Final signal guards received parent review and testing; native behavior remains open |
| Local commits | Reviewed implementation committed locally; audit artifacts accompany it | Final signature/worktree verification recorded in the handoff |

## Fixture boundaries

Reconciler tests replace SystemOps with hard failures and use FakeOps. Process
fixtures copy the process modules, supplying only the watcher's notification
target. IPC fixtures use a fake engine and test the actual CLI. The daemon
fixture replaces SystemOps, power delivery and the route executable explicitly.
Native-area fixtures replace launchctl/plutil executable paths and extract only
the unchanged pure acknowledgement logic. Production macOS behavior is not
replaced to make Linux pass.

Each generator has a distinct Cargo package identity. Sharing a build cache
previously let the daemon fixture reuse the IPC fixture's reduced library;
that compile failure was corrected and the affected generators rerun. This was
a fixture isolation defect, not a production compile failure.

The WoL fixture binds its owned sender and receiver to 127.0.0.1 and checks the
complete packet. Production broadcast behavior is unchanged. Test traffic does
not require public ports, firewall changes, production proxy changes or home
machine access.
