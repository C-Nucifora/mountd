#[cfg(test)]
mod round3_policy {
    use super::*;
    use crate::ops::fake::{Call, FakeOps};

    const MP: &str = "/private/tmp/mountd-round3-policy-synthetic";
    const A: &str = "192.0.2.10";
    const B: &str = "192.0.2.20";
    const C: &str = "192.0.2.30";

    fn config() -> String {
        format!(
            r#"
[defaults]
probe_timeout_ms = 50
health_interval_secs = 0
[deps.present]
cmd = "/round3-policy-never-executed"
[deps.a]
cmd = "/round3-policy-never-executed"
[deps.b]
cmd = "/round3-policy-never-executed"
[[mount]]
name = "nas"
protocol = "smb"
share = "synthetic"
mountpoint = "{MP}"
[mount.when]
all_of = ["dep:present"]
[mount.policy]
migrate = true
migrate_confirmations = 2
unmount_grace_secs = 30
[[mount.endpoint]]
address = "{A}"
only_if = "dep:a"
[[mount.endpoint]]
address = "{B}"
requires = "dep:b"
[[mount.endpoint]]
address = "{C}"
"#
        )
    }

    fn fixture(text: &str, path: PathBuf) -> (Arc<Engine>, Arc<FakeOps>) {
        let ops = FakeOps::with(|st| {
            st.mounted.insert(MP.into(), format!("//user@{C}/s"));
            st.reachable.extend([A.into(), B.into(), C.into()]);
        });
        let eng = Engine::with_ops(path, Config::from_toml_str(text).unwrap(), ops.clone());
        (eng, ops)
    }

    fn dep(ops: &FakeOps, name: &str, up: bool) {
        if up {
            ops.state().deps_down.remove(name);
        } else {
            ops.state().deps_down.insert(name.into());
        }
    }

    async fn mature_grace(eng: &Engine) {
        eng.runtime.lock().await.get_mut("nas").unwrap().gone_since =
            Some(Instant::now() - Duration::from_secs(31));
    }

    async fn migration(eng: &Engine) -> (Option<usize>, u32) {
        let rt = eng.runtime.lock().await;
        let entry = &rt["nas"];
        (entry.migrate_target, entry.migrate_streak)
    }

    #[tokio::test]
    async fn migration_sequences_match_consecutive_winner_model() {
        let cfg = Config::from_toml_str(&config()).unwrap();
        for sequence in 0..625 {
            let ops = FakeOps::with(|st| {
                st.mounted.insert(MP.into(), format!("//user@{C}/s"));
                st.reachable.extend([A.into(), B.into(), C.into()]);
                st.unmount_needs_force.insert(MP.into());
            });
            let eng = Engine::with_ops("/unused-round3-policy".into(), cfg.clone(), ops.clone());
            let mut digits = sequence;
            let mut previous = None;
            let mut attempts = 0;
            for step in 0..4 {
                let symbol = digits % 5;
                digits /= 5;
                // A wins, B wins, no eligible target, condition loss, A probe fails.
                dep(&ops, "present", symbol != 3);
                dep(&ops, "a", matches!(symbol, 0 | 3 | 4));
                dep(&ops, "b", symbol == 1);
                if symbol == 4 {
                    ops.kill_endpoint(A);
                } else {
                    ops.revive_endpoint(A);
                }
                let winner = match symbol {
                    0 => Some(0),
                    1 => Some(1),
                    _ => None,
                };
                if winner.is_some() && winner == previous {
                    attempts += 1;
                    previous = None; // A busy attempt needs two new confirmations.
                } else {
                    previous = winner;
                }
                ops.state().calls.clear();
                eng.reconcile_once().await;
                let expected_streak = u32::from(previous.is_some());
                assert_eq!(
                    migration(&eng).await,
                    (previous, expected_streak),
                    "sequence={sequence} step={step}"
                );
                let rt = eng.runtime.lock().await;
                assert_eq!(rt["nas"].mounted_via.as_deref(), Some(C));
                assert_eq!(rt["nas"].health_failures, 0);
                drop(rt);
                assert!(ops.mount_calls().is_empty());
                assert!(ops.unmount_calls().iter().all(|(_, force)| !force));
                let expected_attempt = winner.is_some() && previous.is_none();
                assert_eq!(ops.unmount_calls().len(), usize::from(expected_attempt));
                for call in ops.calls() {
                    if let Call::Probe { address } = call {
                        assert_ne!(symbol, 3, "condition loss must skip health/migration");
                        assert!(
                            address == C
                                || (address == A && matches!(symbol, 0 | 4))
                                || (address == B && symbol == 1)
                        );
                    }
                }
            }
            assert!(attempts <= 2);
        }
        println!("migration model: 625 sequences, 2500 reconcile transitions");
    }

