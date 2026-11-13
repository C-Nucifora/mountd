# Round 4 pure configuration and mount-source identity review

Execution blocked before tests. Source review has not established a new substantive defect.
The parent owns integration and the round-level disposition. This review does
not establish a completed global clean round or native acceptance.

## Checkout and scope

Reviewed `/home/christian/Documents/dev/mountd`, origin
`https://github.com/C-Nucifora/mountd.git`, branch `hardening-audit-rounds`, at
`6c51d55a64321e6d8cbff1773ad67bb8d5467c8c`. Verified `2224097` is an ancestor.
The relevant tracking reflog records the latest push at
`2026-11-12 11:52:14 +1000`, pointing to `6c51d55`. No hosting request was made
because the task prohibits network access. No local commit was ahead of the
tracking ref at entry. Every repository shell command used the explicit mountd
working directory, never the unrelated default project.

Read the supplied instructions and ponytail, unslop and realistic-commits.
No additional AGENTS.md was present in the repository, applicable subdirectories
or ancestor paths. The explicit no-commit instruction overrides commit creation
and timing. Read README, coverage, review-disposition, ipc-config, process,
round3-policy-breaker, round3-status-menu-review, round2-credential-review,
swarm-rounds and mac-verification. Compared the existing source tests and
round2-credential-path-mount and round3-policy cases before choosing this suite.

Only this report and `tests/audit/round4-identity*` belong to this reviewer.
No production edit, commit, push, branch, worktree, top-level thread or child
agent was created. Other-owner files were preserved. No production config,
share, mount table, native API, Keychain, resolver, network or mount helper was
accessed. Generated argv values were strings only.

## Contracts and source reasoning

- `src/config.rs:630` requires an endpoint to be a bare hostname or IP address,
  with the stated purpose that probe and mount identify the same host. It rejects
  authority delimiters, whitespace/control bytes, zone markers, brackets,
  non-IPv6 colons and leading flags. Valid address strings are not case-folded,
  resolved or canonicalized. Endpoint order remains the TOML array order.
- `src/mountcmd.rs:58` percent-encodes each SMB component's UTF-8 bytes.
  `url_host` at line 73 brackets IPv6; `smb_source` at line 86 composes the URL.
  `mount_argv` at line 204 uses that producer for SMB and brackets the NFS host
  before appending the absolute export unchanged. No shell interpretation is
  involved in these pure builders.
- `src/mtab.rs:77` documents `//user@host/share` and `host:/export` source forms.
  Its existing tests explicitly cover bracketed SMB and NFS IPv6 and the
  negative local-device/empty cases. The parser takes the authority before the
  first slash, removes the last userinfo separator, trims the NFS colon and
  removes paired brackets. On producer-generated IPv6, closing brackets protect
  significant trailing address colons, including `::` and `2001:db8::`.
- Engine bookkeeping at `src/engine.rs:439` obtains the actual source address
  each pass. A changed value clears health/migration evidence. That sequence
  behavior already belongs to round 3 and is not repeated here. Health uses the
  parsed value. Migration at line 570 finds its rank through exact string
  equality and returns no target when attribution is absent or unconfigured.
  This review tests the producer/parser values fed to that equality expression;
  it does not execute the async engine or claim new policy coverage.
- `find_by_mountpoint` at `src/mtab.rs:73` requires a previously normalized path.
  Engine `mountpoint_of` at line 249 performs that normalization. Matching strips
  trailing slashes only. It must not select a case/Unicode alias, parent or
  neighboring prefix. Conservative alias refusal belongs to separate preparation
  and config checks; it is not a license to fold the mount-table lookup.

The contract does not define a complete grammar for every native source string.
`source_address` is not a general validating URL parser: arbitrary nonempty
relative text can become an address. No oracle here assumes rejection of every
unsupported native spelling. Config authority rejection and the existing
local-device/empty negatives are tested where the contract does require them.
The broad malformed-metadata fail-closed requirement from round 2 applies to
credential metadata and is not transferred to mount-table strings.

