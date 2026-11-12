#[cfg(test)]
mod round2_credential_path {
    use super::*;
    use serde_json::{json, Value};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    const RAW: &str = "r2-Q7-ümlaut:@/";
    const HEX: &str = "72322d51372dc3bc6d6c6175743a402f";
    const FAKE: &str = r#"#!/usr/bin/python3
import json, os, pathlib, sys, time
root = pathlib.Path(__file__).parent
mode = 'w' if '-w' in sys.argv else 'g'
case = json.loads((root / 'case.json').read_text())[mode]
(root / (mode + '.pid')).write_text(str(os.getpid()))
time.sleep(case.get('delay', 0))
for stream, fd in [('stdout', 1), ('stderr', 2)]:
    repeat = case.get('repeat', 1) if case.get('repeat_stream', stream) == stream else 1
    data = case.get(stream, '').encode() * repeat
    if case.get('invalid') and fd == 1:
        data = b'\xff\xfe\n'
    while data:
        data = data[os.write(fd, data):]
sys.exit(case.get('exit', 0))
"#;

    fn fixture(w: Value, g: Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("case.json"),
            json!({"w": w, "g": g}).to_string(),
        )
        .unwrap();
        let program = dir.path().join("security-fake");
        std::fs::write(&program, FAKE).unwrap();
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    async fn fetch(dir: &Path, budget: Duration, lease: OperationLease) -> FetchOutcome {
        fetch_with_program(
            dir.join("security-fake").to_str().unwrap(),
            "synthetic.invalid",
            "fake-account",
            Some(&dir.join("never-a-keychain")),
            budget,
            lease,
        )
        .await
    }

    fn private_diagnostics(outcome: &FetchOutcome) {
        let mut detail = format!("{outcome:?} {}", outcome.reason());
        if let FetchOutcome::Failed(error) = outcome {
            detail.push_str(error);
        }
        for secret in [RAW, HEX, "Q7-", "c3bc6d6c", "r2-Q7-%C3%BCmlaut%3A%40%2F"] {
            assert!(
                !detail.contains(secret),
                "synthetic secret leaked in diagnostic"
            );
        }
    }

