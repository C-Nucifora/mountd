# Hardening handoff

Linux verification and the source audit are complete. Native acceptance is
blocked on a disposable Mac and separate authorization for Keychain, launchd
registration and real-share cases. No installed app, real shares, production
credentials or LaunchAgents were changed. Nothing was pushed or published.

## Local implementation commits

| Commit | Change |
| --- | --- |
| `187973f` | Daemon state, fresh gates, cancellation ownership, subprocess drain, mount options, paths, credentials, IPC and portable core fixtures |
| `898d074` | Bounded menu commands, refresh/Reveal guards, accessibility, installer rollback and CI fixtures |
| `d988c34` | Launchd uncertainty/recovery, signal handling in both clients, actual CLI drain and native/CLI regression fixtures |

The following documentation commit contains this handoff, the area ledgers,
independent reports, exact Mac checklist, verification runner, logs and source
hashes. The branch remains `main`; the remote-tracking baseline is unchanged at
`b39ea7e`. No reliable last-push timestamp was available from the tracking reflog
or the earlier hosting-event check. Commit timestamps use a short window within
the allowed past-day fallback. Configured SSH signing was retained.

## Evidence

- [Verification commands, counts and logs](verification.md). Suites overlap;
  counts are not a unique-test total. Apple-target Clippy passed, but does not
  execute or link native Mac behavior.
- [Requirement coverage](coverage.md) and [review dispositions](review-disposition.md).
  Four independent reports are preserved. The final signal variant was fixed
  as recommended, extended to the equivalent menu wrappers, and verified by the
  parent. No additional independent review followed that small correction.
- [Mac commands and acceptance checklist](mac-verification.md). Start with
  `bash tests/verify-macos.sh` in a disposable Mac checkout. It builds and runs
  synthetic checks without installing or reading credentials.
- Original area ledgers: [reconciler](reconciler.md), [process](process.md),
  [IPC/config](ipc-config.md), and [native/UI](native.md).

## Remaining limits and inputs

A linked disposable macOS 13+ account or VM with Xcode Command Line Tools, Rust,
Python 3 and zsh is needed. Native linking, Swift/AppKit, VoiceOver, IOKit,
APFS case/normalization behavior, Darwin peer credentials and launchd ordering
remain unverified. A separately authorized disposable Keychain and NFS/SMB
endpoints are needed for their acceptance cases. No home-machine SSH is assumed.

Supervision proves direct-child cleanup, not arbitrary descendant completion.
Dependency commands must remain trusted foreground predicates. Kernel/libc work
can keep shutdown pending with ownership retained. SMB passwords remain in
helper argv until exit, potentially beyond the usual deadline for a stuck
child. Path-based mount APIs retain a same-user rename/external-mount race after
safe preparation. The service probe does not certify ancillary NFSv3 RPC
services. These boundaries are documented, not certified away by mocks.

Temporary fixtures are stopped. No task container, public listener, proxy or
firewall change was created; unrelated services were left untouched.
