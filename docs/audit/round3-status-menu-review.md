# Round 3 independent status and menu source review

No substantive new defect is established. All six new portable tests passed
with the reviewed dependency versions, and source identity checks passed.
This report does not claim Swift decoding,
AppKit recovery, focus or accessibility execution. The parent owns round-level
acceptance, integration and any production correction.

## Reviewed source and boundaries

Reviewed `/home/christian/Documents/dev/mountd`, branch
`hardening-audit-rounds`, at reviewed local commit
`2224097e28815cfcba16a288a37ff22ad158058f`, directly descended from
`171eab5b7d8d8d183f2cb2cb39df745a5f32631f`. Origin is
`https://github.com/C-Nucifora/mountd.git`. All shell commands used that explicit
working directory. Local tracking reflog records the last branch push at
`2026-11-12 10:30:07 +1000`, at `171eab5`. No network query was made. The only
local commit beyond that tracking ref at entry was `2224097`. During review
the parent advanced and pushed the shared branch to
`e38bd63bddb4b5850bc97fb62ae8403a2a4db741`, record credential parser verification.
Its push reflog timestamp is `2026-11-12 11:23:52 +1000`. All six reviewed
production files remain byte-identical to `2224097`; this reviewer did not
publish anything. At handoff HEAD was
`92635eaa9a7a052d48b23c7ff005fd5b2fb9eec7`, the parent's decision-review
artifact commit, pushed at `2026-11-12 11:43:43 +1000`. The final checks log
confirms the six production files and Cargo.lock still match `2224097`.

Read the supplied instructions and ponytail, unslop and realistic-commits.
No additional repository or ancestor AGENTS.md was found. The explicit
no-commit instruction overrides commit creation and scheduling. Read README,
coverage, review-disposition, native, round1-ipc-review,
round1-installer-review, round2-credential-review, swarm-rounds and
mac-verification. Existing state tests, engine status tests, IPC/CLI tests and
portable fixture definitions informed the new matrix. Existing menu command
fixtures supply no MountStatus or AppKit acceptance and were not run.

Only this report and `tests/audit/round3-status-menu*` belong to this review.
No production files, previous findings or other owners' artifacts were edited.
No branch, worktree, thread, subagent, commit or push was created. No credentials,
production config, installed app, LaunchAgent, native security, real share,
mount operation, network, listener, browser or device was used.

The separate process/drain assignment remains stopped after the provider
refused it with "This content was flagged for possible cybersecurity risk." Its
`tests/audit/round2-process*` contents were not inspected, executed, adapted or
modified. Initial Git status listed their names only. No command-supervisor,
deadline, reaping, resolver or DNS test was authored or run, and no route/model
change or takeover occurred. No part of this status review was refused.

## Contract review

| Contract | Exact source and conclusion |
| --- | --- |
| Lifecycle to wire | `src/state.rs:46` and `:75` define ten kinds and sixteen variant/payload forms. `src/engine.rs:133` copies each configured name and path, renders state and kind from the same runtime value, and carries nullable endpoint/error details separately. Missing runtime becomes Unknown. It iterates config order and omits orphan runtime entries. |
| IPC to control output | `src/ipc.rs:45` uses a typed response, with optional status. The status handler at `:77` always returns `Some`, including an empty list. `src/bin/mountctl.rs:44` rejects `ok:false` before the status branch at `:65` requires the payload and emits the array rather than the envelope. |
| Swift schema | `menubar/Sources/main.swift:12` requires name, state and mountpoint strings. Endpoint, error and kind are optional strings. Rust emits matching keys and string/null details. Older omitted kind defaults to an empty Rust string and reaches Swift's fallback. Swift source allows null kind; Rust's String does not. Neither current output nor the documented older omitted-field format emits null kind, so this asymmetry does not establish a producer/consumer failure. |
| Kind selection | Swift `:23` uses every nonempty structured kind before considering display prefixes. All ten current Rust kinds pass through unchanged. Older mounted, error, waiting, suspect, recovering, waking and migrating prefixes have explicit fallback. Legacy idle and unmounted fall back to unknown, which has the same neutral glyph and Mount Now action as their structured forms. No current action mismatch follows. |
| Degraded state | Swift `:174` prioritizes daemon error, error, suspect/recovering, then mounted. `:191` gives migrating an orange row, waiting/waking yellow, and other kinds neutral. `:219` treats mounted and suspect as present, matching Rust `is_mounted`. Migrating and recovering use Mount Now in source, consistent with that helper's false value. This is a state-model comparison, not evidence that a filesystem has disappeared at every intermediate instant. |
| Empty, unknown and future kinds | A successful empty array reaches no mounts configured. Unknown and unsupported nonempty kinds retain their full textual state but use neutral/default presentation. An unknown-only aggregate says no mounts active, including before the first reconcile. That is an existing presentation limit, not proof of filesystem absence. Future kind semantics are not specified, so this review does not invent an action or restyling requirement. |
| Invalid or absent data | Swift `:115` resets refreshing before decoding. Failure supplies an empty row list plus the helper error or could not parse status. `:135` replaces the stored state and updates the icon. Later valid data sets the error to nil, including a valid empty list. A malformed response uses the generic daemon unreachable heading with parse detail; it does not retain stale rows after rebuilding. This is source evidence only. |

