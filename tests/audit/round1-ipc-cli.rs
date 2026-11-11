use mountd::{config::Config, ipc::Request};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

async fn invoke(args: &[&str]) -> (std::process::Output, Option<Request>) {
    let home = tempfile::tempdir().unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = home.path().join("Library/Application Support/mountd");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.join("mountd.sock");
    assert!(path.as_os_str().len() < 104);
    let listener = UnixListener::bind(path).unwrap();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut peer)
            .read_line(&mut line)
            .await
            .unwrap();
        let request = serde_json::from_str::<Request>(&line).unwrap();
        peer.write_all(b"{\"ok\":true}\n").await.unwrap();
        request
    });
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_mountctl"))
            .env("HOME", home.path())
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    let mut server = server;
    let request = match tokio::time::timeout(Duration::from_millis(100), &mut server).await {
        Ok(result) => Some(result.unwrap()),
        Err(_) => {
            server.abort();
            let _ = server.await;
            None
        }
    };
    (output, request)
}

#[tokio::test]
async fn option_terminator_preserves_config_names() {
    for name in [
        "normal",
        "--help",
        "-h",
        "--",
        "-archive",
        "quote\"",
        "日本語",
    ] {
        let (output, request) = invoke(&["unmount", "--", name]).await;
        assert!(output.status.success());
        assert_eq!(request, Some(Request::Unmount { name: name.into() }));
    }
    eprintln!("CLI_TERMINATOR names=7 exact_requests=7");
}

#[tokio::test]
async fn menu_argument_shape_addresses_every_accepted_name() {
    let mut failures = Vec::new();
    for name in ["--help", "-h", "--", "-archive"] {
        let config = format!("[[mount]]\nname={name:?}\nprotocol=\"nfs\"\nexport=\"/synthetic\"\nmountpoint=\"/synthetic\"\n[[mount.endpoint]]\naddress=\"127.0.0.1\"\n");
        Config::from_toml_str(&config).unwrap();
        for action in ["mount", "unmount"] {
            // Exercise the actual menu call's two supported shapes. If that call
            // is refactored, update this extraction rather than silently guessing.
            let menu = include_str!(concat!(
                env!("MOUNTD_AUDIT_ROOT"),
                "/menubar/Sources/main.swift"
            ));
            let with_terminator = format!("self?.action([\"{action}\", \"--\", s.name])");
            let without_terminator = format!("self?.action([\"{action}\", s.name])");
            let args = if menu.contains(&with_terminator) {
                vec![action, "--", name]
            } else {
                assert!(
                    menu.contains(&without_terminator),
                    "menu argv extraction needs updating"
                );
                vec![action, name]
            };
            let (output, request) = invoke(&args).await;
            eprintln!("CLI_MENU name={name:?} action={action} exit={:?} request={request:?} stdout={:?} stderr={:?}",
                output.status.code(), String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            if request.is_none() {
                failures.push(format!("{action} {name}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "accepted names never reached IPC: {failures:?}"
    );
}
