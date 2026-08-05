//! The system-effects boundary.
//!
//! Everything the engine does to the outside world — read the mount table,
//! mount, unmount, ask the Keychain, probe a port, send a magic packet — goes
//! through this trait. `SystemOps` is the real implementation; tests install a
//! fake and can then drive every branch of the state machine (dead mounts,
//! denied credentials, endpoint failover) without a network, a server, or a
//! Keychain prompt. Before this seam existed, engine tests could only reach
//! "no endpoint reachable", which is why none of them caught the bugs in the
//! v0.3 review.

use std::time::Duration;

use anyhow::Result;

use crate::config::MountSpec;
use crate::cred::FetchOutcome;
use crate::mtab::MountEntry;
use crate::wol::WolSpec;
use crate::BoxFut;

/// Async methods take owned arguments and return a boxed future so the trait
/// stays object-safe without an `async-trait` dependency.
pub trait Ops: Send + Sync + 'static {
    fn list_mounts(&self) -> Vec<MountEntry>;

    fn mount(
        &self,
        spec: MountSpec,
        address: String,
        password: Option<String>,
    ) -> BoxFut<Result<()>>;

    /// `force` permits the rungs that can yank a volume from under an open
    /// file. Policy-driven unmounts pass false.
    fn unmount(&self, mountpoint: String, force: bool) -> BoxFut<Result<()>>;

    fn fetch_password(
        &self,
        server: String,
        account: String,
        deadline: Duration,
    ) -> BoxFut<FetchOutcome>;

    fn probe(&self, address: String, port: u16, timeout: Duration) -> BoxFut<bool>;

    fn send_wol(&self, spec: WolSpec) -> BoxFut<Result<()>>;
}

/// The real thing.
pub struct SystemOps;

impl Ops for SystemOps {
    fn list_mounts(&self) -> Vec<MountEntry> {
        crate::mtab::list()
    }

    fn mount(
        &self,
        spec: MountSpec,
        address: String,
        password: Option<String>,
    ) -> BoxFut<Result<()>> {
        Box::pin(async move { crate::mountcmd::mount(&spec, &address, password.as_deref()).await })
    }

    fn unmount(&self, mountpoint: String, force: bool) -> BoxFut<Result<()>> {
        Box::pin(async move { crate::mountcmd::unmount(&mountpoint, force).await })
    }

    fn fetch_password(
        &self,
        server: String,
        account: String,
        deadline: Duration,
    ) -> BoxFut<FetchOutcome> {
        Box::pin(
            async move { crate::cred::fetch_smb_password(&server, &account, None, deadline).await },
        )
    }

    fn probe(&self, address: String, port: u16, timeout: Duration) -> BoxFut<bool> {
        Box::pin(async move { crate::probe::probe_tcp(&address, port, timeout).await })
    }

    fn send_wol(&self, spec: WolSpec) -> BoxFut<Result<()>> {
        Box::pin(async move { crate::wol::send(&spec).await })
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    /// One thing the engine did, in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Call {
        Mount { name: String, address: String },
        Unmount { mountpoint: String, force: bool },
        Probe { address: String },
        FetchPassword { server: String, account: String },
        Wol { mac: String },
    }

    /// A scriptable stand-in for the system.
    #[derive(Default)]
    pub struct FakeState {
        /// Mountpoints the kernel would report, with the source address.
        pub mounted: BTreeMap<String, String>,
        /// Addresses whose probe succeeds. Everything else is unreachable.
        pub reachable: BTreeSet<String>,
        /// Extra latency per address, so priority can be tested against speed.
        pub probe_delay: BTreeMap<String, Duration>,
        /// Addresses whose mount attempt fails, with the error text.
        pub mount_fails: BTreeMap<String, String>,
        /// Mountpoints whose unmount fails unless forced.
        pub unmount_needs_force: BTreeSet<String>,
        /// Keychain answers by server; absent means `Missing`.
        pub passwords: BTreeMap<String, FetchOutcome>,
        pub calls: Vec<Call>,
    }

    #[derive(Clone, Default)]
    pub struct FakeOps(pub Arc<Mutex<FakeState>>);

    impl FakeOps {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with(f: impl FnOnce(&mut FakeState)) -> Arc<Self> {
            let ops = FakeOps::default();
            f(&mut ops.0.lock().unwrap());
            Arc::new(ops)
        }

        pub fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }

        pub fn calls(&self) -> Vec<Call> {
            self.0.lock().unwrap().calls.clone()
        }

        pub fn mount_calls(&self) -> Vec<(String, String)> {
            self.calls()
                .into_iter()
                .filter_map(|c| match c {
                    Call::Mount { name, address } => Some((name, address)),
                    _ => None,
                })
                .collect()
        }

        pub fn unmount_calls(&self) -> Vec<(String, bool)> {
            self.calls()
                .into_iter()
                .filter_map(|c| match c {
                    Call::Unmount { mountpoint, force } => Some((mountpoint, force)),
                    _ => None,
                })
                .collect()
        }