## Menu tracking, action values and accessibility

`main.swift:78` marks the menu open before requesting a refresh. Completion
updates cached statuses and the status-button icon, but `:140` sets
pendingRebuild instead of calling removeAllItems while tracking. `:83` clears
the open flag and rebuilds once if pending. Several changed responses therefore
coalesce to the latest cached value. A later identical response does not erase
an earlier pending rebuild. Returning to the initial value may cause an
unnecessary rebuild after close, but does not rebuild during tracking.

`main.swift:213` iterates value-type MountStatus rows. Each action closure
captures that iteration's `s`; Reveal uses its mountpoint and mount/unmount
use its name. The handler never looks up a current array index. Old rows can
remain visible until the menu closes while the status button already reflects
a newer result. This is intentional deferral, not evidence of a wrong target.
A reload that removes a selected name can still make the daemon reject that
snapshot's action. This review does not add config-version or pathname identity
guarantees. The already accepted round-1 `--` fix is preserved and is not
counted as new coverage.

`main.swift:166` supplies an image description, explicit status-button label
and tooltip. Each row at `:217` has a textual name/state label, independent of
the emoji. Action items have textual titles and self-targeted selectors at
`:288`. Disabled explanatory rows have textual titles. Key equivalents are
empty and there is no custom keyboard or focus implementation. These facts
support a native NSMenu navigation check; they cannot demonstrate VoiceOver
announcement order, disabled-row discoverability or focus preservation.
AppKit's menu-close/action-dispatch ordering, including whether a closing
rebuild affects activation of the chosen item, remains native acceptance.

No substantive source objection requires a production fix in this review.
The semantic limits above and the native checks below remain open. In
particular, an executable Rust copy of Swift/AppKit was not used as a UI oracle.

## New portable matrix and extraction limits

The new fixture has six test functions. The prior state tests cover selected
prefixes and kinds, and existing IPC tests cover an empty response plus selected
invalid frames. This suite instead composes every lifecycle variant with the
actual engine projection, response schema and CLI serialization expressions,
and exercises row-level schema failures and value preservation.

| New hypothesis | Matrix and oracle |
| --- | --- |
| Less-used variants change meaning across status serialization | Sixteen cases assert exact state/kind, Rust is_mounted, six emitted fields and preservation through response decoding and CLI array serialization. Includes both adopted/pinned forms, both manual flags, degraded states and maximal retry seconds. |
| Optional-field omission breaks older status rows | Eight combinations omit kind/endpoint/error, plus two nullable detail cases. Assert documented defaults and exact normalized row preservation. |
| A malformed row becomes a partial or empty successful list | Thirty-one malformed rows, before and after a valid row, require 62 whole-response decode failures. Each is followed by a valid decode. Seven malformed array documents are followed by successful empty-array decoding. This proves independent Rust decode recovery only. |
| An error or missing payload becomes no mounts configured | Eleven envelope cases distinguish empty success, absent/null status, false ok with null/empty/nonempty payload, missing error and malformed ok/status types. Exact CLI missing-status and daemon-error expressions determine refusal. |
| Status order or snapshots depend on runtime-map order | Exact status method omits an orphan, follows reverse-sorted config order, defaults missing runtime, and returns detached values across later runtime/path changes. Clearing synthetic config returns an empty list. No real reload or lifecycle policy is executed. |
| Escaping or future fields change opaque status data | Four discriminant forms preserve quotes, backslashes, Unicode and multiline error text. Unknown additive fields are ignored. A deliberately mismatched display/kind stays unchanged in Rust; Swift precedence is separately reviewed in source. |