Exact equality does not promise DNS equivalence, canonical IPv6 spelling or
hostname-to-address attribution after a native helper rewrites the source.
Duplicate endpoint address strings also select the first equal rank. The
configuration has no unique-endpoint-address requirement, and a source host
alone cannot distinguish two entries with the same address. These observations
supply no new deterministic contract violation. Native rewriting remains open.

## Focused evidence

Five new tests in `tests/audit/round4-identity.rs` compose unchanged functions.
They do not rebrand the prior option matrix, path preparation or external-source
replacement tests.

| Test | New oracle and cases |
| --- | --- |
| `smb_config_to_argv_to_address_preserves_identity` | 864 combinations of eight accepted endpoint spellings, six username forms, six share strings and three synthetic password choices. Config preserves the endpoint; argv and direct SMB producer agree; parser returns the exact endpoint. Delimiters remain inside encoded components. One independent exact-byte example checks percent-encoding and UTF-8. |
| `nfs_config_to_argv_to_address_preserves_identity` | 64 combinations of the same hosts and eight absolute export strings. Includes root, trailing/repeated slashes, spaces, punctuation, literal percent bytes and Unicode. Exact source bytes retain the export while address extraction returns the original host. |
| `authority_syntax_is_rejected_at_configuration_boundary` | 24 invalid authorities under both protocols, 48 rejections. URI prefixes, userinfo, paths, ports, brackets, zone markers, malformed IPv6, whitespace, controls and flags cannot enter the builder through validated config. Four empty/local-device source strings have no address. |
| `endpoint_order_and_mount_names_preserve_exact_values` | Six permutations of three endpoints under both protocols, 36 generated-source rank checks. Five case, whitespace and composed/decomposed Unicode mount names preserve exact lookup identity and config order with separate ASCII mountpoints. No wire-size or CLI parsing checks are repeated. |
| `normalized_mountpoints_match_only_the_exact_entry` | 192 single-entry comparisons over eight paths and three trailing-slash forms, plus multi-entry selection. Root, neighbors, descendants, case variants, Unicode variants and trailing spaces remain distinct. Three config/normalizer/argv/matcher compositions cover bare names and repeated separators. Seven invalid empty/root/dot/home paths fail config validation. |

These are bounded matrix rows, not unique Rust test-function counts. Encoded
share/user punctuation and unusual NFS export text prove byte handling only.
They do not establish which names a native server or helper accepts. All names,
addresses and paths are synthetic; tests never open those paths.

## Fixture fidelity and execution

The generator copies the complete unchanged config module before its old test
module. It extracts unchanged MountEntry, same_path, find_by_mountpoint,
source_address, all pure mountcmd functions from expand_tilde through
mount_argv, the two conservative lexical comparison helpers, and WolSpec with
its default functions. WolSpec is needed to deserialize the actual Endpoint
type; no wake/network implementation is included. Imports and module wrappers
are fixture glue. No native API behavior is replaced to make Linux pass.
Existing source test modules are excluded, so only the five new tests run.
Engine is reviewed and hashed but not linked into the crate.

The distinct package is `mountd-round4-identity-review-fixture`. Before
extraction the generator verifies the source bytes against `6c51d55`, the remote
and branch. It seeds Cargo.lock from the reviewed lock with only the root package
renamed. Offline Cargo metadata prunes unused dependencies, then every resulting
registry name, version, source and checksum must match the reviewed lock before
compilation. The test invocation uses `--offline --locked`, and the lock and
production bytes are checked again afterward. The log records every dependency,
source/extraction/generated-file hash and exact command. Scratch is retained;
no active directory is deleted.

One acquisition and one focused Cargo test, explicit cwd
`/home/christian/Documents/dev/mountd`:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 3600 -E 75 /home/christian/.local/state/agent-swarms/heavy.lock \
  timeout -k 5s 235s python3 -u tests/audit/round4-identity.py \
  > tests/audit/round4-identity-suite.log 2>&1
