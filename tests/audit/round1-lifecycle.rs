#[cfg(test)]
mod round1_lifecycle {
    use super::*;
    use crate::ops::fake::{FakeChild, FakeOps};

    const MP: &str = "/private/tmp/mountd-round1-lifecycle-never-created";
    const A: &str = "10.17.0.1";
    const B: &str = "10.17.0.2";

    fn config(address: &str) -> String {
        format!(
            r#"
[defaults]
probe_timeout_ms = 20
health_interval_secs = 0
sleep_unmount_budget_secs = 0
wake_settle_secs = 0
debounce_secs = 0
poll_interval_secs = 3600

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

    fn setup() -> (tempfile::TempDir, Arc<Engine>, Arc<FakeOps>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, config(A)).unwrap();
        let ops = FakeOps::with(|st| {
            st.reachable.extend([A.to_string(), B.to_string()]);
        });
        let eng = Engine::with_ops(
            path,
            Config::from_toml_str(&config(A)).unwrap(),
            ops.clone(),
        );
        (dir, eng, ops)
    }

    async fn bounded<F: std::future::Future>(future: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(3), future)
            .await
            .expect("bounded lifecycle step")
    }

    async fn signal(notify: &Notify) {
        bounded(notify.notified()).await;
    }

    fn gate(ops: &FakeOps, stage: usize, child: Option<Arc<FakeChild>>) {
        let mut st = ops.state();
        match stage {
            0 => st.credential_child = child,
            1 => st.prepare_child = child,
            2 => st.mount_child = child,
            3 => st.unmount_child = child,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn successful_manual_sequences_match_model() {
        // Exhaust all 4^4 sequences. All endpoints stay reachable; no real effects.
        for sequence in 0..256 {
            let (_dir, eng, ops) = setup();
            let mut suppressed = false;
            let mut mounted = false;
            let mut code = sequence;
            for step in 0..4 {
                let action = code % 4;
                code /= 4;
                match action {
                    0 => {
                        bounded(eng.force_mount("nas")).await.unwrap();
                        suppressed = false;
                        mounted = true;
                    }
                    1 => {
                        bounded(eng.force_unmount("nas")).await.unwrap();
                        suppressed = true;
                        mounted = false;
                    }
                    2 => {
                        bounded(eng.reconcile_once()).await;
                        mounted |= !suppressed;
                    }
                    3 => {
                        // Give cleanup a nonzero budget for an immediate fake unmount.
                        eng.cfg.write().await.defaults.sleep_unmount_budget_secs = 1;
                        bounded(eng.unmount_for_sleep()).await;
                        assert!(eng.sleeping.load(Ordering::SeqCst));
                        bounded(eng.note_wake(Duration::ZERO)).await;
                        bounded(eng.reconcile_once()).await;
                        mounted = !suppressed;
                    }
                    _ => unreachable!(),
                }
                let actual_mounted = ops.state().mounted.contains_key(MP);
                assert_eq!(actual_mounted, mounted, "sequence={sequence} step={step}");
                let rt = eng.runtime.lock().await;
                let entry = rt.get("nas").unwrap();
                assert_eq!(
                    entry.user_suppressed, suppressed,
                    "sequence={sequence} step={step}"
                );
                if mounted {
                    assert_eq!(entry.mounted_via.as_deref(), Some(A));
                }
                assert!(entry.operation.try_lock().is_ok());
            }
        }
        println!("model: 256 sequences, 1024 lifecycle actions passed");
    }

    #[tokio::test]
    async fn explicit_mount_of_reappeared_volume_clears_manual_intent() {
        let (_dir, eng, ops) = setup();
        eng.force_mount("nas").await.unwrap();
        eng.force_unmount("nas").await.unwrap();
        // Finder or another same-user tool remounts before the explicit command.
        ops.state()
            .mounted
            .insert(MP.into(), format!("//user@{B}/s"));
        eng.force_mount("nas").await.unwrap();
        let after_explicit_mount = eng.runtime.lock().await["nas"].user_suppressed;
        eng.cfg.write().await.defaults.sleep_unmount_budget_secs = 1;
        eng.unmount_for_sleep().await;
        eng.note_wake(Duration::ZERO).await;
        eng.reconcile_once().await;
        println!(
            "reappeared volume: suppression={after_explicit_mount}, status={:?}, calls={:?}",
            eng.status().await,
            ops.calls()
        );
        assert!(
            ops.state().mounted.contains_key(MP),
            "explicit mount must cancel manual suppression so wake can remount"
        );
    }

    #[tokio::test]
    async fn failed_explicit_mount_reenables_automatic_retry() {
        let (_dir, eng, ops) = setup();
        eng.force_mount("nas").await.unwrap();
        eng.force_unmount("nas").await.unwrap();
        ops.state()
            .mount_fails
            .insert(A.into(), "synthetic transient failure".into());
        assert!(eng.force_mount("nas").await.is_err());
        ops.state().mount_fails.clear();
        eng.note_wake(Duration::ZERO).await;
        eng.reconcile_once().await;
        println!(
            "failed explicit mount retry: status={:?}, calls={:?}",
            eng.status().await,
            ops.calls()
        );
        assert!(
            ops.state().mounted.contains_key(MP),
            "explicit mount ends earlier suppression even if its first attempt fails"
        );
    }

    #[tokio::test]
    async fn cancelled_operations_block_reload_through_removal_and_readdition() {
        for stage in 0..4 {
            for replacement in 0..3 {
                let (_dir, eng, ops) = setup();
                if stage == 3 {
                    eng.force_mount("nas").await.unwrap();
                }
                let child = Arc::new(FakeChild::default());
                gate(&ops, stage, Some(child.clone()));
                let operation = {
                    let eng = eng.clone();
                    tokio::spawn(async move {
                        if stage == 3 {
                            eng.force_unmount("nas").await
                        } else {
                            eng.force_mount("nas").await
                        }
                    })
                };
                signal(&child.started).await;
                let old_claim = eng.runtime.lock().await["nas"].operation.clone();
                operation.abort();
                assert!(bounded(operation).await.unwrap_err().is_cancelled());
                signal(&child.cancelled).await;
                let changed = match replacement {
                    0 => String::new(),
                    1 => config(B),
                    2 => config(A),
                    _ => unreachable!(),
                };
                std::fs::write(&eng.config_path, changed).unwrap();
                let mut reload = Box::pin(eng.reload());
                assert!(tokio::time::timeout(Duration::from_millis(10), &mut reload)
                    .await
                    .is_err());
                assert_eq!(bounded(eng.status()).await.len(), 1);
                assert!(eng.claim("nas").await.is_none());
                assert!(old_claim.try_lock().is_err());
                let effects = (ops.mount_calls(), ops.unmount_calls());
                gate(&ops, stage, None);
                child.release.notify_one();
                signal(&child.reaped).await;
                bounded(reload).await.unwrap();
                assert!(eng.runtime.lock().await.is_empty());
                assert_eq!(
                    (ops.mount_calls(), ops.unmount_calls()),
                    effects,
                    "cancelled stage={stage} produced a late effect"
                );
                assert!(old_claim.try_lock().is_ok());
                if replacement == 0 {
                    eng.reconcile_once().await;
                    assert!(eng.status().await.is_empty());
                    assert_eq!((ops.mount_calls(), ops.unmount_calls()), effects);
                    std::fs::write(&eng.config_path, config(B)).unwrap();
                    eng.reload().await.unwrap();
                }
                eng.reconcile_once().await;
                let expected = if stage == 3 || replacement == 2 { A } else { B };
                assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(expected));
                assert!(!Arc::ptr_eq(
                    &old_claim,
                    &eng.runtime.lock().await["nas"].operation
                ));
            }
        }
        println!("reload cancellation matrix: 4 gated stages x 3 config replacements passed");
    }

    #[tokio::test]
    async fn reload_waits_for_successful_pending_unmount_then_uses_new_endpoint() {
        let (_dir, eng, ops) = setup();
        eng.force_mount("nas").await.unwrap();
        let child = Arc::new(FakeChild::default());
        gate(&ops, 3, Some(child.clone()));
        let unmount = {
            let eng = eng.clone();
            tokio::spawn(async move { eng.force_unmount("nas").await })
        };
        signal(&child.started).await;
        std::fs::write(&eng.config_path, config(B)).unwrap();
        let mut reload = Box::pin(eng.reload());
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut reload)
            .await
            .is_err());
        assert_eq!(
            bounded(eng.status()).await[0].mounted_via.as_deref(),
            Some(A)
        );
        child.release.notify_one();
        bounded(unmount).await.unwrap().unwrap();
        bounded(reload).await.unwrap();
        gate(&ops, 3, None);
        eng.reconcile_once().await;
        assert_eq!(
            ops.mount_calls(),
            vec![("nas".into(), A.into()), ("nas".into(), B.into())]
        );
        assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(B));
        assert!(!eng.runtime.lock().await["nas"].user_suppressed);
    }

    #[tokio::test]
    async fn removal_leaves_volume_and_readdition_adopts_actual_source() {
        let (_dir, eng, ops) = setup();
        eng.force_mount("nas").await.unwrap();
        std::fs::write(&eng.config_path, "").unwrap();
        eng.reload().await.unwrap();
        eng.reconcile_once().await;
        assert!(eng.status().await.is_empty());
        assert!(ops.state().mounted.contains_key(MP));
        assert!(ops.unmount_calls().is_empty());
        std::fs::write(&eng.config_path, config(B)).unwrap();
        eng.reload().await.unwrap();
        eng.reconcile_once().await;
        assert_eq!(
            eng.status().await[0].mounted_via.as_deref(),
            Some(A),
            "configured endpoint B must not replace actual source A in status"
        );
        assert_eq!(ops.mount_calls().len(), 1);
        assert!(ops.unmount_calls().is_empty());
        eng.force_unmount("nas").await.unwrap();
        eng.force_mount("nas").await.unwrap();
        assert_eq!(eng.status().await[0].mounted_via.as_deref(), Some(B));
    }

    #[tokio::test]
    async fn queued_sleep_wake_sleep_waits_for_reap_without_intermediate_mount() {
        let (_dir, eng, ops) = setup();
        let child = Arc::new(FakeChild::default());
        gate(&ops, 2, Some(child.clone()));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let run = tokio::spawn(eng.clone().run(Some(rx)));
        signal(&child.started).await;
        let (ack1, done1) = tokio::sync::oneshot::channel();
        let (ack2, mut done2) = tokio::sync::oneshot::channel();
        tx.send(power::PowerEvent::WillSleep { ack: ack1 }).unwrap();
        tx.send(power::PowerEvent::DidWake).unwrap();
        tx.send(power::PowerEvent::WillSleep { ack: ack2 }).unwrap();
        signal(&child.cancelled).await;
        bounded(done1).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut done2)
            .await
            .is_err());
        assert!(eng.sleeping.load(Ordering::SeqCst));
        assert!(eng.claim("nas").await.is_none());
        gate(&ops, 2, None);
        child.release.notify_one();
        signal(&child.reaped).await;
        bounded(done2).await.unwrap();
        assert!(eng.sleeping.load(Ordering::SeqCst));
        assert!(ops.mount_calls().is_empty());
        tx.send(power::PowerEvent::DidWake).unwrap();
        bounded(async {
            while ops.mount_calls().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        eng.begin_shutdown();
        run.abort();
        let _ = bounded(run).await;
        bounded(eng.drain()).await;
        assert_eq!(ops.mount_calls().len(), 1);
        assert!(eng.claim("nas").await.is_some());
    }

    #[tokio::test]
    async fn shutdown_rejects_queued_wake_reload_and_final_mount() {
        for stage in 0..2 {
            for forced in [false, true] {
                let (_dir, eng, ops) = setup();
                let child = Arc::new(FakeChild::default());
                gate(&ops, stage, Some(child.clone()));
                let operation = {
                    let eng = eng.clone();
                    tokio::spawn(async move {
                        if forced {
                            eng.force_mount("nas").await
                        } else {
                            eng.reconcile_once().await;
                            Ok(())
                        }
                    })
                };
                signal(&child.started).await;
                let mut reload = Box::pin(eng.reload());
                let mut wake = Box::pin(eng.note_wake(Duration::ZERO));
                assert!(tokio::time::timeout(Duration::from_millis(10), &mut reload)
                    .await
                    .is_err());
                assert!(tokio::time::timeout(Duration::from_millis(10), &mut wake)
                    .await
                    .is_err());
                eng.begin_shutdown();
                assert!(tokio::time::timeout(Duration::from_millis(10), eng.drain())
                    .await
                    .is_err());
                child.release.notify_one();
                let outcome = bounded(operation).await.unwrap();
                if forced {
                    assert!(outcome.is_err());
                }
                assert!(bounded(reload).await.is_err());
                bounded(wake).await;
                bounded(eng.drain()).await;
                assert!(eng.sleeping.load(Ordering::SeqCst));
                assert!(eng.force_mount("nas").await.is_err());
                assert!(eng.force_unmount("nas").await.is_err());
                eng.reconcile_once().await;
                eng.unmount_for_sleep().await;
                assert!(ops.mount_calls().is_empty());
                assert!(ops.unmount_calls().is_empty());
                assert!(eng.claim("nas").await.is_some());
            }
        }
        println!("shutdown matrix: credential/preparation x automatic/manual passed");
    }
}