The final generator verifies six production files and Cargo.lock byte-for-byte
against `2224097`
before extracting anything. It copies unchanged `state.rs` without its old test
module, MountStatus, Engine::status, IPC Response and the three explanation data
types. Explanation types are needed only to compile the actual Response type;
no explanation or decision code executes. The fixture's Engine shell contains
only a config mount list and runtime locks. The config shell contains only name
and mountpoint strings. It does not stand in for config validation or engine
construction.

A pure wrapper joins the exact CLI `ok` guard, required-status statement and
serde_json serialization expression. It does not invoke the control binary,
IPC transport, helpers or runtime shutdown. All oracles consume synthetic data.
The source's old tests are excluded; the six new functions are the unique count.
Matrix rows are not separate test functions. No negative control is needed to
claim a fix because no fix or newly failing product regression is claimed.

The disposable Cargo package has a distinct name and directory under disk
TMPDIR. Cargo is offline and uses `target/portable-daemon` only while the shared
lock is held. The generator logs exact source/extraction/fixture/generated
hashes, the compiler command, exits and elapsed time. It rechecks source bytes
after execution and retains only its own inactive scratch. It never deletes
other scratch.

## Commands and results

Each focused invocation used this command, with explicit cwd
`/home/christian/Documents/dev/mountd`, is:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 3600 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round3-status-menu.py \
  > tests/audit/round3-status-menu-suite.log 2>&1
