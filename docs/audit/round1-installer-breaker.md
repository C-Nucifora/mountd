# Round 1 installer overlap

Parent reproduction against production `menubar/build.sh` at `b4f33bf`, whose
implementation is unchanged from the first audit batch.

## R1-INSTALL-1, medium: rollback erases a successful concurrent update

`tests/audit/round1-installer-overlap.py` reuses the existing installer fixture's
fake tools. Two separate build directories share one disposable HOME. The first
installer pauses at the plist write after stopping the old service and replacing
the app. A second installer then completes successfully. The first plist write
fails; its EXIT rollback stops the second install's service, removes its app and
restores the original app.

The reproduction checks the second process returned success and installed
`update-b`, then releases the first failure and checks the final binary. Actual:
`old-app`. Expected: concurrent transactions cannot undo another successful
transaction. Rejecting a conflicting installer before effects, or serializing
transactions through rollback, would satisfy that invariant.

Root cause: per-invocation staging protects backups but does not provide mutual
exclusion for the shared app, plist and service state.

Command, run under the cross-project heavy lock:

```sh
env TMPDIR=/home/christian/.local/share/agent-tmp \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1 \
  flock -w 300 /home/christian/.local/state/agent-swarms/heavy.lock \
  python3 tests/audit/round1-installer-overlap.py
```

Observed output, exit 1:

```text
second update returned success with update-b; after first rollback: old-app
AssertionError: failed concurrent installer rolled back a successful update
```

An initial invocation found a Python string-delimiter error in this new fixture;
that was corrected before the successful product-failure reproduction above.
The traceback's original fixture filename reflects the reused setup prefix,
not a failure in the original sequential fixture.

No production file was changed for this reproduction. No real app, launchd
service, signing operation, compiler, config or credential was used. Native
locking/launchd behavior still needs Mac acceptance. The separate implementation and passing regression evidence are recorded in
[the fix handoff](round1-installer-fix.md). Fresh independent post-fix review
remains deferred at the user-requested checkpoint.