    #[tokio::test]
    async fn interrupted_grace_and_busy_policy_retry_require_continuous_loss() {
        let (eng, ops) = fixture(
            &config().replace("migrate = true", "migrate = false"),
            "/unused-round3-policy".into(),
        );
        eng.reconcile_once().await;
        dep(&ops, "present", false);
        eng.reconcile_once().await;
        mature_grace(&eng).await;
        dep(&ops, "present", true);
        eng.reconcile_once().await;
        assert!(eng.runtime.lock().await["nas"].gone_since.is_none());
        dep(&ops, "present", false);
        eng.reconcile_once().await;
        assert!(ops.unmount_calls().is_empty());
        mature_grace(&eng).await;
        ops.state().unmount_needs_force.insert(MP.into());
        for _ in 0..2 {
            eng.reconcile_once().await;
        }
        assert_eq!(ops.unmount_calls(), vec![(MP.into(), false); 2]);
        assert_eq!(
            eng.runtime.lock().await["nas"].mounted_via.as_deref(),
            Some(C)
        );
        dep(&ops, "present", true);
        eng.reconcile_once().await;
        assert!(eng.runtime.lock().await["nas"].last_error.is_none());
        dep(&ops, "present", false);
        eng.reconcile_once().await;
        assert_eq!(ops.unmount_calls().len(), 2);
        mature_grace(&eng).await;
        ops.state().unmount_needs_force.clear();
        eng.reconcile_once().await;
        eng.reconcile_once().await;
        assert_eq!(ops.unmount_calls(), vec![(MP.into(), false); 3]);
        assert!(ops.mount_calls().is_empty());
        dep(&ops, "present", true);
        eng.reconcile_once().await;
        assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
    }

    #[tokio::test]
    async fn source_replacement_discards_partial_migration_evidence() {
        for (source, expected) in [
            (format!("//user@{B}/s"), Some(B)),
            ("198.51.100.99:/export".into(), Some("198.51.100.99")),
            ("/dev/disk9s1".into(), None),
        ] {
            let (eng, ops) = fixture(&config(), "/unused-round3-policy".into());
            ops.revive_endpoint("198.51.100.99");
            eng.reconcile_once().await;
            assert_eq!(migration(&eng).await, (Some(0), 1));
            ops.state().mounted.insert(MP.into(), source.clone());
            eng.reconcile_once().await;
            assert!(ops.unmount_calls().is_empty(), "source={source}");
            assert!(ops.mount_calls().is_empty());
            assert_eq!(
                eng.runtime.lock().await["nas"].mounted_via.as_deref(),
                expected
            );
            let expected_streak = if source.contains(B) {
                (Some(0), 1)
            } else {
                (None, 0)
            };
            assert_eq!(migration(&eng).await, expected_streak);
            ops.state()
                .mounted
                .insert(MP.into(), format!("{C}:/export"));
            eng.reconcile_once().await;
            assert!(ops.unmount_calls().is_empty());
            assert_eq!(migration(&eng).await, (Some(0), 1));
            eng.reconcile_once().await;
            assert_eq!(ops.unmount_calls(), vec![(MP.into(), false)]);
            assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
        }
    }

