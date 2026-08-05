# mountd

Condition-aware, probe-gated network share mount manager for macOS.

`mountd` mounts your NFS/SMB shares when — and only when — it makes sense to:
on the right network, after your VPN is actually up, and never before the
file server answers on its service port. Each logical mount can list several
addresses in priority order (LAN IP first, Tailscale IP second, …) and mounts
via the first one that is reachable.

Built as a replacement for tray-style auto-mounters that fire mounts on a
timer and hang Finder when the server isn't there.

## Why

Existing tools miss one or more of these:

1. **Conditional mounts** — mount only when a condition holds (specific
   network, dependency up).
2. **Dependency gating** — wait for something like Tailscale to connect
   before even considering a mount.
3. **Probe gating** — never *attempt* a mount until a TCP connect to the
   service port (445/2049) succeeds. A port probe is a rehearsal of the
   mount's own first syscall; ICMP ping is never used (it false-positives
   when the host is up but the service is down).
4. **Ordered address priority** — one logical mount, several endpoints:
   try the LAN IP first, fall back to the Tailscale IP, then MagicDNS.
   The daemon tracks "mount X, currently realized via address Y" instead of
   string-matching the mount table against a single configured address.

## Design

- **Per-user LaunchAgent** (never a daemon: root-context mounts produce
  root-owned volumes and can't reach the login Keychain), `RunAtLoad` +
  `KeepAlive` — login trigger and crash-restart for free.
- **Network identity by fingerprint** (subnet + gateway MAC), not SSID —
  SSID is unreadable without a Location-authorized app bundle on modern
  macOS, and the gateway MAC works on both Wi-Fi and Ethernet with zero
  permission prompts.
- **Reconciler model** — network changes are *hints* that schedule a
  debounced pass; only a fresh successful probe permits a mount attempt.
  Mount enumeration uses `getfsstat(MNT_NOWAIT)`, which cannot block on a
  dead mount.
- **Soft mounts by default** — NFS gets
  `soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp` and SMB gets `-o soft`, so a
  server that disappears makes file operations fail instead of blocking
  Finder. SMB uses `mount_smbfs -N` so it never raises an invisible auth
  prompt; the daemon fetches the Keychain credential itself (see below).
- **Health loop** — the mount table is built from `getfsstat(MNT_NOWAIT)`,
  which never contacts the server and so can never tell you a mount has died.
  Each mounted share's endpoint is re-probed on an interval; after
  `health_failures` consecutive failures the dead mount is torn down and
  rebuilt, via a lower-priority endpoint if the current one is gone.
- **Unmounting requires sustained evidence** — a mount is only removed once
  `when` has stayed false for `unmount_grace_secs`, and never within
  `wake_settle_secs` of waking, when the network has not come up and every
  condition reads false. Policy unmounts are polite (`diskutil unmount` →
  `umount`); the forceful rungs (`diskutil unmount force` → `umount -f`) are
  reserved for an explicit `mountctl unmount` and for a server the probe has
  proved is gone.
- **Clean unmount before sleep** — IOKit power notifications, bounded by
  `sleep_unmount_budget_secs`; the acknowledgement is always sent, so a wedged
  share can never hold the system awake.
- Fixed user-owned mountpoints under `mount_root` (default `~/mnt`) — no
  `/Volumes/share-1` drift.

## Install

```sh
cargo install --path .
mountd check                 # validate ~/.config/mountd/config.toml
mountctl install             # write + bootstrap the LaunchAgent
mountctl status              # see what the daemon thinks
```

## Configuration

`~/.config/mountd/config.toml` — see [`examples/config.example.toml`](examples/config.example.toml).

```toml
[networks.home]
subnet      = "192.168.1.0/24"
gateway_mac = "aa:bb:cc:dd:ee:ff"   # `arp -n <router-ip>`; pins the identity

[deps.tailscale]
cmd       = "/Applications/Tailscale.app/Contents/MacOS/Tailscale"
args      = ["status", "--json"]
json_path = "BackendState"
equals    = "Running"

[[mount]]
name       = "nas-media"
protocol   = "smb"
share      = "media"
username   = "me"
mountpoint = "~/mnt/nas-media"

  [mount.when]                        # when should this mount exist?
  any_of = ["network:home", "dep:tailscale"]

  [[mount.endpoint]]                  # priority order, first reachable wins
  address = "192.168.1.10"            # LAN — fastest path
  only_if = "network:home"            # don't even probe this off-LAN

  [[mount.endpoint]]
  address          = "100.64.0.10"    # via Tailscale
  requires         = "dep:tailscale"  # wait for the tailnet before probing
  probe_timeout_ms = 4000             # relayed worst case
```

Semantics:

- `when` decides whether the mount should exist (the reconciler unmounts
  when it stops holding, unless `policy.unmount_when_gone = false`).
- `deps` / `requires` decide when an attempt may *start*.
- Endpoint order plus the probe gate decide *how*. The gate defaults to the
  protocol's service port (`tcp:445` for SMB, `tcp:2049` for NFS).

### SMB credentials

Store the share credential in the login Keychain once (connect via Finder
⌘K and tick "Remember", or
`security add-internet-password -r "smb " -s <server> -a <user> -w`).

At mount time the daemon fetches the password from that item via
`/usr/bin/security` and passes it in the mount URL — `mount_smbfs -N` alone
does *not* consult the Keychain (that's a NetFS/Finder feature; `-N` only
reads `~/.nsmbrc`). Set `keychain_server` on a mount so one credential item
covers every endpoint (LAN IP, tailnet IP, hostname); it defaults to the
endpoint address being mounted. The first read of an item created by
another app triggers a one-time macOS permission prompt — click "Always
Allow".

If the credential cannot be read the mount **fails** rather than silently
retrying without one: against a server with guest access, a passwordless
mount succeeds as nobody and nothing anywhere says so. `mountctl status`
names the reason (no Keychain item, prompt not answered, read denied). Set
`policy.allow_passwordless` if you genuinely want anonymous mounts.

Trade-off to know about: the password is visible to local `ps` for the
lifetime of the mount attempt — bounded at 15 seconds, not merely "during
the syscall" — and again on each retry. It is never written to disk, and is
redacted from logs and error messages in both its encoded and raw forms.

One macOS quirk worth knowing: `security -w` returns the password
hex-encoded whenever it contains anything outside printable ASCII (an
accented letter, an emoji, a tab). mountd detects that and decodes it, using
`security -g` to tell a genuinely-encoded value apart from a password that
merely looks like hex. Before v0.4 any such password produced
`Authentication error` with no explanation.

## CLI

```
mountctl status [--json]    # state of every mount (JSON feeds SwiftBar etc.)
mountctl explain [name]     # why is/isn't this mounted: conditions, endpoint
                            # eligibility, live probe results
mountctl mount <name>       # force an attempt (ignores `when`, honors probes)
mountctl unmount <name>     # unmount and stay unmounted until conditions move
mountctl reload             # re-read config
mountctl poke               # trigger a reconcile pass now
mountctl install|uninstall  # manage the LaunchAgent
```

```
$ mountctl explain nas-media
nas-media: mounted — decision: Keep
  when any_of:
    ✗ network:home
    ✓ dep:tailscale
  state: mounted via 100.64.0.10
  endpoints (priority order):
    192.168.1.10  skipped (network:home not met)
    100.64.0.10   eligible, probe ok <- in use
```

## Menu bar app

`menubar/` contains a native AppKit menu bar client (no Xcode project, no
third-party dependencies — a single Swift file built with `swiftc`):

```sh
cd menubar && ./build.sh install   # builds, ad-hoc signs, installs to
                                   # ~/Applications, registers a login item
```

The icon reflects aggregate state (drive+checkmark = mounted, triangle =
a mount stopped responding, exclamation = error, question mark = daemon
unreachable). The menu shows each mount with its live state and offers Mount
Now / Unmount / Reveal in Finder per mount, plus Reconcile Now, Reload
Config, and Edit Config. It talks to the daemon through `mountctl`, so it
needs no extra permissions or entitlements.

Every `mountctl` call is off the main thread with a hard deadline, and
Reveal in Finder never touches the mountpoint on the main thread — a share
whose server died still looks mounted, and stat-ing it would freeze the
status item and then Finder.

## Where things live

| What | Path |
| --- | --- |
| Config | `~/.config/mountd/config.toml` |
| IPC socket | `~/Library/Application Support/mountd/mountd.sock` (mode 0600) |
| Daemon log | `~/Library/Logs/mountd.log` |
| Daemon LaunchAgent | `~/Library/LaunchAgents/org.mountd.agent.plist` |
| Menu bar LaunchAgent | `~/Library/LaunchAgents/org.mountd.menubar.plist` |
| Menu bar app | `~/Applications/Mountd Menu Bar.app` |

The menu bar app finds `mountctl` by checking `~/.cargo/bin`,
`/opt/homebrew/bin`, then `/usr/local/bin` — absolute paths only, because
launchd's `PATH` is just `/usr/bin:/bin`.

### IPC protocol

One newline-delimited JSON request per connection, one JSON response back.
Requests are tagged by `cmd`; anything else gets `{"ok":false,"error":…}`.

```sh
$ echo '{"cmd":"status"}' | nc -U ~/Library/Application\ Support/mountd/mountd.sock
{"ok":true,"status":[{"name":"nas-media","state":"mounted via 192.168.1.10",…}]}
```

`{"cmd":"mount","name":"nas-media"}`, `unmount`, `reload`, `poke`, and
`{"cmd":"explain","name":…}` (name optional) round out the set. `state` is a
human string whose prefix is stable; `state_kind` is the machine-readable
discriminant (`mounted`, `suspect`, `waiting`, `waking`, `migrating`,
`recovering`, `error`, `idle`, `unmounted`, `unknown`).

## Status

v0.4 — the roadmap is complete. Working: config, fingerprinting, dep checks,
probe-gated ordered endpoints, per-mount concurrent reconcile, IPC + CLI,
LaunchAgent install, event-driven wakeups (routing-socket monitor feeding the
debounced reconciler, with the poll as a safety net), `mountctl explain`,
plus everything that was previously planned:

- **Pre-sleep clean unmount** via IOKit power notifications, with
  monotonic-clock-gap detection kept as a fallback for wake
- **Health loop** with endpoint liveness probes, a SUSPECT state, and
  automatic recovery onto a surviving endpoint
- **Opt-in migration** to a higher-priority endpoint when one appears
  (`policy.migrate`), skipped when the share is busy
- **Wake-on-LAN** per endpoint (`[mount.endpoint.wake_on_lan]`)

Ideas, not commitments: NetFS `NetFSMountURLAsync` to keep the SMB password
out of argv entirely, and per-mount bandwidth-aware endpoint scoring.

## License

MIT