identity_exit=$?
printf 'outer_exit=%s\n' "$identity_exit" >> tests/audit/round4-identity-suite.log
exit "$identity_exit"
```

The lock wait is bounded at 3600 seconds with exit 75; execution is bounded at
235 seconds plus five seconds of termination grace. The timeout owns the Cargo
command group. TMPDIR is on disk, build jobs and test threads are one, and
incremental compilation is disabled. The shared `target/portable-daemon` cache
is used only under the lock. No parallel heavy job, repeated acquisition retry,
general fuzz load or broad suite is started by this reviewer.

The recovered worker log records exit 1 before tests: offline Cargo metadata
could not download `getrandom v0.2.17`. No identity test passed or failed.
The owner attempted one bounded final run on 2026-11-13, with the same command
but a 45-second lock wait and a separate `round4-identity-final-suite.log`.
It exited 75 without acquiring heavy.lock. There was no retry or dependency
change. The five tests remain unexecuted. Formatting, Python syntax and the
source hash manifest passed owner verification.

## Source hashes

The complete manifest also includes the generator and Rust fixture:
`tests/audit/round4-identity-source-sha256.txt`.

```text
d8c0c95ae6ff5f0f96dd145bce6f076f0119df01b1a0666113bd924669a78094  src/config.rs
aab40c5d1ef589b85e8ee525e961ba8a66c6f26a0fae9a6e74b3e5828ef93a21  src/mtab.rs
5ec31492c6a9b23dd06fca046db8f3a39077706694ae80159765a954bf8dbef4  src/mountcmd.rs
5d60f728b38039f1cbc3ec525fe8e657efbe1699761995dbb43bb5e58e121dd7  src/engine.rs
136eaa29ea9b0af777cd7d86239765a5e7f836a0aa88a7aefdf4ffaedc466bb0  src/wol.rs
96600607dafc9fdf9131da60b851787f9611fcd38f41b040334ae838a4da1d3c  Cargo.toml
6408d5a07ea8c80acd23da7efcdeff9165c474625964420ed1fd2abc85cec3e3  Cargo.lock
```

## Prior objections and limits

Round 1's four corrected findings and their independent challenges retain their
accepted portable dispositions. R2-CRED-1 remains fixed and independently
accepted. Literal quote/escape grammar, encoded display suffixes, consistency
between native credential reads and the five ignored Keychain tests remain
open. This review adds no credential result.

Round 3's two reviews found no substantive issue in their exercised portable
scope. Their source-replacement, reordered-reload, policy and status/menu evidence
is not counted again here. E1-E6, RNI-1 through RNI-4, RF-1 and its signal
follow-up retain their prior dispositions. Historical ipc-config/process text
allowing proto=tcp or proto=tcp6 is superseded by the current E2 rejection and
source. This suite uses default options and does not reopen that option audit.

Native mount-source formatting, source truncation/encoding, getfsstat ABI and
nonblocking behavior, hostname/IP canonicalization and actual SMB/NFS source
attribution remain unrun. Raw-byte fixtures prove none of those native facts.
APFS case/Unicode interpretation and preparation races remain open; synthetic
string differences are not filesystem identity evidence. Darwin peers, IOKit,
launchd/zsh, Swift/AppKit/VoiceOver, native option semantics and real-share
behavior remain under mac-verification. The existing SMB argv exposure,
pathname race, descendant/kernel/libc ownership and NFS ancillary-service
limits are unchanged.

The separate round-2 process/drain task was provider-refused and stays stopped.
No `tests/audit/round2-process*` content was read, run, adapted or modified.
Git status and file listing exposed filenames only. No subprocess deadline,
output, cancellation, child lifetime, drain or resolver test was added or run.
No refused scope was reassigned or retried, and no route was switched. No portion
of this pure-identity review was refused.

## Owned files

- `docs/audit/round4-identity-review.md`
- `tests/audit/round4-identity.py`
- `tests/audit/round4-identity.rs`
- `tests/audit/round4-identity-suite.log`
- `tests/audit/round4-identity-source-sha256.txt`
- `tests/audit/round4-identity-checks.log`