        pub fn wol_calls(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter_map(|c| match c {
                    Call::Wol { mac } => Some(mac),
                    _ => None,
                })
                .collect()
        }

        /// Simulate the server behind `address` going away while mounted.
        pub fn kill_endpoint(&self, address: &str) {
            self.0.lock().unwrap().reachable.remove(address);
        }

        pub fn revive_endpoint(&self, address: &str) {
            self.0.lock().unwrap().reachable.insert(address.to_string());
        }
    }

    impl Ops for FakeOps {
        fn list_mounts(&self) -> Vec<MountEntry> {
            self.0
                .lock()
                .unwrap()
                .mounted
                .iter()
                .map(|(on, from)| MountEntry {
                    fstype: "smbfs".into(),
                    from: from.clone(),
                    on: on.clone(),
                })
                .collect()
        }

        fn mount(
            &self,
            spec: MountSpec,
            address: String,
            _password: Option<String>,
        ) -> BoxFut<Result<()>> {
            let shared = self.0.clone();
            let mountpoint = crate::mountcmd::normalize_mountpoint(&spec.mountpoint)
                .to_string_lossy()
                .to_string();
            Box::pin(async move {
                let mut st = shared.lock().unwrap();
                st.calls.push(Call::Mount {
                    name: spec.name.clone(),
                    address: address.clone(),
                });
                if let Some(err) = st.mount_fails.get(&address).cloned() {
                    return Err(anyhow::anyhow!("{err}"));
                }
                st.mounted
                    .insert(mountpoint, format!("//user@{address}/share"));
                Ok(())
            })
        }

        fn unmount(&self, mountpoint: String, force: bool) -> BoxFut<Result<()>> {
            let shared = self.0.clone();
            Box::pin(async move {
                let mut st = shared.lock().unwrap();
                st.calls.push(Call::Unmount {
                    mountpoint: mountpoint.clone(),
                    force,
                });
                if st.unmount_needs_force.contains(&mountpoint) && !force {
                    return Err(anyhow::anyhow!("resource busy"));
                }
                st.mounted.remove(&mountpoint);
                Ok(())
            })
        }

        fn fetch_password(
            &self,
            server: String,
            account: String,
            _deadline: Duration,
        ) -> BoxFut<FetchOutcome> {
            let shared = self.0.clone();
            Box::pin(async move {
                let mut st = shared.lock().unwrap();
                st.calls.push(Call::FetchPassword {
                    server: server.clone(),
                    account,
                });
                st.passwords
                    .get(&server)
                    .cloned()
                    .unwrap_or(FetchOutcome::Missing)
            })
        }

        fn probe(&self, address: String, _port: u16, _timeout: Duration) -> BoxFut<bool> {
            let shared = self.0.clone();
            Box::pin(async move {
                let (up, delay) = {
                    let mut st = shared.lock().unwrap();
                    st.calls.push(Call::Probe {
                        address: address.clone(),
                    });
                    (
                        st.reachable.contains(&address),
                        st.probe_delay.get(&address).copied(),
                    )
                };
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
                up
            })
        }

        fn send_wol(&self, spec: WolSpec) -> BoxFut<Result<()>> {
            let shared = self.0.clone();
            Box::pin(async move {
                shared
                    .lock()
                    .unwrap()
                    .calls
                    .push(Call::Wol { mac: spec.mac });
                Ok(())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Call, FakeOps};
    use super::*;

    #[tokio::test]
    async fn fake_reports_a_scripted_mount_table() {
        let ops = FakeOps::with(|st| {
            st.mounted
                .insert("/mnt/a".into(), "//user@10.0.0.1/share".into());
        });
        let entries = ops.list_mounts();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].on, "/mnt/a");
        assert!(crate::mtab::find_by_mountpoint(&entries, "/mnt/a").is_some());
    }

    #[tokio::test]
    async fn fake_records_calls_in_order() {
        let ops = FakeOps::with(|st| {
            st.reachable.insert("10.0.0.1".into());
        });
        assert!(
            ops.probe("10.0.0.1".into(), 445, Duration::from_millis(1))
                .await
        );
        assert!(
            !ops.probe("10.0.0.2".into(), 445, Duration::from_millis(1))
                .await
        );
        ops.unmount("/mnt/a".into(), true).await.unwrap();

        assert_eq!(
            ops.calls(),
            vec![
                Call::Probe {
                    address: "10.0.0.1".into()
                },
                Call::Probe {
                    address: "10.0.0.2".into()
                },
                Call::Unmount {
                    mountpoint: "/mnt/a".into(),
                    force: true
                },
            ]
        );
    }

    #[tokio::test]
    async fn fake_busy_mount_needs_force() {
        let ops = FakeOps::with(|st| {
            st.mounted.insert("/mnt/a".into(), "//u@h/s".into());
            st.unmount_needs_force.insert("/mnt/a".into());
        });
        assert!(ops.unmount("/mnt/a".into(), false).await.is_err());
        assert!(ops.unmount("/mnt/a".into(), true).await.is_ok());
        assert!(ops.list_mounts().is_empty());
    }
}
