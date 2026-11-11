# Hardening baseline

Audit started 2026-11-12 Australia/Brisbane on a Linux x86_64 VPS.

## Repository

No existing mountd checkout was found under the user's home or /tmp in the initial discovery. Cloned https://github.com/C-Nucifora/mountd into /home/christian/Documents/dev/mountd. Both remote branches, main and v0.4-review-fixes, point to b39ea7e. The new checkout had no local changes. There is no repository AGENTS.md. The supplied thread instructions apply.

Read README.md, Cargo.toml, and the historical v0.4 plan under docs/superpowers/plans. The plan describes the current design and is not a new feature request. Preserve per-user LaunchAgents, probe-gated mounts, nonblocking status/UI, polite policy unmounts and fake effect boundaries.

Neither the origin/main tracking reflog nor available user GitHub PushEvents established the branch's last push. Commit dates are not push evidence. This is not coursework. Use the realistic-commits past-day fallback for reviewed local commits; do not push.

## Initial checks at b39ea7e

- `cargo fmt --check`: passed.
- `cargo test --locked`: failed during compilation on Linux. Darwin `getfsstat`, `MNT_NOWAIT` and statfs fields are unavailable; IOKit/CoreFoundation framework linkage requires an Apple target. No tests ran.
- `cargo clippy --locked --all-targets -- -D warnings`: same platform compilation blockers.
- Existing CI runs Rust checks and Swift typecheck/build on macOS. Native acceptance has not been performed here.

Any later portable tests must retain Darwin behavior and state their limits. A fake mount table cannot prove getfsstat behavior on an unreachable real share.

## VPS isolation

Initial `ss -lntup` showed existing unrelated services, including public SSH/HTTP/HTTPS and port 3773, loopback services, and Tailscale listeners. None was created or changed for this audit. Read-only nginx routing inspection found existing application proxies and no mountd route. Docker inspection failed because /var/run/docker.sock was absent; no Docker containers were created by this audit. No test, preview or debug server was started at baseline. Any later TCP fixture must bind explicitly to 127.0.0.1 and be stopped after use. Do not alter production listeners, proxy configuration or firewall rules.

## Pending acceptance inputs

A disposable macOS 13+ account or VM with Xcode Command Line Tools is required for Rust native tests, Swift typecheck/build, accessibility, real Keychain ACL behavior and sleep/wake acceptance. Real-share tests additionally require approved disposable NFS/SMB endpoints and explicit authorization. The VPS cannot reach home machines. No SSH attempt to a home machine is authorized or planned.

## Recovery after inference interruption

The four original implementation workers failed during the provider interruption. Their uncommitted changes survived in the single mountd checkout at b39ea7e. No alternate worktrees or local commits required integration. Each original worker resumed with its existing context and disjoint file ownership. No duplicate workers or top-level threads were created, and no authentication settings changed.

On the recovered tree, `cargo check --locked --target aarch64-apple-darwin --all-targets` and `cargo clippy --locked --target aarch64-apple-darwin --all-targets -- -D warnings` passed. These are intermediate compile checks, before the remaining process-ownership and shutdown fixes. They neither execute tests nor establish Mac runtime acceptance. Final checks must run again on the integrated changes.

The origin/main tracking reflog and the available GitHub user events still do not establish a last push. The past-day commit timing fallback remains applicable. Listener inspection found no mountd test service; existing unrelated listeners remain untouched. Docker inspection remains unavailable because the daemon socket is absent. No container or proxy was created for this task.