    #[tokio::test]
    async fn invalid_reload_preserves_confirmations_valid_reorder_restarts_them() {
        for invalid in [
            "invalid = [".to_string(),
            config().replace("migrate_confirmations = 2", "migrate_confirmations = 0"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("policy.toml");
            let (eng, ops) = fixture(&config(), path.clone());
            eng.reconcile_once().await;
            assert_eq!(migration(&eng).await, (Some(0), 1));
            std::fs::write(&path, invalid).unwrap();
            assert!(eng.reload().await.is_err());
            assert_eq!(migration(&eng).await, (Some(0), 1));
            eng.reconcile_once().await;
            assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);

            // New priority is B, A, C. The old volume remains A, now rank 1.
            let reordered = config()
                .replace(A, "ADDRESS_SWAP")
                .replace(B, A)
                .replace("ADDRESS_SWAP", B);
            std::fs::write(&path, reordered).unwrap();
            eng.reload().await.unwrap();
            assert!(eng.runtime.lock().await.is_empty());
            ops.state().calls.clear();
            eng.reconcile_once().await;
            assert_eq!(
                eng.runtime.lock().await["nas"].mounted_via.as_deref(),
                Some(A)
            );
            assert_eq!(migration(&eng).await, (Some(0), 1));
            assert!(ops.unmount_calls().is_empty());
            eng.reconcile_once().await;
            assert_eq!(ops.unmount_calls(), vec![(MP.into(), false)]);
            assert_eq!(ops.mount_calls(), vec![("nas".into(), B.into())]);
        }
    }

    #[tokio::test]
    async fn invalid_reload_retains_grace_valid_policy_change_discards_it() {
        for pinned in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("policy.toml");
            let original = config().replace("migrate = true", "migrate = false");
            let (eng, ops) = fixture(&original, path.clone());
            dep(&ops, "present", false);
            eng.reconcile_once().await;
            mature_grace(&eng).await;
            let before = eng.runtime.lock().await["nas"].gone_since;
            std::fs::write(
                &path,
                original.replace("unmount_grace_secs = 30", "unmount_grace_secs = 86401"),
            )
            .unwrap();
            assert!(eng.reload().await.is_err());
            assert_eq!(eng.runtime.lock().await["nas"].gone_since, before);
            ops.state().unmount_needs_force.insert(MP.into());
            eng.reconcile_once().await;
            assert_eq!(ops.unmount_calls(), vec![(MP.into(), false)]);
            let replacement = if pinned {
                original.replace(
                    "unmount_grace_secs = 30",
                    "unmount_when_gone = false\nunmount_grace_secs = 60",
                )
            } else {
                original.replace("unmount_grace_secs = 30", "unmount_grace_secs = 60")
            };
            std::fs::write(&path, replacement).unwrap();
            eng.reload().await.unwrap();
            ops.state().calls.clear();
            ops.state().unmount_needs_force.clear();
            eng.reconcile_once().await;
            assert!(ops.unmount_calls().is_empty());
            eng.runtime.lock().await.get_mut("nas").unwrap().gone_since =
                Some(Instant::now() - Duration::from_secs(61));
            eng.reconcile_once().await;
            assert_eq!(ops.unmount_calls().len(), usize::from(!pinned));
            assert!(ops.unmount_calls().iter().all(|(_, force)| !force));
            assert!(ops.mount_calls().is_empty());
        }
    }

    #[tokio::test]
    async fn health_success_breaks_failure_streak_before_confirmed_migration() {
        let (eng, ops) = fixture(
            &config().replace("migrate_confirmations = 2", "migrate_confirmations = 4"),
            "/unused-round3-policy".into(),
        );
        for (step, (up, failures)) in [(false, 1), (false, 2), (true, 0), (false, 1)]
            .into_iter()
            .enumerate()
        {
            if up {
                ops.revive_endpoint(C);
            } else {
                ops.kill_endpoint(C);
            }
            eng.reconcile_once().await;
            assert_eq!(
                eng.runtime.lock().await["nas"].health_failures,
                if step < 3 { failures } else { 0 }
            );
            assert_eq!(ops.unmount_calls().len(), usize::from(step == 3));
            assert_eq!(ops.mount_calls().len(), usize::from(step == 3));
        }
        assert_eq!(
            ops.unmount_calls(),
            vec![(MP.into(), false)],
            "the success interrupts the failure streak; migration remains polite"
        );
        assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
    }
}
