#!/bin/bash
# Build and synthetic checks only. No install, daemon run or Keychain integration.
set -uo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
    echo "This verification requires macOS." >&2
    exit 1
fi
cd "$(dirname "$0")/.." || exit 1
export TMPDIR="$HOME/.local/share/agent-tmp"
mkdir -p "$TMPDIR" || exit 1
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1
artifacts="$(mktemp -d "$TMPDIR/mountd-mac.XXXXXX")" || exit 1
failed=0
check() {
    local name="$1"
    shift
    if "$@" >"$artifacts/$name.log" 2>&1; then
        printf '%s pass\n' "$name" | tee -a "$artifacts/results.txt"
    else
        printf '%s fail\n' "$name" | tee -a "$artifacts/results.txt"
        failed=1
    fi
}
git rev-parse HEAD >"$artifacts/revision.txt"
git diff --binary >"$artifacts/tracked-changes.patch"
git status --short >"$artifacts/worktree.txt"
sw_vers >"$artifacts/platform.txt"
uname -m >>"$artifacts/platform.txt"
check rust-version rustc --version --verbose
check swift-version swiftc --version
check format cargo fmt --check
check lint cargo clippy --locked --all-targets -- -D warnings
check rust-tests cargo test --locked
check swift-typecheck swiftc -typecheck -swift-version 5 menubar/Sources/main.swift menubar/Sources/Command.swift
check swift-commands zsh menubar/test-command.sh
check installer-fixture python3 menubar/test-build.py
check app-build zsh menubar/build.sh
check app-signature codesign --verify --deep --strict "menubar/build/Mountd Menu Bar.app"
printf 'Artifacts: %s\n' "$artifacts"
exit "$failed"
