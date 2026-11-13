import Foundation
import Darwin

/// Bounded child execution. Nonblocking reads avoid workers stranded on pipes
/// inherited by grandchildren, and SIGKILL handles a child that ignores SIGTERM.
func runCommand(_ executable: String, args: [String], timeout: TimeInterval) -> (Data?, String?) {
  let proc = Process()
  proc.executableURL = URL(fileURLWithPath: executable)
  proc.arguments = args
  let output = Pipe()
  let errors = Pipe()
  proc.standardOutput = output
  proc.standardError = errors
  defer {
    try? output.fileHandleForReading.close()
    try? errors.fileHandleForReading.close()
    try? output.fileHandleForWriting.close()
    try? errors.fileHandleForWriting.close()
  }
  for pipe in [output, errors] {
    let fd = pipe.fileHandleForReading.fileDescriptor
    guard fcntl(fd, F_SETFL, O_NONBLOCK) != -1 else {
      return (nil, "could not configure mountctl output")
    }
  }
  do {
    try proc.run()
  } catch {
    return (nil, error.localizedDescription)
  }
  // A parent write handle would prevent EOF after the child exits.
  try? output.fileHandleForWriting.close()
  try? errors.fileHandleForWriting.close()
  let deadline = DispatchTime.now() + timeout
  let limit = 1_048_576
  var stdout = Data()
  var stderr = Data()
  var readFailed = false
  func drain(_ pipe: Pipe, into data: inout Data) {
    var buffer = [UInt8](repeating: 0, count: 8192)
    // Bound each pass too: a child continuously writing must not starve timeout.
    for _ in 0..<128 {
      let n = Darwin.read(pipe.fileHandleForReading.fileDescriptor, &buffer, buffer.count)
      if n > 0 {
        if data.count + n > limit { readFailed = true; return }
        data.append(contentsOf: buffer.prefix(n))
      } else {
        if n < 0 && errno != EAGAIN && errno != EINTR { readFailed = true }
        return
      }
    }
  }
  while true {
    drain(output, into: &stdout)
    drain(errors, into: &stderr)
    if readFailed || DispatchTime.now() >= deadline {
      if proc.isRunning { kill(proc.processIdentifier, SIGKILL) }
      return (nil, readFailed ? "mountctl output exceeded limit or could not be read" : "mountctl timed out after \(Int(timeout))s")
    }
    if !proc.isRunning { break }
    Thread.sleep(forTimeInterval: 0.01)
  }
  // Collect bytes written between the last read and process termination.
  drain(output, into: &stdout)
  drain(errors, into: &stderr)
  if readFailed { return (nil, "mountctl output exceeded limit or could not be read") }
  if proc.terminationStatus == 0 { return (stdout, nil) }
  let message = String(data: stderr, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
  return (nil, message?.isEmpty == false ? message : "mountctl exited \(proc.terminationStatus)")
}
