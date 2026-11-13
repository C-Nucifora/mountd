#[cfg(test)]
mod review {
    use super::*;
    use serde_json::{json, Value};

    fn row() -> Value {
        json!({"name":"fixture", "state":"mounted via fixture.invalid",
            "mountpoint":"~/synthetic/fixture", "state_kind":"mounted",
            "mounted_via":"fixture.invalid", "last_error":null})
    }

    fn envelope(rows: Value) -> Response {
        serde_json::from_value(json!({"ok":true, "status":rows})).unwrap()
    }

    fn engine(names: &[&str]) -> Engine {
        Engine {
            cfg: RwLock::new(FixtureConfig {
                mounts: names
                    .iter()
                    .map(|name| FixtureMount {
                        name: (*name).into(),
                        mountpoint: format!("~/synthetic/{name}"),
                    })
                    .collect(),
            }),
            runtime: Mutex::new(BTreeMap::new()),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn every_lifecycle_variant_survives_status_envelope_and_cli_projection() {
        let cases = vec![
            (MountState::Unknown, "unknown", "unknown", false),
            (MountState::Idle, "idle", "idle (conditions not met)", false),
            (
                MountState::Waiting {
                    reason: "synthetic".into(),
                },
                "waiting",
                "waiting (synthetic)",
                false,
            ),
            (
                MountState::Waking {
                    address: "a".into(),
                },
                "waking",
                "waking (a sent wake-on-LAN)",
                false,
            ),
            (
                MountState::Mounted { via: "a".into() },
                "mounted",
                "mounted via a",
                true,
            ),
            (
                MountState::Adopted {
                    via: Some("a".into()),
                },
                "mounted",
                "mounted via a (adopted)",
                true,
            ),
            (
                MountState::Adopted { via: None },
                "mounted",
                "mounted (adopted)",
                true,
            ),
            (
                MountState::MountedPinned {
                    via: Some("a".into()),
                },
                "mounted",
                "mounted via a (pinned; conditions not met)",
                true,
            ),
            (
                MountState::MountedPinned { via: None },
                "mounted",
                "mounted (pinned; conditions not met)",
                true,
            ),
            (
                MountState::Suspect {
                    via: "a".into(),
                    failures: 2,
                },
                "suspect",
                "suspect (via a, 2 failed probes)",
                true,
            ),
            (
                MountState::Recovering { via: "a".into() },
                "recovering",
                "recovering (was via a)",
                false,
            ),
            (
                MountState::Migrating {
                    from: "a".into(),
                    to: "b".into(),
                },
                "migrating",
                "migrating (a -> b)",
                false,
            ),
            (
                MountState::Unmounted { manual: true },
                "unmounted",
                "unmounted (manual, until conditions change)",
                false,
            ),
            (
                MountState::Unmounted { manual: false },
                "unmounted",
                "unmounted",
                false,
            ),
            (
                MountState::UnmountedConditionsGone,
                "unmounted",
                "unmounted (conditions gone)",
                false,
            ),
            (
                MountState::Error {
                    retry_secs: u64::MAX,
                },
                "error",
                "error (retry in 18446744073709551615s)",
                false,
            ),
        ];
        let eng = engine(&["fixture"]);
        for (state, kind, display, mounted) in cases {
            assert_eq!(state.is_mounted(), mounted);
            eng.runtime.lock().await.insert(
                "fixture".into(),
                MountRuntime {
                    state,
                    mounted_via: Some("source independent of display".into()),
                    last_error: Some("synthetic detail".into()),
                    ..Default::default()
                },
            );
            let status = eng.status().await;
            assert_eq!(status[0].state_kind, kind);
            assert_eq!(status[0].state, display);
            let value = serde_json::to_value(&status).unwrap();
            assert_eq!(value[0].as_object().unwrap().len(), 6);
            let wire = serde_json::to_string(&envelope(value.clone())).unwrap();
            let output = cli_json(serde_json::from_str(&wire).unwrap()).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), value);
            assert_eq!(
                serde_json::from_str::<Vec<MountStatus>>(&output).unwrap(),
                status
            );
        }
        println!("16 lifecycle cases, exact display/kind, six-field envelope/array preservation");
    }

    #[test]
    fn legacy_omissions_and_nullable_details_preserve_rows() {
        for mask in 0..8 {
            let mut value = row();
            for (bit, key) in ["state_kind", "mounted_via", "last_error"]
                .iter()
                .enumerate()
            {
                if mask & (1 << bit) != 0 {
                    value.as_object_mut().unwrap().remove(*key);
                }
            }
            let statuses: Vec<MountStatus> = serde_json::from_value(json!([value])).unwrap();
            assert_eq!(
                statuses[0].state_kind,
                if mask & 1 == 0 { "mounted" } else { "" }
            );
            assert_eq!(
                statuses[0].mounted_via.as_deref(),
                if mask & 2 == 0 {
                    Some("fixture.invalid")
                } else {
                    None
                }
            );
            assert!(statuses[0].last_error.is_none());
            let output = cli_json(envelope(serde_json::to_value(&statuses).unwrap())).unwrap();
            assert_eq!(
                serde_json::from_str::<Vec<MountStatus>>(&output).unwrap(),
                statuses
            );
        }
        for key in ["mounted_via", "last_error"] {
            let mut value = row();
            value[key] = Value::Null;
            assert!(serde_json::from_value::<MountStatus>(value).is_ok());
        }
        println!("8 omission combinations and 2 nullable detail cases");
    }