    async fn settled(dir: &Path) {
        tokio::time::timeout(Duration::from_secs(2), crate::mountcmd::drain_commands())
            .await
            .unwrap();
        for mode in ["w", "g"] {
            if let Ok(text) = std::fs::read_to_string(dir.join(format!("{mode}.pid"))) {
                let pid = text.parse::<i32>().unwrap();
                assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "owned helper remains");
            }
        }
    }

    #[tokio::test]
    async fn metadata_valid_and_invalid_shapes() {
        let cases = [
            (
                "encoded",
                format!("password: 0x{HEX}  \"display\"\n"),
                Some(RAW),
            ),
            ("literal", format!("password: \"{HEX}\"\n"), Some(HEX)),
            ("missing", "permission denied\n".into(), None),
            ("odd_hex", "password: 0xabc\n".into(), None),
            ("nonhex", "password: 0xzz\n".into(), None),
            ("invalid_utf8_hex", "password: 0xff\n".into(), None),
            ("empty_hex", "password: 0x\n".into(), None),
            ("unterminated_text", "password: \"text\n".into(), None),
        ];
        for (label, metadata, expected) in cases {
            let dir = fixture(
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stderr": metadata}),
            );
            let got = fetch(dir.path(), Duration::from_secs(2), Arc::new(())).await;
            private_diagnostics(&got);
            settled(dir.path()).await;
            assert_eq!(got.password(), expected, "{label}");
            if expected.is_none() {
                assert!(matches!(got, FetchOutcome::Failed(_)), "{label}");
            }
            println!("metadata {label}: checked");
        }
    }

    #[tokio::test]
    async fn truncated_single_quote_must_fail_closed() {
        let dir = fixture(
            json!({"stdout": format!("{HEX}\n")}),
            json!({"stderr": "password: \"\n"}),
        );
        let got = fetch(dir.path(), Duration::from_secs(2), Arc::new(())).await;
        private_diagnostics(&got);
        settled(dir.path()).await;
        assert!(
            matches!(got, FetchOutcome::Failed(_)),
            "truncated one-quote metadata accepted: {got:?}"
        );
    }

    #[tokio::test]
    async fn contradictory_records_must_fail_closed() {
        let mut accepted = Vec::new();
        for (label, metadata) in [
            (
                "literal_then_encoded",
                format!("password: \"{HEX}\"\npassword: 0x{HEX}  \"display\"\n"),
            ),
            (
                "encoded_then_literal",
                format!("password: 0x{HEX}  \"display\"\npassword: \"{HEX}\"\n"),
            ),
        ] {
            let dir = fixture(
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stderr": metadata}),
            );
            let got = fetch(dir.path(), Duration::from_secs(2), Arc::new(())).await;
            private_diagnostics(&got);
            settled(dir.path()).await;
            if !matches!(got, FetchOutcome::Failed(_)) {
                accepted.push(label);
            }
        }
        assert!(
            accepted.is_empty(),
            "contradictory metadata accepted in orders: {accepted:?}"
        );
    }

    #[tokio::test]
    async fn first_call_interpretation_and_cross_call_failure_caps() {
        for (label, w, g, second, expected) in [
            (
                "missing",
                json!({"stdout": RAW, "stderr": HEX, "exit": 44}),
                json!({}),
                false,
                "missing",
            ),
            (
                "first_denied",
                json!({"stdout": RAW, "stderr": HEX, "exit": 1}),
                json!({}),
                false,
                "failed",
            ),
            (
                "invalid_utf8",
                json!({"invalid": true}),
                json!({}),
                false,
                "failed",
            ),
            (
                "first_stdout_cap",
                json!({"stdout": HEX, "repeat": 5000}),
                json!({}),
                false,
                "failed",
            ),
            (
                "first_stderr_cap",
                json!({"stdout": format!("{HEX}\n"), "stderr": RAW, "repeat": 5000, "repeat_stream": "stderr"}),
                json!({}),
                false,
                "failed",
            ),
            (
                "second_denied",
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stderr": RAW, "exit": 1}),
                true,
                "failed",
            ),
            (
                "second_missing",
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stderr": HEX, "exit": 44}),
                true,
                "failed",
            ),
            (
                "second_stdout_cap",
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stdout": RAW, "repeat": 5000}),
                true,
                "failed",
            ),
            (
                "second_stderr_cap",
                json!({"stdout": format!("{HEX}\n")}),
                json!({"stderr": HEX, "repeat": 5000}),
                true,
                "failed",
            ),
        ] {
            let dir = fixture(w, g);
            let got = fetch(dir.path(), Duration::from_secs(2), Arc::new(())).await;
            private_diagnostics(&got);
            settled(dir.path()).await;
            assert_eq!(dir.path().join("g.pid").exists(), second, "{label}");
            assert!(
                match expected {
                    "missing" => got == FetchOutcome::Missing,
                    _ => matches!(got, FetchOutcome::Failed(_)),
                },
                "{label}: {got:?}"
            );
            println!("caller {label}: checked");
        }
        for raw in ["abc", "DEADBEEF", "", "literal\n", "literal\n\n"] {
            let dir = fixture(json!({"stdout": format!("{raw}\n")}), json!({"exit": 1}));
            let got = fetch(dir.path(), Duration::from_secs(2), Arc::new(())).await;
            settled(dir.path()).await;
            assert!(!dir.path().join("g.pid").exists());
            if raw.is_empty() {
                assert_eq!(got, FetchOutcome::Missing);
            } else {
                assert_eq!(got.password(), Some(raw));
            }
        }
        println!("caller literal/empty/newline cases: checked");
    }

    struct ReapLease {
        root: std::path::PathBuf,
        early: Arc<AtomicBool>,
    }
    impl Drop for ReapLease {
        fn drop(&mut self) {
            for mode in ["w", "g"] {
                if let Ok(text) = std::fs::read_to_string(self.root.join(format!("{mode}.pid"))) {
                    let pid = text.parse::<i32>().unwrap();
                    if unsafe { libc::kill(pid, 0) } == 0 {
                        self.early.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn total_deadlines_and_cancelled_second_call_keep_ownership() {
        for (label, delay_w, delay_g, budget_ms, second, cancel) in [
            ("zero", 0.0, 0.0, 0, false, false),
            ("first_exhausted", 1.0, 0.0, 120, false, false),
            ("shared_budget", 0.25, 0.35, 450, true, false),
            ("second_cancelled", 0.0, 3.0, 2000, true, true),
        ] {
            let dir = fixture(
                json!({"stdout": format!("{HEX}\n"), "delay": delay_w}),
                json!({"stderr": format!("password: 0x{HEX}  \"display\"\n"), "delay": delay_g}),
            );
            let early = Arc::new(AtomicBool::new(false));
            let lease = Arc::new(ReapLease {
                root: dir.path().into(),
                early: early.clone(),
            });
            let weak = Arc::downgrade(&lease);
            let root = dir.path().to_owned();
            let start = std::time::Instant::now();
            let worker =
                tokio::spawn(
                    async move { fetch(&root, Duration::from_millis(budget_ms), lease).await },
                );
            if cancel {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while std::fs::read_to_string(dir.path().join("g.pid"))
                        .ok()
                        .and_then(|value| value.parse::<i32>().ok())
                        .is_none()
                    {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
                assert!(weak.upgrade().is_some());
                worker.abort();
                assert!(worker.await.unwrap_err().is_cancelled());
            } else {
                let got = worker.await.unwrap();
                private_diagnostics(&got);
                assert_eq!(got, FetchOutcome::TimedOut, "{label}");
            }
            settled(dir.path()).await;
            assert!(weak.upgrade().is_none(), "{label}: retained after drain");
            assert!(
                !early.load(Ordering::SeqCst),
                "{label}: lease released before reap"
            );
            assert_eq!(dir.path().join("g.pid").exists(), second, "{label}");
            if budget_ms == 0 {
                assert!(!dir.path().join("w.pid").exists());
            }
            assert!(start.elapsed() < Duration::from_millis(1800), "{label}");
            println!("deadline/ownership {label}: checked");
        }
    }
}
