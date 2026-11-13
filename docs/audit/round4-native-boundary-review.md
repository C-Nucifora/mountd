# Round 4 enumeration and power boundary review

Source review has found no confirmed substantive defect. The recovered focused pure-helper
suite passed all four tests, with outer exit 0. Native execution is
unavailable here. Parent owns aggregate round acceptance and integration.

## Checkout and scope

All commands use `/home/christian/Documents/dev/mountd`, not the unrelated
default course-planner directory. Origin is
`https://github.com/C-Nucifora/mountd.git`, branch `hardening-audit-rounds`.
Entry HEAD is `6c51d55a64321e6d8cbff1773ad67bb8d5467c8c`, descended from
`2224097e28815cfcba16a288a37ff22ad158058f`. The reviewed production files and
Cargo.lock have no difference between those commits. The tracking reflog records
the last branch push at `2026-11-12 11:52:14 +1000`, updating to `6c51d55`.
No hosting query was made because this assignment forbids network access.

Read the supplied instructions and ponytail, unslop and realistic-commits.
No additional ancestor or repository AGENTS.md was found. The explicit
no-commit instruction overrides commit creation and timing. Read README,
coverage, review-disposition, native, review-native-ipc, round3-policy-breaker,
round3-status-menu-review, swarm-rounds and mac-verification. Read existing
mtab and power tests and the portable-native generator as the extraction
reference. That generator was not run or modified.

Only this report and `tests/audit/round4-native-boundary*` belong to this
review. No production edits, commits, pushes, branches, worktrees, threads or
subagents were created. Other owners' artifacts remain untouched. No real
power notification, enumeration, mount, unmount, engine effect, credential,
Keychain, network, socket, LaunchAgent or installed application was used.
The test runner invokes only compiler/test tools, with no external fake helper.

The provider-refused round-2 process/drain assignment remains stopped.
Its artifact names appeared in Git status, but their contents were not read,
run, adapted or modified. No supervisor, resolver, subprocess-output,
deadline, cancellation, reaping, drain or child-topology test was undertaken.
The acknowledgement channel below is only the existing power helper's local
completion signal. It is not a replacement route for the refused task.
No part of this review was refused.

## Enumeration source review

`src/mtab.rs:28` first calls `getfsstat` with null storage, zero size and
`MNT_NOWAIT`. A negative count returns the OS error immediately. It allocates
zeroed `libc::statfs` entries for the count plus sixteen. At line 34 the fill
call again uses `MNT_NOWAIT`, with allocated byte size. A negative result
returns an error. A result at or beyond capacity returns the explicit
mount-table-change error. Only a smaller nonnegative count reaches truncation
and conversion. Zero returned entries therefore produce an empty vector.
Exact capacity is conservatively rejected, even if every record fitted.

The source uses the crate's Darwin declaration rather than a handwritten
layout. Reviewed Cargo.lock pins libc 0.2.189. Its cached Apple module declares
16-byte filesystem-name and 1024-byte mountpoint/source fields, `MNT_NOWAIT`
as 2, and `getfsstat` with an integer byte count and integer return. It selects
`getfsstat$INODE64` on non-aarch64 macOS. The reviewed module's SHA-256 is
`30b07c6607e27c676ef00fcd0b8d23001fe6222360a5406f26db06111dd047de`.
This is binding source inspection, not an SDK ABI or linking test.

`cstr_field` at line 15 iterates only the supplied slice, stops at its first
zero and casts each character back to a byte before lossy UTF-8 conversion.
It does not form an unbounded C string, dereference beyond a field, resolve a
path or stat a mountpoint. An unterminated field consumes at most its width.
Invalid UTF-8 is replaced; different invalid byte sequences can therefore
produce the same string. This suite tests that stated conversion behavior,
not byte-preserving native path identity.

The two calls are not an atomic mount-table snapshot. Sixteen spare records
and refusal at capacity do not prevent later table changes. The allocation
size is narrowed to `c_int` without a checked conversion. An enormous reported
count could exceed that representation or available memory, but this review
has no native count evidence or existing supported-scale requirement that
establishes a reachable defect. No invented `getfsstat` implementation was
used to produce such a finding. Native count/fill semantics, concurrent table
changes, real field encoding and dead-share nonblocking behavior remain open.

## Power source review

`src/power.rs:166` creates an unbounded event channel and starts a dedicated
thread. That thread allocates the context on the heap, converts its Box to a
stable raw pointer, and passes it with port/notifier output pointers and the
actual C callback to `IORegisterForSystemPower`. The output variables live
through the registration call. The context initially contains root port zero.
If the returned root port is zero or notification port is null, the code logs
fallback, frees the context and returns. On success it writes the returned
root port before obtaining and adding the run-loop source, then runs the loop
on the same thread in common modes.

