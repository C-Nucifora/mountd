use std::future::Future;
use std::pin::Pin;

pub mod config;
pub mod cred;
pub mod deps;
pub mod engine;
pub mod fingerprint;
pub mod ipc;
pub mod launchd;
pub mod mountcmd;
pub mod mtab;
pub mod ops;
pub mod power;
pub mod probe;
pub mod reconcile;
pub mod state;
pub mod watcher;
pub mod wol;

/// A boxed future. Used by the `Ops` trait so it stays object-safe without
/// pulling in `async-trait`.
pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
