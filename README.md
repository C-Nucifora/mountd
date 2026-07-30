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
  `soft,intr,deadtimeout=60,retrycnt=0` so the kernel reaps dead mounts
  itself; SMB mounts with `mount_smbfs -N` (credentials from the login
  Keychain, never an invisible auth prompt).
- **Escalating unmount** — `diskutil unmount` → `umount` → `umount -f`.
- Fixed user-owned mountpoints under `~/mnt` — no `/Volumes/share-1` drift.

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
Allow". Trade-off to know about: the password is briefly visible to local
`ps` during the mount syscall (it is never written to disk, and is redacted
from logs and error messages).

## CLI

```
mountctl status [--json]    # state of every mount (JSON feeds SwiftBar etc.)
mountctl explain [name]     # why is/isn't this mounted: conditions, endpoint
                            # eligibility, live probe results
mountctl mount <name>       # force an attempt (ignores `when`, honors probes)
mountctl unmount <name>
mountctl reload             # re-read config
mountctl poke               # trigger a reconcile pass now
mountctl install|uninstall  # manage the LaunchAgent
```

```
$ mountctl explain nas-media
nas-media: not mounted — decision: Attempt
  when any_of:
    ✗ network:home
    ✓ dep:tailscale
  endpoints (priority order):
    192.168.1.10  skipped (network:home not met)
    100.64.0.10   eligible, probe ok
```

## Status / roadmap

v0.2. Working: config, fingerprinting, dep checks, probe-gated ordered
endpoints, reconcile loop, IPC + CLI, LaunchAgent install, event-driven
wakeups (routing-socket monitor feeding the debounced reconciler, with the
poll as a safety net), wake-from-sleep detection (monotonic vs wall-clock
gap), and `mountctl explain`. Planned:

- Pre-sleep clean unmount via IOKit power notifications (wake is already
  detected; the willSleep window needs IOKit registration)
- Health loop with child-process liveness probes and a SUSPECT state
- Opt-in migration to a better endpoint when one appears
- Wake-on-LAN

## License

MIT
