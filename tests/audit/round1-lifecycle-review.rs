#[cfg(test)]
mod round1_lifecycle_review {
    use super::*;
    use crate::ops::fake::{Call, FakeChild, FakeOps};

    const MP: &str = "/private/tmp/mountd-round1-lifecycle-review-never-created";
    const A: &str = "192.0.2.11";
    const B: &str = "192.0.2.12";

    fn config(address: &str) -> String {
        format!(
            r#"
[defaults]
probe_timeout_ms = 10
health_interval_secs = 0
sleep_unmount_budget_secs = 0
wake_settle_secs = 0
[deps.ready]
cmd = "/synthetic/never-executed"
[[mount]]
name = "nas"
protocol = "smb"
share = "s"
username = "synthetic"
mountpoint = "{MP}"
[mount.policy]
allow_passwordless = true
[[mount.endpoint]]
address = "{address}"
"#
        )
    }

    async fn setup() -> (tempfile::TempDir, Arc<Engine>, Arc<FakeOps>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, config(A)).unwrap();
        let ops = FakeOps::with(|st| st.reachable.extend([A.into(), B.into()]));
        let eng = Engine::with_ops(
            path,
            Config::from_toml_str(&config(A)).unwrap(),
            ops.clone(),
        );
        eng.force_mount("nas").await.unwrap();
        eng.force_unmount("nas").await.unwrap();
        assert!(eng.runtime.lock().await["nas"].user_suppressed);
        assert!(eng.runtime.lock().await["nas"].when_held);
        ops.state().calls.clear();
        (dir, eng, ops)
    }

    async fn bounded<F: std::future::Future>(future: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(3), future)
            .await
            .expect("review step exceeded three seconds")
    }

    fn gate(ops: &FakeOps, stage: usize, child: Option<Arc<FakeChild>>) {
        let mut st = ops.state();
        match stage {
            0 => st.credential_child = child,
            1 => st.prepare_child = child,
            2 => st.mount_child = child,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn accepted_failures_clear_intent_without_bypassing_gates() {
        for failure in [
            "enumeration",
            "probe",
            "requires",
            "only_if",
            "missing",
            "cleanup",
            "final_probe",
        ] {
            let (_dir, eng, ops) = setup().await;
            match failure {
                "enumeration" => ops.state().list_mounts_fails = true,
                "probe" => ops.state().reachable.clear(),
                "requires" | "only_if" => {
                    let mut cfg = eng.cfg.write().await;
                    let condition = Some(crate::config::CondRef::Dep("ready".into()));
                    if failure == "requires" {
                        cfg.mounts[0].endpoints[0].requires = condition;
                    } else {
                        cfg.mounts[0].endpoints[0].only_if = condition;
                    }
                    ops.state().deps_down.insert("ready".into());
                }
                "missing" => eng.cfg.write().await.mounts[0].policy.allow_passwordless = false,
                "cleanup" => {
                    ops.state()
                        .passwords
                        .insert(A.into(), FetchOutcome::CleanupPending);
                }
                "final_probe" => ops.state().disconnect_on_password = true,
                _ => unreachable!(),
            }
            assert!(bounded(eng.force_mount("nas")).await.is_err(), "{failure}");
            assert!(
                !eng.runtime.lock().await["nas"].user_suppressed,
                "{failure}"
            );
            assert!(ops.mount_calls().is_empty(), "{failure}");
            assert!(ops.unmount_calls().is_empty());
            assert!(eng.claim("nas").await.is_some());
            if matches!(failure, "enumeration" | "requires" | "only_if") {
                assert!(
                    ops.calls().is_empty(),
                    "{failure} should stop before probes or credentials"
                );
            }
            {
                let mut st = ops.state();
                st.list_mounts_fails = false;
                st.reachable.insert(A.into());
                st.deps_down.clear();
                st.passwords
                    .insert(A.into(), FetchOutcome::Found("synthetic".into()));
                st.disconnect_on_password = false;
            }
            // No wake, reload or false-to-true condition edge repairs the intent.
            bounded(eng.reconcile_once()).await;
            assert_eq!(
                ops.mount_calls(),
                vec![("nas".into(), A.into())],
                "{failure}"
            );
        }
        println!("accepted failure matrix: 7 failures; zero premature mounts; direct retry passed");
    }

    #[tokio::test]
    async fn refusals_preserve_both_existing_intents_and_runtime() {
        for suppressed in [true, false] {
            for refusal in ["sleep", "shutdown", "unknown", "claim"] {
                let (_dir, eng, ops) = setup().await;
                if !suppressed {
                    eng.force_mount("nas").await.unwrap();
                }
                let before = eng.status().await;
                ops.state().calls.clear();
                let held = if refusal == "claim" {
                    eng.claim("nas").await
                } else {
                    None
                };
                match refusal {
                    "sleep" => eng.sleeping.store(true, Ordering::SeqCst),
                    "shutdown" => eng.begin_shutdown(),
                    _ => {}
                }
                let name = if refusal == "unknown" {
                    "missing"
                } else {
                    "nas"
                };
                let error = bounded(eng.force_mount(name))
                    .await
                    .unwrap_err()
                    .to_string();
                let expected = match refusal {
                    "sleep" => "system is sleeping",
                    "shutdown" => "daemon is stopping",
                    "unknown" => "no mount named",
                    _ => "mount operation already in flight",
                };
                assert!(error.contains(expected), "{refusal}: {error}");
                assert_eq!(eng.runtime.lock().await["nas"].user_suppressed, suppressed);
                assert_eq!(eng.runtime.lock().await.len(), 1);
                assert_eq!(eng.status().await, before, "{refusal}");
                assert!(ops.calls().is_empty(), "{refusal}");
                drop(held);
            }
        }
        println!("refusal matrix: 4 refusal paths x 2 prior intents passed");
    }

    #[tokio::test]
    async fn aborted_accepted_mount_keeps_new_intent_and_cleanup_ownership() {
        for stage in 0..3 {
            let (_dir, eng, ops) = setup().await;
            let child = Arc::new(FakeChild::default());
            gate(&ops, stage, Some(child.clone()));
            let task = {
                let eng = eng.clone();
                tokio::spawn(async move { eng.force_mount("nas").await })
            };
            bounded(child.started.notified()).await;
            assert!(!eng.runtime.lock().await["nas"].user_suppressed);
            task.abort();
            assert!(bounded(task).await.unwrap_err().is_cancelled());
            bounded(child.cancelled.notified()).await;
            assert!(!eng.runtime.lock().await["nas"].user_suppressed);
            assert!(eng.claim("nas").await.is_none());
            assert!(eng.lifecycle.try_write().is_err());
            assert!(eng.force_unmount("nas").await.is_err());
            bounded(eng.reconcile_once()).await;
            assert!(ops.mount_calls().is_empty());
            assert!(ops.unmount_calls().is_empty());
            gate(&ops, stage, None);
            child.release.notify_one();
            bounded(child.reaped.notified()).await;
            bounded(eng.drain()).await;
            assert!(
                ops.mount_calls().is_empty(),
                "cancelled stage {stage} produced a late mount"
            );
            bounded(eng.reconcile_once()).await;
            assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
        }
        println!("cancellation matrix: credential/preparation/mount; ownership and retry passed");
    }

    #[tokio::test]
    async fn final_sleep_or_shutdown_refusal_does_not_restore_old_intent() {
        for stopping in [false, true] {
            let (_dir, eng, ops) = setup().await;
            let child = Arc::new(FakeChild::default());
            gate(&ops, 1, Some(child.clone()));
            let task = {
                let eng = eng.clone();
                tokio::spawn(async move { eng.force_mount("nas").await })
            };
            bounded(child.started.notified()).await;
            if stopping {
                eng.begin_shutdown();
            } else {
                eng.unmount_for_sleep().await;
            }
            child.release.notify_one();
            assert!(bounded(task).await.unwrap().is_err());
            assert!(!eng.runtime.lock().await["nas"].user_suppressed);
            assert!(ops.mount_calls().is_empty());
            assert!(ops.unmount_calls().is_empty());
            bounded(eng.drain()).await;
            if !stopping {
                gate(&ops, 1, None);
                eng.note_wake(Duration::ZERO).await;
                eng.reconcile_once().await;
                assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
            }
        }
    }

    #[tokio::test]
    async fn shortcut_preserves_actual_sources_and_allows_retry_after_external_loss() {
        for (source, expected) in [
            (format!("//user@{B}/s"), Some(B)),
            ("/dev/synthetic".into(), None),
        ] {
            let (_dir, eng, ops) = setup().await;
            ops.state().mounted.insert(MP.into(), source.clone());
            eng.force_mount("nas").await.unwrap();
            assert!(!eng.runtime.lock().await["nas"].user_suppressed);
            assert!(
                ops.calls().is_empty(),
                "shortcut must not invoke credentials, probes or helpers"
            );
            assert_eq!(ops.state().mounted.get(MP), Some(&source));
            // Status is reconciled asynchronously; the shortcut must not invent A.
            assert_ne!(eng.status().await[0].mounted_via.as_deref(), Some(A));
            eng.reconcile_once().await;
            assert_eq!(eng.status().await[0].mounted_via.as_deref(), expected);
            assert!(ops.mount_calls().is_empty());
            assert!(ops.unmount_calls().is_empty());
            if let Some(address) = expected {
                assert!(ops.calls().contains(&Call::Probe {
                    address: address.into()
                }));
                assert!(!ops.calls().contains(&Call::Probe { address: A.into() }));
            }
            ops.state().mounted.remove(MP);
            eng.reconcile_once().await;
            assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
            assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(A));
        }
    }

    #[tokio::test]
    async fn failed_reload_preserves_intent_and_successful_reload_resets_it() {
        for accepted in [false, true] {
            for invalid in [false, true] {
                let (_dir, eng, ops) = setup().await;
                if accepted {
                    ops.state().list_mounts_fails = true;
                    assert!(eng.force_mount("nas").await.is_err());
                    ops.state().list_mounts_fails = false;
                }
                let claim = eng.runtime.lock().await["nas"].operation.clone();
                let before = eng.status().await;
                if invalid {
                    std::fs::write(&eng.config_path, "[[mount]").unwrap();
                } else {
                    std::fs::remove_file(&eng.config_path).unwrap();
                }
                assert!(bounded(eng.reload()).await.is_err());
                assert_eq!(eng.status().await, before);
                assert_eq!(eng.runtime.lock().await["nas"].user_suppressed, !accepted);
                assert!(Arc::ptr_eq(
                    &claim,
                    &eng.runtime.lock().await["nas"].operation
                ));
                eng.reconcile_once().await;
                assert_eq!(ops.mount_calls().len(), usize::from(accepted));
                // Remove only the fake table entry to inspect the replacement endpoint.
                ops.state().mounted.clear();
                std::fs::write(&eng.config_path, config(B)).unwrap();
                eng.reload().await.unwrap();
                assert!(eng.runtime.lock().await.is_empty());
                eng.reconcile_once().await;
                assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(B));
                assert!(!Arc::ptr_eq(
                    &claim,
                    &eng.runtime.lock().await["nas"].operation
                ));
            }
        }
        println!("reload matrix: 2 prior intents x unreadable/invalid file; replacement B passed");
    }

    #[tokio::test]
    async fn cancelled_queued_reload_cannot_reset_accepted_intent_or_publish_config() {
        let (_dir, eng, ops) = setup().await;
        let child = Arc::new(FakeChild::default());
        gate(&ops, 0, Some(child.clone()));
        let task = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_mount("nas").await })
        };
        bounded(child.started.notified()).await;
        task.abort();
        assert!(bounded(task).await.unwrap_err().is_cancelled());
        bounded(child.cancelled.notified()).await;
        std::fs::write(&eng.config_path, config(B)).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), eng.reload())
                .await
                .is_err()
        );
        assert_eq!(eng.cfg.read().await.mounts[0].endpoints[0].address, A);
        assert!(!eng.runtime.lock().await["nas"].user_suppressed);
        assert!(eng.claim("nas").await.is_none());
        gate(&ops, 0, None);
        child.release.notify_one();
        bounded(child.reaped.notified()).await;
        bounded(eng.reconcile_once()).await;
        assert_eq!(ops.mount_calls(), vec![("nas".into(), A.into())]);
        eng.reload().await.unwrap();
        eng.reconcile_once().await;
        assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(A));
        assert_eq!(ops.mount_calls().len(), 1);
    }
}
