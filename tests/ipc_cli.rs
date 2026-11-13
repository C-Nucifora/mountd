//! CLI protocol fixtures use an isolated HOME and a fake Unix server only.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;

fn cli_fixture(
    args: &[&str],
    expected_request: &'static str,
    response: Vec<u8>,
) -> std::process::Output {
    // sockaddr_un is only 104 bytes on macOS; HOME includes the library suffix.
    let home = tempfile::Builder::new()
        .prefix(".md-")
        .tempdir_in(dirs::home_dir().unwrap())
        .unwrap();
    let dir = home.path().join("Library/Application Support/mountd");
    std::fs::create_dir_all(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("mountd.sock")).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        assert_eq!(line, expected_request);
        // Oversize-response tests may close the connection before the write ends.
        let _ = stream.write_all(&response);
    });
    let output = Command::new(env!("CARGO_BIN_EXE_mountctl"))
        .env("HOME", home.path())
        .args(args)
        .output()
        .unwrap();
    server.join().unwrap();
    output
}

fn status_fixture(response: Vec<u8>) -> std::process::Output {
    cli_fixture(&["status", "--json"], "{\"cmd\":\"status\"}\n", response)
}

#[test]
fn explain_requires_its_payload_and_preserves_the_request_name() {
    let output = cli_fixture(
        &["explain", "synthetic", "--json"],
        "{\"cmd\":\"explain\",\"name\":\"synthetic\"}\n",
        b"{\"ok\":true}\n".to_vec(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing explanations"));
    assert!(output.stdout.is_empty());
}

#[test]
fn status_json_and_daemon_errors() {
    let output = status_fixture(b"{\"ok\":true,\"status\":[]}\n".to_vec());
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
    let output = status_fixture(b"{\"ok\":false,\"error\":\"synthetic failure\"}\n".to_vec());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("synthetic failure"));
}

#[test]
fn malformed_responses_do_not_report_an_empty_mount_list() {
    for response in [
        b"{\"ok\":true}\n".to_vec(),
        b"{\"ok\":true,\"status\":[]}".to_vec(),
        b"not json\n".to_vec(),
        vec![b' '; 4 * 1024 * 1024 + 1],
    ] {
        let output = status_fixture(response);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