status_menu_exit=$?
printf 'outer_exit=%s\n' "$status_menu_exit" >> tests/audit/round3-status-menu-suite.log
exit "$status_menu_exit"
```

The runner gives its one offline Cargo test invocation 220 seconds, within the
235-second outer execution limit and five-second termination grace. There are
no parallel heavy jobs or expired-lock retries from this reviewer. Each
invocation releases the lock before the next acquisition.

The initial invocation passed all six tests, with zero failed, ignored or
filtered tests. Cargo and the outer command both exited 0. It took 13.73 seconds
after acquisition, with 0.02 seconds reported by the test binary. Source identity
passed. That generator allowed Cargo to resolve cached dependencies offline;
final inspection found serde_json 1.0.154 and Tokio 1.54.1 rather than the
repository's 1.0.151 and 1.53.1. This is a fixture-fidelity limit, not a product
failure. Its unmodified output is retained as
`tests/audit/round3-status-menu-initial-suite.log`, and its generated source and
lockfile remain under
`/home/christian/.local/share/agent-tmp/mountd-round3-status-menu-mwth3w1c`.

The generator now seeds the reviewed Cargo.lock with only the disposable root
package renamed and checks every resulting registry package's name, version and
checksum against that reviewed lockfile. Cargo may remove unused packages and
feature dependencies in the smaller crate. A second serialized invocation ran
the same six tests to resolve that dependency-fidelity concern; it is not six
additional tests or new coverage. No test oracle or production source changed.

The dependency-aligned invocation passed all six tests, with zero failures,
ignored or filtered tests. Cargo and the outer command both exited 0. Every
registry dependency matched a version and checksum from the reviewed Cargo.lock,
including serde_json 1.0.151 and Tokio 1.53.1. Source identity passed after the
run. Elapsed execution was 5.137 seconds, with 0.02 seconds reported by the test
binary. Its exact generated Rust bytes match the initial run, SHA-256
`ef39f80ee2e2c154f0fafdfcc5537e0f7829b65271b638e0175b87375350ee6f`.
The final generator SHA-256 is
`6a92ef9321d3dce9c6383f8f04c00023a6e9c5c0d0d68e664a955b44f3f4c140`;
the Rust fixture SHA-256 is
`81e07029caccd841e5ac7902907f851b3c552d9b8adbee6bf9e78144347cd5c0`.
The inactive final scratch is
`/home/christian/.local/share/agent-tmp/mountd-round3-status-menu-njakc9qt`.
Both invocations have completed and released the shared lock. No test job
remains queued by this reviewer. The runner uses Python 3.11+ for tomllib;
this host ran Python 3.14.4.

Production hashes are retained in
`tests/audit/round3-status-menu-source-sha256.txt`. Exact lightweight commands
and exit statuses are in `tests/audit/round3-status-menu-checks.log`. Python AST
parsing, Rust formatting, owned-file whitespace, reviewed-source identity and
final artifact inspection passed. Linux has no
`swiftc` in PATH; no Swift compiler or AppKit execution was attempted. Broader
Cargo, portable suites and the parent's Apple-target check are not
repeated or claimed here.

## Preserved dispositions and objections

Round 1's four fixes remain accepted within their recorded portable scope.
Fresh IPC and installer challenges are preserved. The IPC reviewer's expired
negative-control waits remain historical; the parent later ran both controls
and observed their required failures. The installer owner-shell TERM/HUP
observations remain unrun. They are not taken over by this assignment.

R2-CRED-1's corrected parser remains accepted by its independent parser review
and the parent's portable checks at this checkpoint. The parent's Apple-target
check was pending at assignment. The five native Keychain tests remain unrun.
Embedded literal quotes/escaping, encoded display suffix grammar and consistency
between two native reads remain unresolved native grammar objections. Exact
record recognition and lossy stderr interpretation are not a complete native
format specification. No result here closes those objections.

RNI-1 through RNI-4, E1 through E6, RF-1 including its signal follow-up,
resolver accumulation and historical N1 through N7/P1 through P7 keep their
recorded dispositions. This review adds no independent execution evidence for
those areas. Specifically, retained credential/lifecycle ownership and the
fresh preparation/dependency/network gate ordering remain required. Path alias
refusal still needs APFS evidence. A launchd presence snapshot cannot settle an
uncertain mutation; attempted files and recovery backups must survive it.
Installer locks do not settle outstanding service requests after client exit.

All recorded limits remain: direct-child ownership does not prove arbitrary
descendant completion; trusted dependencies must stay foreground; blocked
kernel/libc work can retain ownership and prevent shutdown; SIGKILL, runtime
destruction and OS teardown defeat in-process retention. SMB passwords remain
in helper argv until exit. Final path-based operations still permit same-user
rename/external-mount races; conservative Unicode comparisons can reject
distinct paths. A configured TCP service gate does not certify NFSv3 ancillary
RPC services. Retained uninstall behavior after plist-removal failure does not
promise new rollback semantics. No general heap/RSS bound follows here.

Swift's helper deadline starts after executable lookup and launch. This review
establishes no bound on those operations. A permanently blocked NSWorkspace
call retains one Reveal worker and prevents another Reveal until completion or
app restart. Existing helper/Foundation and process test gaps remain outside
this assignment. Round 2 is incomplete because of the stopped refused scope;
this status review cannot close it or establish two consecutive clean rounds.

## Native acceptance still required

Use only a separately authorized disposable macOS 13+ account or VM and a
noninstalled fixture build. Record OS, architecture, source commit, exact
synthetic responses and outcomes. These checks have not run here.

1. Typecheck/build the real Swift sources. Feed actual Foundation decoding all
   sixteen current states, omitted optional fields, nullable details, unknown
   kind, extra fields and malformed required values. Include old responses
   without state_kind. Confirm the null-kind asymmetry explicitly if testing
   interoperability with any producer that emits it.
2. With a synthetic mountctl data source, observe valid mounted, mixed degraded,
   unknown, empty, malformed, absent and valid-again results. The error state
   must clear after recovery; empty success must show no mounts configured;
   failed/malformed data must not become a successful empty result. These are
   data/UI checks, not new helper deadline or process tests.
3. Hold the menu and submenu open while responses change repeatedly, including
   valid to invalid to valid. Require stable row identity and keyboard focus
   while tracking, a final rebuild after close and the latest icon/label. Check
   both mouse and keyboard selection at the close/action boundary.
4. Capture synthetic action arguments and Reveal's disposable local path while
   later responses reorder rows or change the same name's path. Activation must
   use the selected row's captured value. Do not invoke a real mount/unmount or
   follow a production mountpoint. The round-1 option-name acceptance remains
   separate prior coverage.
5. Use VoiceOver and Accessibility Inspector for each aggregate state and row
   kind, including unknown/empty/error recovery. Confirm status-button and row
   name/state labels, readable explanatory text, submenu action announcements,
   keyboard activation and focus preservation through deferred refresh.

The wider Mac checklist still governs native linking, Darwin peer credentials,
IOKit, zsh/APFS locks and path spelling, launchd ordering/recovery and daemon
exit policy. Disposable Keychain and real-share checks need their own
authorization. No Linux fixture or Apple cross-check settles these runtime gaps.

## Owned files

- `docs/audit/round3-status-menu-review.md`
- `tests/audit/round3-status-menu.py`
- `tests/audit/round3-status-menu.rs`
- `tests/audit/round3-status-menu-suite.log`
- `tests/audit/round3-status-menu-initial-suite.log`
- `tests/audit/round3-status-menu-source-sha256.txt`
- `tests/audit/round3-status-menu-checks.log`
