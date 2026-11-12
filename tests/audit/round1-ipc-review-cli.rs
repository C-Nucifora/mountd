use mountd::ipc::Request;
use std::sync::atomic::Ordering;
use std::time::Duration;
#[allow(dead_code)]
mod common {
    include!(env!("MOUNTD_REVIEW_COMMON"));
}
use common::*;

#[tokio::test]
async fn menu_shapes_and_generated_boundaries_reach_actual_cli() {
    let menu = std::fs::read_to_string(env!("MOUNTD_REVIEW_MENU")).unwrap();
    let budget = 65536
        - serde_json::to_vec(&Request::Unmount {
            name: String::new(),
        })
        .unwrap()
        .len()
        - 1;
    let mut names: Vec<String> = [
        "--",
        "-",
        "--help=archive",
        "--json",
        "-hV",
        "---",
        "--version",
        "--config=/synthetic",
        " \"\\日😀 ",
        "\u{200d}",
        "\u{feff}",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    names.extend([generated(41, budget - 1), generated(42, budget)]);
    let mut calls = 0;
    for name in names {
        let server = Server::new(&name).await;
        server.engine.permits.add_permits(2);
        for action in ["mount", "unmount", "explain"] {
            let terminated =
                action == "explain" || menu.contains(&format!("[\"{action}\", \"--\", s.name]"));
            assert!(
                terminated || menu.contains(&format!("[\"{action}\", s.name]")),
                "unrecognized menu call shape"
            );
            let mut args = vec![action];
            if terminated {
                args.push("--");
            }
            args.push(&name);
            let output = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::process::Command::new(env!("CARGO_BIN_EXE_mountctl"))
                    .args(&args)
                    .env("HOME", server.dir.path())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                output.status.success(),
                "action={action} bytes={} exit={:?} stderr={}",
                name.len(),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
            calls += 1;
        }
        assert_eq!(
            server.engine.completed.load(Ordering::SeqCst),
            3,
            "successful CLI must dispatch every name"
        );
        server.finish().await;
    }
    // Direct CLI input can bypass config validation; the transport cap still applies.
    let server = Server::new("--").await;
    let too_large = generated(42, budget + 1);
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_mountctl"))
            .args(["unmount", "--", &too_large])
            .env("HOME", server.dir.path())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("IPC request too large"));
    assert_eq!(server.engine.entered.load(Ordering::SeqCst), 0);
    server.finish().await;
    // Keep real help available without a daemon, including explain's own option.
    for action in ["mount", "unmount", "explain"] {
        let home = tempfile::tempdir().unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_mountctl"))
                .args([action, "--help"])
                .env("HOME", home.path())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    }
    eprintln!("CLI names=13 action_calls={calls} unmodified_help=3 generated_boundary_names=2 oversized_cli_rejected=1");
}
