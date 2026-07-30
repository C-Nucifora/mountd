// mountd menu bar client. Renders `mountctl status --json` as an
// NSStatusItem menu with per-mount actions. Pure AppKit, no Xcode project —
// built by menubar/build.sh with swiftc.

import AppKit
import Foundation

struct MountStatus: Codable {
  let name: String
  let state: String
  let mountpoint: String
  let mounted_via: String?
  let last_error: String?
}

func expandTilde(_ path: String) -> String {
  (path as NSString).expandingTildeInPath
}

/// Locate mountctl: next to common install locations, then $PATH.
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

  func applicationDidFinishLaunching(_ notification: Notification) {
    statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    menu.delegate = self
    statusItem.menu = menu
    setIcon(symbol: "externaldrive", description: "mountd")
    refresh()
    Timer.scheduledTimer(withTimeInterval: 15, repeats: true) { [weak self] _ in
      self?.refresh()
    }
  }

  func menuWillOpen(_ menu: NSMenu) {
    refresh()
  }

  // MARK: - mountctl plumbing

  func runMountctl(_ args: [String], completion: @escaping (Data?, String?) -> Void) {
    guard let ctl = mountctlPath() else {
      completion(nil, "mountctl not found (install with `cargo install --path .`)")
      return
    }
    DispatchQueue.global(qos: .userInitiated).async {
      let proc = Process()
      proc.executableURL = URL(fileURLWithPath: ctl)
      proc.arguments = args
      let outPipe = Pipe()
      let errPipe = Pipe()
      proc.standardOutput = outPipe
      proc.standardError = errPipe
      do {
        try proc.run()
      } catch {
        DispatchQueue.main.async { completion(nil, error.localizedDescription) }
        return
      }
      let out = outPipe.fileHandleForReading.readDataToEndOfFile()
      let err = errPipe.fileHandleForReading.readDataToEndOfFile()
      proc.waitUntilExit()
      DispatchQueue.main.async {
        if proc.terminationStatus == 0 {
          completion(out, nil)
        } else {
          let msg = String(data: err, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
          completion(nil, msg?.isEmpty == false ? msg : "mountctl exited \(proc.terminationStatus)")
        }
      }
    }
  }

  func refresh() {
    runMountctl(["status", "--json"]) { [weak self] data, error in
      guard let self else { return }
      if let data, let parsed = try? JSONDecoder().decode([MountStatus].self, from: data) {
        self.statuses = parsed
        self.daemonError = nil
      } else {
        self.statuses = []
        self.daemonError = error ?? "could not parse status"
      }
      self.rebuildMenu()
      self.updateIcon()
    }
  }

  func action(_ args: [String]) {
    runMountctl(args) { [weak self] _, error in
      if let error {
        let alert = NSAlert()
        alert.messageText = "mountd"
        alert.informativeText = error
        alert.alertStyle = .warning
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
  }

  func updateIcon() {
    if daemonError != nil {
      setIcon(symbol: "externaldrive.badge.questionmark", description: "mountd: daemon unreachable")
    } else if statuses.contains(where: { $0.state.hasPrefix("error") }) {
      setIcon(symbol: "externaldrive.badge.exclamationmark", description: "mountd: error")
    } else if statuses.contains(where: { $0.state.hasPrefix("mounted") }) {
      setIcon(symbol: "externaldrive.fill.badge.checkmark", description: "mountd: mounted")
    } else {
      setIcon(symbol: "externaldrive", description: "mountd: no mounts active")
    }
  }

  func glyph(for state: String) -> String {
    if state.hasPrefix("mounted") { return "🟢" }
    if state.hasPrefix("error") { return "🔴" }
    if state.hasPrefix("waiting") { return "🟡" }
    return "⚪️"
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
        title: "\(glyph(for: s.state)) \(s.name) — \(s.state)",
        action: nil, keyEquivalent: "")
      let sub = NSMenu()
      let mounted = s.state.hasPrefix("mounted")
      if mounted {
        sub.addItem(
          closureItem("Reveal in Finder") { [weak self] in
            NSWorkspace.shared.open(URL(fileURLWithPath: expandTilde(s.mountpoint)))
            self?.refresh()
          })
        sub.addItem(
          closureItem("Unmount") { [weak self] in
            self?.action(["unmount", s.name])
          })
      } else {
        sub.addItem(
          closureItem("Mount Now") { [weak self] in
            self?.action(["mount", s.name])
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
        NSWorkspace.shared.open(URL(fileURLWithPath: expandTilde("~/.config/mountd/config.toml")))
      })
    menu.addItem(.separator())
    menu.addItem(
      closureItem("Quit mountd Menu Bar") {
        NSApp.terminate(nil)
      })
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