Success deliberately retains the context and registration for process lifetime.
Returning from the run loop does not free callback storage. The root-port write
assumes no callback runs during registration or before the source is added.
The failure branch assumes neither failure indication leaves an active callback
registration. `IONotificationPortGetRunLoopSource` has no explicit null check;
`IOAllowPowerChange` return statuses are ignored. These are native contract
dependencies and source-only concerns, not demonstrated failures. Native
headers/runtime evidence is needed to decide whether partial registration,
null-source or rejected-ack cases are reachable. A stubbed registration could
invent those semantics and would not settle the question.

Dispatch at line 95 casts `message_argument` to an `isize` token without
dereferencing it and reads the registered context. Its source branches are:

| Message | Source behavior |
| --- | --- |
| CanSystemSleep, `0xE0000270` | Calls `IOAllowPowerChange` immediately with this context's root port and the supplied token. No sleep event or veto. |
| SystemWillSleep, `0xE0000280` | Calls `acknowledge_sleep` with fixed MAX_ACK_WAIT and a closure containing one `IOAllowPowerChange` call for this token. |
| SystemWillNotSleep, `0xE0000290` | Sends DidWake, allowing cancelled-sleep recovery. No permission acknowledgement. |
| SystemHasPoweredOn, `0xE0000300` | Sends DidWake. No permission acknowledgement. |
| Other message | No action. |

The module's broad introductory phrase about every notification is qualified
by N7 and the actual permission-bearing cases above. This review does not
invent an acknowledgement requirement for wake, cancellation or unknown
messages. The hardcoded constants and handwritten IOKit/CoreFoundation types
still require native SDK and runtime validation. Their existing unit assertions
repeat numeric constants and cannot provide that validation.

At line 124 each helper invocation allocates a fresh oneshot channel, sends
one WillSleep event and waits only if event delivery succeeds. Completed and
dropped acknowledgement senders both finish the wait. Timeout drops this
invocation's receiver. A failed event send skips waiting. The final FnOnce
closure runs once on each of these ordinary return paths. No native token,
deadline or receiver is cached between invocations.

The callback supplies 25 seconds and the helper clamps any supplied budget to
that same constant before computing its monotonic deadline. It does not capture
the reloadable cleanup budget, preserving RNI-4's separation. The polling loop
sleeps in 20 ms steps. Therefore 25 seconds bounds the requested wait, not
strict wall-clock callback completion: polling can overshoot and scheduling or
the native allow call can add delay. This review neither claims a hard
real-time guarantee nor uses a real 25-second wait as native acceptance.
Actual acknowledgement latency and IOKit's return status remain required.

## Four new portable tests

Existing power tests cover individual successful, dropped, closed and timed-out
waits. Existing mount-table tests call native enumeration or parse source names.
The new matrix adds fixed-field extremes and repeated-call independence.

| Test | New cases and oracle |
| --- | --- |
| `every_nul_position_and_full_fields_remain_bounded` | 1045 cases across widths 0, 1, 16 and 1024. Every possible first-NUL position and a fully unterminated field must produce exactly the ASCII prefix. Invalid bytes after NUL must be ignored. |
| `byte_values_and_utf8_edges_obey_first_nul` | All 256 byte values plus ten explicit multibyte, truncated, overlong and embedded-NUL cases. Literal expected strings check byte casting, first-NUL truncation and documented lossy replacement. |
| `expired_tokens_cannot_complete_successive_notifications` | Four zero-budget expirations each precede a fresh notification. Sending the previous expired acknowledgement fails while the current receiver remains open. Two current senders complete and two drop. Exactly eight distinct synthetic allow labels arrive in order, with no extra events or allow calls. |
| `closed_event_channel_repeats_do_not_reuse_queued_ack` | Close after one queued expired event, either retain the closed receiver or drop it with the event queued. Sixteen subsequent calls per variant use zero, 1 ms, 25-second and maximal Duration budgets. They must bypass waiting and invoke allow once each, with no queued replacement event. These large budgets exercise closed-send bypass only, not the live-channel cap. |

The token labels belong to synthetic closures, not IOKit notification IDs.
The live-channel waits use 150 ms budgets and a one-second test coordination
bound. No engine or unmount logic is compiled. Matrix rows are not additional
test functions. No negative control or failing regression is claimed because
no production fix or confirmed defect is proposed.

## Extraction, execution and cleanup

