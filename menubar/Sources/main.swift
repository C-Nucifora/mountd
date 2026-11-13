// mountd menu bar client. Renders `mountctl status --json` as an
// NSStatusItem menu with per-mount actions. Pure AppKit, no Xcode project —
// built by menubar/build.sh with swiftc.
//
// The whole point of this app is to be the thing that still works when a file
// server has vanished, so two rules govern it: nothing touches a mountpoint on
// the main thread, and every call into `mountctl` is bounded in time.

import AppKit
import Foundation

struct MountStatus: Codable, Equatable {
  let name: String
  let state: String
  let mountpoint: String
  let mounted_via: String?
  let last_error: String?
  /// Added in v0.4; older daemons omit it, so it must stay optional.
  let state_kind: String?

  /// Prefer the structured discriminant, fall back to the string prefix that
  /// v0.3 daemons emit.
  var kind: String {
    if let k = state_kind, !k.isEmpty { return k }
    for prefix in ["mounted", "error", "waiting", "suspect", "recovering", "waking", "migrating"]
    where state.hasPrefix(prefix) {
      return prefix
    }
    return "unknown"
  }
}

func expandTilde(_ path: String) -> String {
  (path as NSString).expandingTildeInPath
}

/// Locate mountctl. Only absolute candidates: under launchd the PATH is
/// /usr/bin:/bin, so a PATH search would find nothing useful anyway.
func mountctlPath() -> String? {
  let candidates = [
    expandTilde("~/.cargo/bin/mountctl"),
    "/opt/homebrew/bin/mountctl",
    "/usr/local/bin/mountctl",
  ]
  for c in candidates where FileManager.default.isExecutableFile(atPath: c) {
    return c
  }
  return nil
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
  var statusItem: NSStatusItem!
  let menu = NSMenu()
  var statuses: [MountStatus] = []
  var daemonError: String?
  /// True while a menu is being tracked, so a refresh landing mid-navigation
  /// does not rebuild the items under the user's cursor.
  var menuIsOpen = false
  var pendingRebuild = false
  var refreshing = false
  var revealing = false

  func applicationDidFinishLaunching(_ notification: Notification) {
    statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    menu.delegate = self
    statusItem.menu = menu
    setIcon(symbol: "externaldrive", description: "mountd")
    rebuildMenu()
    refresh()

    let timer = Timer(timeInterval: 15, repeats: true) { [weak self] _ in
      self?.refresh()
    }
    // .common so the poll keeps running while a menu is open.
    RunLoop.main.add(timer, forMode: .common)
  }

  func menuWillOpen(_ menu: NSMenu) {
    menuIsOpen = true
    refresh()
  }

  func menuDidClose(_ menu: NSMenu) {
    menuIsOpen = false
    if pendingRebuild {
      pendingRebuild = false
      rebuildMenu()
    }
  }

  // MARK: - mountctl plumbing

  /// Run mountctl off the main thread with a hard deadline.
  ///
  /// Without the deadline a wedged daemon strands one dispatch worker and one
  /// live child process per 15-second tick; after a few minutes the pool is
  /// exhausted, refreshes stop starting, and the icon silently freezes showing
  /// state that may be hours stale.
  func runMountctl(
    _ args: [String], timeout: TimeInterval = 20,
    completion: @escaping (Data?, String?) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      guard let ctl = mountctlPath() else {
        DispatchQueue.main.async {
          completion(nil, "mountctl not found (install with `cargo install --path .`)")
        }
        return
      }
      let result = runCommand(ctl, args: args, timeout: timeout)
      DispatchQueue.main.async { completion(result.0, result.1) }
    }
  }

  func refresh() {
    guard !refreshing else { return }
    refreshing = true
    runMountctl(["status", "--json"]) { [weak self] data, error in
      guard let self else { return }
      self.refreshing = false
      let newStatuses: [MountStatus]
      let newError: String?
      if let data, let parsed = try? JSONDecoder().decode([MountStatus].self, from: data) {
        newStatuses = parsed
        newError = nil
      } else {
        newStatuses = []
        newError = error ?? "could not parse status"
      }

      // Only touch the menu when something actually changed. Rebuilding while
      // the user is navigating swaps items out from under the cursor, and the
      // submenu's first item is "Reveal in Finder" when mounted but "Mount
      // Now" when not — so a mistimed rebuild turns one click into another.
      let changed = newStatuses != self.statuses || newError != self.daemonError
      self.statuses = newStatuses
      self.daemonError = newError
      self.updateIcon()
      guard changed else { return }
      if self.menuIsOpen {
        self.pendingRebuild = true
      } else {
        self.rebuildMenu()
      }
    }
  }

  func action(_ args: [String]) {
    runMountctl(args) { [weak self] _, error in
      if let error {
        let alert = NSAlert()
        alert.messageText = "mountd"
        alert.informativeText = error
        alert.alertStyle = .warning
        // An accessory app has no active window; without this the panel can
        // appear behind whatever is frontmost and never be seen.
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
      }
      self?.refresh()
    }
  }

  // MARK: - UI

  func setIcon(symbol: String, description: String) {
    let image = NSImage(systemSymbolName: symbol, accessibilityDescription: description)
    image?.isTemplate = true
    statusItem.button?.image = image
    statusItem.button?.setAccessibilityLabel(description)
    statusItem.button?.toolTip = description
  }

  func updateIcon() {
    let kinds = Set(statuses.map(\.kind))
    if daemonError != nil {
      setIcon(symbol: "externaldrive.badge.questionmark", description: "mountd: daemon unreachable")
    } else if kinds.contains("error") {
      setIcon(symbol: "externaldrive.badge.exclamationmark", description: "mountd: error")
    } else if kinds.contains("suspect") || kinds.contains("recovering") {
      setIcon(
        symbol: "externaldrive.trianglebadge.exclamationmark",
        description: "mountd: a mount is not responding")
    } else if kinds.contains("mounted") {
      setIcon(symbol: "externaldrive.fill.badge.checkmark", description: "mountd: mounted")
    } else {
      setIcon(symbol: "externaldrive", description: "mountd: no mounts active")
    }
  }

  func glyph(for status: MountStatus) -> String {
    switch status.kind {
    case "mounted": return "🟢"
    case "error": return "🔴"
    case "suspect", "recovering", "migrating": return "🟠"
    case "waiting", "waking": return "🟡"
    default: return "⚪️"
    }
  }

  func rebuildMenu() {
    menu.removeAllItems()

    if let daemonError {
      menu.addItem(disabled("daemon unreachable"))
      menu.addItem(disabled(daemonError))
      menu.addItem(.separator())
    } else if statuses.isEmpty {
      menu.addItem(disabled("no mounts configured"))
      menu.addItem(.separator())
    }

    for s in statuses {
      let item = NSMenuItem(
        title: "\(glyph(for: s)) \(s.name) — \(s.state)",
        action: nil, keyEquivalent: "")
      item.setAccessibilityLabel("\(s.name): \(s.state)")
      let sub = NSMenu()
      let mounted = s.kind == "mounted" || s.kind == "suspect"
      if mounted {
        sub.addItem(
          closureItem("Reveal in Finder") { [weak self] in
            self?.reveal(s)
          })
        sub.addItem(
          closureItem("Unmount") { [weak self] in
            self?.action(["unmount", "--", s.name])
          })
      } else {
        sub.addItem(
          closureItem("Mount Now") { [weak self] in
            self?.action(["mount", "--", s.name])
          })
      }
      if let via = s.mounted_via {
        sub.addItem(.separator())
        sub.addItem(disabled("via \(via)"))
      }
      if let err = s.last_error {
        sub.addItem(.separator())
        sub.addItem(disabled("last error: \(err)"))
      }
      item.submenu = sub
      menu.addItem(item)
    }

    menu.addItem(.separator())
    menu.addItem(closureItem("Reconcile Now") { [weak self] in self?.action(["poke"]) })
    menu.addItem(closureItem("Reload Config") { [weak self] in self?.action(["reload"]) })
    menu.addItem(
      closureItem("Edit Config…") {
        NSWorkspace.shared.open(
          URL(fileURLWithPath: expandTilde("~/.config/mountd/config.toml"), isDirectory: false))
      })
    menu.addItem(.separator())
    menu.addItem(
      closureItem("Quit mountd Menu Bar") {
        NSApp.terminate(nil)
      })
  }

  /// Open a mountpoint in Finder without ever touching it on the main thread.
  ///
  /// The status behind this menu item can be up to a poll interval stale, and
  /// a share whose server died still looks mounted. `URL(fileURLWithPath:)`
  /// stats the path to decide directory-ness unless told, and NSWorkspace then
  /// resolves it — on a dead SMB mount that blocks for as long as the SMB
  /// client takes to give up, freezing the status item and then Finder.
  func reveal(_ status: MountStatus) {
    guard !revealing else { return }
    revealing = true
    let path = expandTilde(status.mountpoint)
    DispatchQueue.global(qos: .userInitiated).async {
      NSWorkspace.shared.open(URL(fileURLWithPath: path, isDirectory: true))
      DispatchQueue.main.async { [weak self] in
        self?.revealing = false
        self?.refresh()
      }
    }
  }

  func disabled(_ title: String) -> NSMenuItem {
    let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
    item.isEnabled = false
    return item
  }

  func closureItem(_ title: String, handler: @escaping () -> Void) -> NSMenuItem {
    let item = ClosureMenuItem(title: title, handler: handler)
    item.target = item
    item.action = #selector(ClosureMenuItem.invoke)
    return item
  }
}

final class ClosureMenuItem: NSMenuItem {
  private var handler: (() -> Void)?

  convenience init(title: String, handler: @escaping () -> Void) {
    self.init(title: title, action: nil, keyEquivalent: "")
    self.handler = handler
  }

  @objc func invoke() {
    handler?()
  }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)  // menu bar only, no Dock icon
let delegate = AppDelegate()
app.delegate = delegate
app.run()
