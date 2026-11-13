#!/bin/zsh
# Run only synthetic subprocesses; no daemon, shares, or credentials.
set -euo pipefail
cd "$(dirname "$0")"
fixture_dir="$(mktemp -d)"
trap 'rm -rf "$fixture_dir"' EXIT
cat > "$fixture_dir/main.swift" <<'SWIFT'
import Foundation

func check(_ value: @autoclosure () -> Bool, _ message: String) {
  precondition(value(), message)
}
let success = runCommand("/usr/bin/printf", args: ["synthetic output"], timeout: 1)
check(success.0 == Data("synthetic output".utf8) && success.1 == nil, "stdout")
let failure = runCommand("/bin/sh", args: ["-c", "printf 'synthetic failure' >&2; exit 7"], timeout: 1)
check(failure.0 == nil && failure.1 == "synthetic failure", "stderr")
let missing = runCommand("/no/such/synthetic-command", args: [], timeout: 1)
check(missing.0 == nil && missing.1 != nil, "launch failure")
let start = Date()
let timeout = runCommand("/bin/sh", args: ["-c", "trap '' TERM; exec /bin/sleep 5"], timeout: 0.1)
check(timeout.0 == nil && timeout.1?.contains("timed out") == true, "SIGTERM-resistant timeout")
check(Date().timeIntervalSince(start) < 2, "deadline")
let flood = runCommand("/usr/bin/yes", args: ["synthetic"], timeout: 2)
check(flood.0 == nil && flood.1?.contains("limit") == true, "output bound")
print("command fixtures passed")
SWIFT
swiftc -swift-version 5 Sources/Command.swift "$fixture_dir/main.swift" -o "$fixture_dir/command-tests"
"$fixture_dir/command-tests"