`round4-native-boundary.py` checks exact SHA-256 values for both production
files and Cargo files before and after execution. It copies exact byte ranges
for cstr_field, PowerEvent, MAX_ACK_WAIT, acknowledge_sleep and its blocking
receive trait/implementation. Every range's offset, length and hash is logged.
Only the platform cfg outside cstr_field is excluded so the unchanged function
can compile on Linux. Imports and new tests surround those exact bytes.
No old tests, native binding, callback, watcher or enumeration function enters
the generated crate. Nothing replaces IOKit, CoreFoundation or getfsstat.

The crate has a distinct package identity and an exclusive temporary directory
under disk TMPDIR, including its own target directory. It uses only libc and
Tokio's sync feature, with pin-project-lite transitively. The minimal lockfile
retains exact reviewed registry versions, sources and checksums. Cargo runs
offline and locked; the runner checks unchanged lock bytes and dependency
identity afterward. This validates the pure helpers with the reviewed crates,
not the production feature set or Darwin c_char/statfs ABI.

One invocation, from the explicit mountd root:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 3600 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round4-native-boundary.py \
  > tests/audit/round4-native-boundary-suite.log 2>&1
native_status=$?
printf 'outer_exit=%s\n' "$native_status" >> tests/audit/round4-native-boundary-suite.log
exit "$native_status"
```

Under that lock the runner records compiler versions and invokes exactly:

```sh
cargo test --offline --locked --lib -- --test-threads=1 --nocapture
```

Execution is bounded at 235 seconds with five seconds of termination grace;
lock acquisition is bounded at 3600 seconds with exit 75. No second suite,
parallel heavy job or expired-lock retry is planned. The runner removes only
its own scratch after successful Cargo exit, and retains inactive scratch on
test failure. The recovered log records all four tests passing and scratch cleanup after Cargo exited.

## Preserved objections and native follow-up

E1-E6, RNI-1 through RNI-4, RF-1 and its signal follow-up, resolver accumulation,
historical N/P findings, round-1 fixes and R2-CRED-1 keep their existing
dispositions. Earlier reports and failing evidence remain untouched. This
review adds pure acknowledgement evidence to N7/RNI-4 but cannot close actual
power delivery, cancelled-sleep ordering or native acknowledgement acceptance.

All prior native limits remain open: Darwin credentials and enumeration, APFS
case/Unicode spelling, native option and mount-source formatting, Keychain
grammar and five ignored checks, zsh locks, launchd ordering/recovery and exit
policy, Swift/Foundation, AppKit focus and accessibility. The recorded direct
child ownership, blocked kernel/libc work, OS teardown, SMB argv visibility,
same-user pathname races, conservative Unicode rejection and NFS ancillary
service limits remain unchanged. Existing helper, installer signal and refused
process gaps are not taken over by this assignment.

Proposed additions for parent integration into the Mac checklist, all unrun:

1. Record supported OS/architecture and compile/link the real bindings. Check
   the SDK's registration failure contract, callback scheduling, null-source
   guarantees and notification-token type against this source. Observe actual
   root/notification port and source validity before the first callback.
2. With a separately authorized disposable native power fixture, correlate
   each permission-bearing message, root port, notification token, allow-call
   time and return status. Include repeated idle/forced/cancelled sleep and
   wake. Use synthetic local completion signals only. A late prior sender
   must not complete the next event; a closed receiver must still allow sleep.
   Record queued-notification order while a previous callback is waiting.
3. Measure actual maximum acknowledgement latency under native scheduling.
   Separate the 25-second requested wait from callback entry-to-allow timing
   and OS acceptance. Test the existing reload-budget cases on Mac as already
   specified in mac-verification, without treating this portable suite as
   engine or sleep acceptance.
4. Record real getfsstat count/fill results, returned field bytes and limits
   on supported macOS versions. Check normal root enumeration and native
   behavior while the table changes. Dead-share timing and real share setup
   remain separately authorized checklist work. Do not substitute fake native
   return values for that evidence.

No Mac checklist or aggregate ledger was edited. Completed source review,
passing pure execution and unrun native acceptance remain separate categories.
No aggregate clean-round count follows from this report.

## Owned files

- `docs/audit/round4-native-boundary-review.md`
- `tests/audit/round4-native-boundary.py`
- `tests/audit/round4-native-boundary.rs`
- `tests/audit/round4-native-boundary-suite.log`

Only these files and this runner's disposable scratch were created by this
review. Parent retains integration and any publication decisions.

## Owner final check

On 2026-11-13 the owner reviewed the fixture and recovered passing log. One
bounded rerun used a 45-second heavy.lock wait and the same 235-second execution
limit. It exited 75 before execution; see round4-native-boundary-final-suite.log.
There was no lock retry. This does not invalidate the recovered four-test pass
but supplies no fresh runtime result. Rust formatting and Python syntax pass.
