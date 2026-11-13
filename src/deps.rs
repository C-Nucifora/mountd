//! Dependency ("wait-until") checks: run a command, optionally compare a JSON
//! field of its stdout. Used to gate mounts on e.g. Tailscale being up, via
//! the bundled CLI (`Tailscale status --json` → BackendState == "Running").

use std::collections::BTreeMap;
use std::time::Duration;

use crate::config::{Config, DepCheck};

/// Look up a dot-separated path (`a.b.c`) in a JSON value.
pub fn json_lookup<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = value;
    for key in path.split('.') {
        cur = cur.get(key)?;
    }
    Some(cur)
}

/// Compare a JSON value against the configured `equals` string. Strings
/// compare by content; other scalars by their JSON rendering.
pub fn json_equals(value: &serde_json::Value, want: &str) -> bool {
    match value {
        serde_json::Value::String(s) => s == want,
        // Value == &str would only ever match JSON strings; non-string scalars
        // (true, 42) must compare by their JSON rendering.
        #[allow(clippy::cmp_owned)]
        other => other.to_string() == want,
    }
}

/// Run one dependency check. Up = command exits 0 within its timeout and, if
/// `json_path` is set, the value there equals `equals`.
pub async fn check(dep: &DepCheck) -> bool {
    let mut command = tokio::process::Command::new(&dep.cmd);
    command.args(&dep.args);
    let out = match crate::mountcmd::command_output(
        command,
        Duration::from_secs(dep.timeout_secs),
        std::sync::Arc::new(()),
    )
    .await
    {
        Ok(out) => out,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    let Some(path) = &dep.json_path else {
        return true;
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return false;
    };
    let Some(found) = json_lookup(&json, path) else {
        return false;
    };
    match &dep.equals {
        Some(want) => json_equals(found, want),
        // json_path without equals: any present, non-null value counts as up
        None => !found.is_null(),
    }
}

/// Evaluate every configured dep concurrently; returns name -> up.
///
/// Genuinely concurrent: each check is spawned, so the total cost is the
/// slowest dep rather than the sum. With a wedged predicate — `tailscale
/// status` after a bad wake, exactly the case this daemon exists for — the
/// difference is every reconcile pass stalling for one timeout instead of all
/// of them added together.
pub async fn check_all(cfg: &Config) -> BTreeMap<String, bool> {
    let mut tasks = tokio::task::JoinSet::new();
    for (name, dep) in &cfg.deps {
        let name = name.clone();
        let dep = dep.clone();
        // Register before spawn: aborting the outer JoinSet does not join a running poll.
        let activity = crate::mountcmd::CommandActivity::begin();
        tasks.spawn(async move {
            let _activity = activity;
            (name, check(&dep).await)
        });
    }

    let mut out = BTreeMap::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((name, up)) => {
                out.insert(name, up);
            }
            // A panicked check is a dep we cannot vouch for, which means down.
            Err(e) => tracing::warn!("dependency check task failed: {e}"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(cmd: &str, args: &[&str]) -> DepCheck {
        DepCheck {
            cmd: cmd.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            json_path: None,
            equals: None,
            timeout_secs: 5,
        }
    }

    #[tokio::test]
    async fn exit_zero_is_up() {
        assert!(check(&dep("/usr/bin/true", &[])).await);
    }

    #[tokio::test]
    async fn exit_nonzero_is_down() {
        assert!(!check(&dep("/usr/bin/false", &[])).await);
    }

    #[tokio::test]
    async fn missing_command_is_down() {
        assert!(!check(&dep("/nonexistent/binary", &[])).await);
    }

    #[tokio::test]
    async fn json_field_match_is_up() {
        let mut d = dep(
            "/bin/sh",
            &[
                "-c",
                r#"echo '{"BackendState":"Running","Self":{"Online":true}}'"#,
            ],
        );
        d.json_path = Some("BackendState".into());
        d.equals = Some("Running".into());
        assert!(check(&d).await);
    }

    #[tokio::test]
    async fn json_field_mismatch_is_down() {
        let mut d = dep("/bin/sh", &["-c", r#"echo '{"BackendState":"Stopped"}'"#]);
        d.json_path = Some("BackendState".into());
        d.equals = Some("Running".into());
        assert!(!check(&d).await);
    }

    #[tokio::test]
    async fn nested_json_path_works() {
        let mut d = dep("/bin/sh", &["-c", r#"echo '{"Self":{"Online":true}}'"#]);
        d.json_path = Some("Self.Online".into());
        d.equals = Some("true".into());
        assert!(check(&d).await);
    }

    #[tokio::test]
    async fn invalid_json_is_down() {
        let mut d = dep("/bin/sh", &["-c", "echo not-json"]);
        d.json_path = Some("BackendState".into());
        assert!(!check(&d).await);
    }

    #[tokio::test]
    async fn timeout_bounds_hanging_command() {
        let mut d = dep("/bin/sleep", &["30"]);
        d.timeout_secs = 1;
        let start = std::time::Instant::now();
        assert!(!check(&d).await);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn deps_are_checked_concurrently() {
        let toml = r#"
[deps.slow_a]
cmd = "/bin/sleep"
args = ["1"]

[deps.slow_b]
cmd = "/bin/sleep"
args = ["1"]
"#;
        let cfg = Config::from_toml_str(toml).unwrap();
        let start = std::time::Instant::now();
        let up = check_all(&cfg).await;
        let elapsed = start.elapsed();

        assert_eq!(up.len(), 2);
        assert!(up["slow_a"] && up["slow_b"]);
        assert!(
            elapsed < Duration::from_millis(1800),
            "two 1s deps must overlap, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cancellation_kills_and_reaps_dependency_child() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("fake-dep");
        let pidfile = dir.path().join("pid");
        std::fs::write(
            &program,
            "#!/bin/sh\necho $$ > \"$1\"\nexec /bin/sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut cfg = Config::from_toml_str("").unwrap();
        cfg.deps.insert(
            "fake".into(),
            dep(program.to_str().unwrap(), &[pidfile.to_str().unwrap()]),
        );
        let task = tokio::spawn(async move { check_all(&cfg).await });
        let pid: i32 = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(&pidfile).await {
                    if let Ok(pid) = text.trim().parse() {
                        break pid;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        let gone = tokio::time::timeout(Duration::from_secs(2), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
        if gone.is_err() {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(gone.is_ok(), "cancelled dependency child was not reaped");
    }

    #[test]
    fn json_lookup_paths() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"a":{"b":{"c":42}},"x":null}"#).unwrap();
        assert_eq!(json_lookup(&v, "a.b.c"), Some(&serde_json::json!(42)));
        assert_eq!(json_lookup(&v, "a.missing"), None);
        assert!(json_lookup(&v, "x").unwrap().is_null());
    }
}