    #[test]
    fn invalid_rows_reject_the_whole_array_and_do_not_poison_later_decodes() {
        let mut malformed = Vec::new();
        for key in ["name", "state", "mountpoint"] {
            let mut missing = row();
            missing.as_object_mut().unwrap().remove(key);
            malformed.push(missing);
            for wrong in [Value::Null, json!(7), json!({}), json!([]), json!(false)] {
                let mut value = row();
                value[key] = wrong;
                malformed.push(value);
            }
        }
        for key in ["state_kind", "mounted_via", "last_error"] {
            for wrong in [json!(7), json!({}), json!([]), json!(false)] {
                let mut value = row();
                value[key] = wrong;
                malformed.push(value);
            }
        }
        let mut null_kind = row();
        null_kind["state_kind"] = Value::Null;
        malformed.push(null_kind);
        assert_eq!(malformed.len(), 31);
        for invalid in malformed {
            for rows in [json!([invalid.clone(), row()]), json!([row(), invalid])] {
                assert!(
                    serde_json::from_value::<Response>(json!({"ok":true, "status":rows})).is_err()
                );
                assert!(cli_json(envelope(json!([row()]))).is_ok());
            }
        }
        for raw in ["", "[", "not json", "null", "{}", "[null]", "[] trailing"] {
            assert!(serde_json::from_str::<Vec<MountStatus>>(raw).is_err());
            assert!(serde_json::from_str::<Vec<MountStatus>>("[]")
                .unwrap()
                .is_empty());
        }
        println!(
            "62 mixed-array refusals with subsequent valid decode; 7 malformed array documents"
        );
    }

    #[test]
    fn empty_success_is_distinct_from_missing_status_and_daemon_failure() {
        assert_eq!(cli_json(envelope(json!([]))).unwrap(), "[]");
        for value in [json!({"ok":true}), json!({"ok":true,"status":null})] {
            assert_eq!(
                cli_json(serde_json::from_value(value).unwrap())
                    .unwrap_err()
                    .to_string(),
                "daemon response missing status"
            );
        }
        for status in [Value::Null, json!([]), json!([row()])] {
            let response = serde_json::from_value(
                json!({"ok":false,"error":"synthetic failure","status":status}),
            )
            .unwrap();
            assert_eq!(
                cli_json(response).unwrap_err().to_string(),
                "synthetic failure"
            );
        }
        assert_eq!(
            cli_json(serde_json::from_value(json!({"ok":false})).unwrap())
                .unwrap_err()
                .to_string(),
            "unknown daemon error"
        );
        for bad in [
            json!({"status":[]}),
            json!({"ok":null,"status":[]}),
            json!({"ok":"true","status":[]}),
            json!({"ok":true,"status":{}}),
        ] {
            assert!(serde_json::from_value::<Response>(bad).is_err());
        }
        println!("11 envelope distinctions, including failure with a plausible status payload");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_projection_keeps_config_order_and_returns_detached_values() {
        let eng = engine(&["z", "a"]);
        for name in ["orphan", "z"] {
            eng.runtime.lock().await.insert(
                name.into(),
                MountRuntime {
                    state: MountState::Suspect {
                        via: "old".into(),
                        failures: 3,
                    },
                    mounted_via: Some("old".into()),
                    last_error: Some("old error".into()),
                    ..Default::default()
                },
            );
        }
        let before = eng.status().await;
        assert_eq!(
            before.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["z", "a"]
        );
        assert_eq!(before[1].state_kind, "unknown");
        assert!(before[1].mounted_via.is_none() && before[1].last_error.is_none());
        eng.runtime
            .lock()
            .await
            .get_mut("z")
            .unwrap()
            .note_mounted("new");
        eng.cfg.write().await.mounts[0].mountpoint = "~/synthetic/new-path".into();
        let after = eng.status().await;
        assert_eq!(after[0].state_kind, "mounted");
        assert_eq!(after[0].mounted_via.as_deref(), Some("new"));
        assert!(after[0].last_error.is_none());
        assert_eq!(after[0].mountpoint, "~/synthetic/new-path");
        assert_eq!(before[0].state_kind, "suspect");
        assert_eq!(before[0].mounted_via.as_deref(), Some("old"));
        assert_eq!(before[0].mountpoint, "~/synthetic/z");
        eng.cfg.write().await.mounts.clear();
        assert!(eng.status().await.is_empty());
        println!("orphan omitted, config order/defaults retained, old Rust snapshot detached, empty config empty");
    }

    #[test]
    fn unknown_discriminants_and_escaped_payloads_survive_without_reinterpretation() {
        for kind in ["unknown", "", "future-state", "mounted"] {
            let mut value = row();
            value["state_kind"] = json!(kind);
            value["state"] = json!("waiting (display deliberately disagrees)");
            value["name"] = json!("quote\" slash\\ café 日本 😀");
            value["mountpoint"] = json!("~/synthetic/café 日本");
            value["last_error"] = json!("synthetic\nsecond line\tquoted \"detail\"");
            value["future_field"] = json!({"allowed":"additive"});
            let output = cli_json(envelope(json!([value.clone()]))).unwrap();
            value.as_object_mut().unwrap().remove("future_field");
            assert_eq!(
                serde_json::from_str::<Value>(&output).unwrap(),
                json!([value])
            );
        }
        println!("4 discriminant cases with Unicode/escaping and additive field compatibility");
    }
}
