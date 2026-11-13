# Final publication handoff

Repository: C-Nucifora/mountd. Purpose: harden the per-user macOS share manager
and preserve audit evidence. Branch hardening-audit-rounds targets main at
b39ea7ed42b418faa870f74f8edf282f60797fde. No other repository belongs to this
owner's publication. RACK, terminal and dashboard work remains with its owners.

Latest recorded branch push before this batch: 2026-11-12 11:52:14 +1000,
6c51d55. The GitHub events query returned no matching push event. One final
evidence commit uses the actual current time within the permitted window.
Configured SSH signing remains enabled. Published history is not rewritten.

## Final checks

- cargo fmt --check: passed.
- rustfmt --check --edition 2021 on both round-4 Rust fixtures: passed.
- Python AST parsing of both round-4 generators: passed.
- round4-identity-source-sha256.txt verification: all nine entries passed.
- Recovered native-boundary suite: four passed, outer exit 0.
- Recovered identity suite: exit 1 before tests, missing offline getrandom 0.2.17.
- Each final suite attempted heavy.lock once with a 45-second wait and exited
  75 before execution. No lock was acquired or retained, no build was started.
- Staged whitespace check is required before the final commit.

No broad suite, full native build or Clippy rerun was warranted for this final
documentation/fixture batch. Historical verification and independent review
remain in verification.md, review-disposition.md and swarm-rounds.md, with their
source revisions and limitations. No production source changed after 6c51d55.

## Publication and remaining gaps

Publish as a draft PR. The sole workflow runs macOS formatting, Clippy, Rust
tests, Swift checks, isolated installer tests and a build. It has no deploy,
release, install or credential step. No repository webhooks were returned.
There is no active pre-push hook or custom hooksPath. Protections are unchanged.
A PR CI result is separate from interactive native acceptance.

Missing acceptance requires a disposable macOS 13+ user or VM, Xcode tools and
the exact checks in mac-verification.md. Keychain, real shares and fixture
LaunchAgent registrations need separate authorization. Swift/AppKit/VoiceOver,
APFS, Darwin peer credentials, IOKit, native helper semantics and launchd remain
unaccepted. The identity suite remains unexecuted. The refused process/drain
assignment stays stopped, with no reroute or retry.

Retained residuals include SMB secrets in helper argv, same-user pathname races,
direct-child rather than arbitrary descendant ownership, kernel/libc blocking
and NFS ancillary-service limits. See review-disposition.md for exact scope.

The following untracked refused artifacts are preserved locally and excluded
from publication without reading or running them:

- tests/audit/round2-process-focused.log
- tests/audit/round2-process-helper.py
- tests/audit/round2-process-mountcmd.rs
- tests/audit/round2-process-probe.rs
- tests/audit/round2-process-runner.py

Both failed worker task records were stopped. This owner launched no service,
container, preview or network test. Listener inspection showed existing public
services and a public UDP container mapping; none belongs to this task or was
changed. Existing infrastructure was left alone.

## Previously published commits included in this PR

```text
187973f3e1abc3a6b7b58ef6f67c2a98d42e0416 harden daemon state and process boundaries
898d07432088c1d88534d8b07e3de7fc0d0bce2f bound menu commands and preserve installer recovery
d988c340c9098c30a7798ee8e16ff2e8090d55ad preserve launchd recovery through command cleanup
a66778e4afe1febeacb9ac611013251b3a69b60b document audit evidence and mac acceptance gaps
b4f33bf92aac511508bf36cae917f1aea0cde47d track extended hardening audit rounds
2c01b863bdc24da5a94241c8b7694a3eb11574c2 keep mount names addressable
674de98ea5b773f0bc37ac5ab867c5e6cf16610f honor explicit mount intent
c367823836ca1aaa685b928240fa04b1e22b692e serialize menu installer transactions
0be609a323c8e957bf8e4c20dc55e06d5e2bba07 record round one checkpoint evidence
b400b9de679f5bab468518e0baa49b406329e2ce challenge control names and shutdown admission
08189996c009a3538d7eec1ba7e2680975505d5a challenge installer locks and uncertain rollback
4ef4824485963172cb471f420765d1fc77a0b156 record review results and stopped audit scope
171eab5b7d8d8d183f2cb2cb39df745a5f32631f verify control name regression oracles
2224097e28815cfcba16a288a37ff22ad158058f reject ambiguous credential metadata
e38bd63bddb4b5850bc97fb62ae8403a2a4db741 record credential parser verification
92635eaa9a7a052d48b23c7ff005fd5b2fb9eec7 exercise migration and policy evidence sequences
6c51d55a64321e6d8cbff1773ad67bb8d5467c8c verify status data and menu contracts
```

The final evidence commit and PR URL are reported in the owner final response.
