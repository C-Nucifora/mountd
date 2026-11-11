//! Mount/unmount execution. Shells out to the system tools:
//!   SMB: `mount_smbfs -N -o soft` with a Keychain-fetched credential in the URL
//!   NFS: `mount_nfs` with a safe soft-mount option set
//! Unmounts escalate politely by default (`diskutil unmount` → `umount`) and
//! only reach for force (`diskutil unmount force` → `umount -f`) when the
//! caller says so — a user asked, or the server is provably gone.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

use anyhow::{bail, Context, Result};

use crate::config::{MountSpec, Protocol};

/// Bound on a single mount attempt. Once the probe gate has passed, SMB
/// negotiation either completes or fails quickly; a longer window mostly
/// serves to keep the credential visible in `ps` (see `smb_source`).
const MOUNT_TIMEOUT: Duration = Duration::from_secs(15);
const UNMOUNT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Creating the mountpoint directory touches the filesystem at that path,
/// which can block if a stale mount is still covering it.
const MKDIR_TIMEOUT: Duration = Duration::from_secs(5);

/// Expand a leading `~` or `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Normalize without touching the filesystem, including dead mount ancestors.
/// Symlink components are rejected during bounded mountpoint preparation.
pub fn normalize_mountpoint(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in expand_tilde(path).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Percent-encode a URL component (RFC 3986 unreserved characters pass).
pub fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Render an endpoint address for use in a URL authority: IPv6 literals need
/// brackets, everything else passes through.
fn url_host(address: &str) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    }
}

/// Build the SMB source URL: `//[user[:password]@]host/share`.
/// The password rides in the mount argv (mount_smbfs offers no other
/// non-interactive channel — `-N` reads ~/.nsmbrc, not the Keychain), so it is
/// visible to local `ps` for the lifetime of the child, normally bounded by
/// `MOUNT_TIMEOUT`. A kernel-stuck child can exceed that deadline. It is never stored.
pub fn smb_source(
    username: Option<&str>,
    password: Option<&str>,
    address: &str,
    share: &str,
) -> String {
    let host = url_host(address);
    match (username, password) {
        (Some(u), Some(p)) => format!(
            "//{}:{}@{}/{}",
            url_escape(u),
            url_escape(p),
            host,
            url_escape(share)
        ),
        (Some(u), None) => format!("//{}@{}/{}", url_escape(u), host, url_escape(share)),
        (None, _) => format!("//{}/{}", host, url_escape(share)),
    }
}

/// Default mount options per protocol. Both are soft: a dead server must make
/// file operations fail, not block Finder forever.
fn default_options(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Smb => "soft",
        Protocol::Nfs => "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp",
    }
}

/// Custom options add to the safety defaults, never replace them. Validate
/// here as well as at config load so direct callers cannot bypass the policy.
pub fn validated_options(spec: &MountSpec) -> Result<String> {
    let mut options: Vec<String> = default_options(spec.protocol)
        .split(',')
        .map(str::to_owned)
        .collect();
    let crate::config::Probe::Tcp(probe_port) = spec.effective_probe();
    let mut service_port = match spec.protocol {
        Protocol::Smb => 445,
        Protocol::Nfs => 2049,
    };
    let mut explicit_port = None;
    for option in spec
        .options
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if option.chars().any(|c| c.is_whitespace() || c.is_control()) {
            bail!("mount options cannot contain whitespace or control characters");
        }
        let (key, value) = option
            .split_once('=')
            .map_or((option, None), |(k, v)| (k, Some(v)));
        let key = key.to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "bg" | "background"
                | "hard"
                | "nosoft"
                | "nointr"
                | "notcp"
                | "udp"
                | "udp6"
                | "mntudp"
                | "mountport"
        ) {
            bail!("mount option `{key}` conflicts with supervised TCP soft mounts");
        }
        if matches!(
            key.as_str(),
            "localhost"
                | "nolocalhost"
                | "proto"
                | "netid"
                | "inet"
                | "inet6"
                | "noinet"
                | "noinet6"
        ) {
            bail!("mount option {key} can redirect the probed endpoint or address family");
        }
        if key == "port" {
            if spec.protocol != Protocol::Nfs {
                bail!("SMB service port override is unsupported");
            }
            service_port = value
                .and_then(|v| v.parse::<u16>().ok())
                .filter(|p| *p != 0)
                .context("invalid NFS service port")?;
            if explicit_port.is_some_and(|port| port != service_port) {
                bail!("conflicting NFS service ports");
            }
            explicit_port = Some(service_port);
        }
        if matches!(
            key.as_str(),
            "soft" | "intr" | "tcp" | "deadtimeout" | "retrycnt" | "nfc"
        ) {
            if !options.iter().any(|existing| existing == option) {
                bail!("mount option `{key}` cannot override safety defaults");
            }
            continue;
        }
        if !options.iter().any(|existing| existing == option) {
            options.push(option.to_string());
        }
    }
    if service_port != probe_port {
        bail!("TCP probe port must equal mount service port");
    }
    Ok(options.join(","))
}

/// Build the full mount argv for a spec at a chosen endpoint address.
/// `mountpoint` must already be expanded to an absolute path.
pub fn mount_argv(
    spec: &MountSpec,
    address: &str,
    mountpoint: &str,
    password: Option<&str>,
) -> Result<Vec<String>> {
    let options = validated_options(spec)?;
    let mut argv: Vec<String> = Vec::new();
    match spec.protocol {
        Protocol::Smb => {
            let share = spec.share.as_deref().context("smb mount missing `share`")?;
            argv.push("/sbin/mount_smbfs".into());
            // -N: never prompt; the credential comes from the Keychain or the
            // attempt fails fast rather than hanging on an invisible dialog.
            argv.push("-N".into());
            if !options.is_empty() {
                argv.push("-o".into());
                argv.push(options);
            }
            argv.push(smb_source(
                spec.username.as_deref(),
                password,
                address,
                share,
            ));
        }
        Protocol::Nfs => {
            let export = spec
                .export
                .as_deref()
                .context("nfs mount missing `export`")?;
            // Supervise the effect-owning helper, not mount's wrapper process.
            argv.push("/sbin/mount_nfs".into());
            if !options.is_empty() {
                argv.push("-o".into());
                argv.push(options);
            }
            argv.push(format!("{}:{}", url_host(address), export));
        }
    }
    argv.push(mountpoint.into());
    Ok(argv)
}

/// Shared engine guards survive caller cancellation until every owned effect ends.
pub type OperationLease = Arc<dyn Send + Sync>;

static ACTIVE_COMMANDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static COMMAND_FINISHED: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub(crate) struct CommandActivity;
impl CommandActivity {
    pub(crate) fn begin() -> Self {
        ACTIVE_COMMANDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
impl Drop for CommandActivity {
    fn drop(&mut self) {
        ACTIVE_COMMANDS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        COMMAND_FINISHED.notify_waiters();
    }
}

/// Stop and join command producers first; keep the runtime alive until this
/// finishes. Kernel-stuck children/threads deliberately keep shutdown pending.
pub async fn drain_commands() {
    loop {
        let finished = COMMAND_FINISHED.notified();
        tokio::pin!(finished);
        finished.as_mut().enable();
        if ACTIVE_COMMANDS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            return;
        }
        finished.await;
    }
}

const OUTPUT_LIMIT: u64 = 64 * 1024;
const REAP_GRACE: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
#[error("command timed out")]
pub(crate) struct CommandTimedOut;

#[derive(Debug, thiserror::Error)]
#[error("command cleanup pending; operation remains locked")]
pub struct CleanupPending;

async fn read_bounded(reader: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        bail!("command output exceeded limit");
    }
    Ok(bytes)
}

/// A separate supervisor owns the child and lease. Dropping the receiver asks
/// it to kill and reap; dropping the caller never drops the lease prematurely.
pub(crate) async fn command_output(
    mut command: tokio::process::Command,
    deadline: Duration,
    lease: OperationLease,
) -> Result<std::process::Output> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let activity = CommandActivity::begin();
    let mut child = command.spawn().context("spawning command")?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (mut send, receive) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _activity = activity;
        let result = tokio::select! {
            _ = send.closed() => Err(anyhow::anyhow!("command cancelled")),
            _ = tokio::time::sleep(deadline) => Err(CommandTimedOut.into()),
            result = async {
                // Leave the leader unreaped until pipe collection finishes,
                // reserving its PID/PGID for safe process-group cancellation.
                let (stdout, stderr) = tokio::try_join!(read_bounded(stdout), read_bounded(stderr))?;
                let status = child.wait().await?;
                Ok(std::process::Output { status, stdout, stderr })
            } => result,
        };
        if result.is_err() {
            // SIGKILL is required even for a TERM-resistant helper. A kernel
            // wait may still stall, so report promptly but retain the lease.
            if let Some(pid) = child.id() {
                // Only this command's new process group, never the daemon's.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.start_kill();
            let reap = async {
                loop {
                    if child.wait().await.is_ok() {
                        break;
                    }
                    tokio::time::sleep(REAP_GRACE).await;
                }
            };
            finish_cleanup(reap, send, result, lease).await;
            return;
        }
        drop(lease);
        let _ = send.send(result);
    });
    receive.await.context("command supervisor stopped")?
}

async fn finish_cleanup(
    reap: impl std::future::Future<Output = ()>,
    send: tokio::sync::oneshot::Sender<Result<std::process::Output>>,
    result: Result<std::process::Output>,
    lease: OperationLease,
) {
    tokio::pin!(reap);
    if tokio::time::timeout(REAP_GRACE, &mut reap).await.is_err() {
        let _ = send.send(Err(CleanupPending.into()));
        reap.await;
    } else {
        let _ = send.send(result);
    }
    drop(lease);
}

async fn run(argv: &[String], deadline: Duration, lease: OperationLease) -> Result<()> {
    let mut command = tokio::process::Command::new(&argv[0]);
    command.args(&argv[1..]);
    let out = command_output(command, deadline, lease).await?;
    if !out.status.success() {
        // Helper output and argv may contain even fragments of credentials.
        bail!("{} failed ({})", argv[0], out.status);
    }
    Ok(())
}

/// Mount `spec` using `address`, creating the mountpoint directory if needed.
///
/// Refuses to mount over a path that already carries a mount: stacking a second
/// mount on one mountpoint takes two unmounts to clear and confuses every
/// subsequent reconcile.
pub async fn mount(spec: &MountSpec, address: &str, password: Option<&str>) -> Result<()> {
    mount_owned(spec, address, password, Arc::new(())).await
}

pub async fn mount_owned(
    spec: &MountSpec,
    address: &str,
    password: Option<&str>,
    lease: OperationLease,
) -> Result<()> {
    prepare_mount(spec, lease.clone()).await?;
    mount_prepared(spec, address, password, lease).await
}

/// Preparation may block. The engine must refresh eligibility and probes after it.
pub async fn prepare_mount(spec: &MountSpec, lease: OperationLease) -> Result<()> {
    let mp = normalize_mountpoint(&spec.mountpoint);
    check_mount_target(&mp)?;
    ensure_mountpoint(&mp, lease).await
}

/// No filesystem preparation may be added after the engine's final gate.
pub async fn mount_prepared(
    spec: &MountSpec,
    address: &str,
    password: Option<&str>,
    lease: OperationLease,
) -> Result<()> {
    let mp = normalize_mountpoint(&spec.mountpoint);
    check_mount_target(&mp)?;
    let argv = mount_argv(spec, address, &mp.to_string_lossy(), password)?;
    run(&argv, MOUNT_TIMEOUT, lease).await
}

fn check_mount_target(mp: &Path) -> Result<()> {
    let entries = crate::mtab::list().context("reading mount table")?;
    if entries
        .iter()
        .any(|entry| paths_may_alias(mp, Path::new(&entry.on)))
    {
        bail!("mountpoint is already mounted or aliases an existing mount");
    }
    if entries.iter().any(|entry| {
        matches!(entry.fstype.as_str(), "nfs" | "smbfs")
            && path_may_be_beneath(mp, Path::new(&entry.on))
    }) {
        bail!("mountpoint may be beneath a network mount");
    }
    Ok(())
}

/// Refuse possible aliases, never use this comparison to select an unmount.
/// Non-ASCII components are conservatively ambiguous across filesystem Unicode rules.
pub fn path_may_be_beneath(path: &Path, ancestor: &Path) -> bool {
    let mut path = path.components();
    ancestor.components().all(|outer| {
        path.next().is_some_and(|inner| {
            let a = outer.as_os_str().to_string_lossy();
            let b = inner.as_os_str().to_string_lossy();
            a.eq_ignore_ascii_case(&b) || !a.is_ascii() || !b.is_ascii()
        })
    })
}

pub fn paths_may_alias(a: &Path, b: &Path) -> bool {
    a.components().count() == b.components().count() && path_may_be_beneath(a, b)
}

/// Bound the caller wait while retaining ownership of filesystem preparation.
async fn ensure_mountpoint(mp: &Path, lease: OperationLease) -> Result<()> {
    let path = mp.to_path_buf();
    let (send, receive) = tokio::sync::oneshot::channel();
    // Filesystem syscalls cannot be cancelled. A plain thread avoids tying
    // runtime shutdown to Tokio's blocking pool and retains the engine guards.
    let activity = CommandActivity::begin();
    std::thread::Builder::new()
        .name("mountpoint".into())
        .spawn(move || {
            let _activity = activity;
            let result = prepare_directory(&path);
            drop(lease);
            let _ = send.send(result);
        })
        .context("starting mountpoint preparation")?;
    match tokio::time::timeout(MKDIR_TIMEOUT, receive).await {
        Ok(Ok(result)) => result.with_context(|| format!("creating mountpoint {}", mp.display())),
        Ok(Err(_)) => bail!("mountpoint preparation stopped"),
        Err(_) => Err(CleanupPending.into()),
    }
}

/// Walk from an open root directory, never following symlinks while creating.
fn prepare_directory(mp: &Path) -> Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;
    if !mp.is_absolute() {
        bail!("mountpoint must be absolute");
    }
    let root = std::fs::File::open("/")?;
    let mut parent: OwnedFd = root.into();
    for component in mp.components() {
        let Component::Normal(name) = component else {
            if component == Component::RootDir {
                continue;
            }
            bail!("mountpoint must be normalized");
        };
        let name = std::ffi::CString::new(name.as_bytes())?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if made < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error().into());
            }
            fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        }
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent = unsafe { OwnedFd::from_raw_fd(fd) };
        #[cfg(target_os = "macos")]
        {
            // F_GETPATH reads the opened vnode's spelling without walking a dead mount.
            let mut actual = [0 as libc::c_char; libc::PATH_MAX as usize];
            if unsafe { libc::fcntl(parent.as_raw_fd(), libc::F_GETPATH, actual.as_mut_ptr()) } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let actual = unsafe { std::ffi::CStr::from_ptr(actual.as_ptr()) };
            let actual = Path::new(std::ffi::OsStr::from_bytes(actual.to_bytes()));
            if actual.file_name().map(|part| part.as_bytes()) != Some(name.as_bytes()) {
                bail!("mountpoint component does not use the filesystem's stored spelling");
            }
        }
    }
    Ok(())
}

/// Unmount with escalation. Polite rungs always run first; `force` adds the
/// rungs that can yank a volume out from under an open file, so it is reserved
/// for explicit user requests and provably dead servers.
/// Returns Ok if any rung succeeds.
pub async fn unmount(mountpoint: &str, force: bool) -> Result<()> {
    unmount_owned(mountpoint, force, Arc::new(())).await
}

pub async fn unmount_owned(mountpoint: &str, force: bool, lease: OperationLease) -> Result<()> {
    let mp = normalize_mountpoint(mountpoint);
    let mp = mp.to_string_lossy().to_string();
    if crate::mtab::find_by_mountpoint(&crate::mtab::list().context("reading mount table")?, &mp)
        .is_none()
    {
        bail!("not mounted at {mp}");
    }
    let mut ladder = unmount_ladder(&mp, force);

    let mut last_err = None;
    for argv in ladder.drain(..) {
        match run(&argv, UNMOUNT_STEP_TIMEOUT, lease.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) if e.is::<CleanupPending>() => return Err(e),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no unmount rung ran for {mp}")))
}

/// The escalation ladder for a mountpoint. Split out so the policy — polite
/// rungs always, forceful rungs only on request — is directly testable.
fn unmount_ladder(mp: &str, force: bool) -> Vec<Vec<String>> {
    let mut ladder: Vec<Vec<String>> = vec![
        vec!["/usr/sbin/diskutil".into(), "unmount".into(), mp.into()],
        vec!["/sbin/umount".into(), mp.into()],
    ];
    if force {
        ladder.push(vec![
            "/usr/sbin/diskutil".into(),
            "unmount".into(),
            "force".into(),
            mp.into(),
        ]);
        ladder.push(vec!["/sbin/umount".into(), "-f".into(), mp.into()]);
    }
    ladder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Protocol};

    fn specs() -> Config {
        Config::from_toml_str(
            r#"
[[mount]]
name = "smb1"
protocol = "smb"
share = "media library"
username = "chris"
mountpoint = "~/mnt/smb1"

  [[mount.endpoint]]
  address = "192.168.1.5"

[[mount]]
name = "nfs1"
protocol = "nfs"
export = "/mnt/tank/scratch"
mountpoint = "~/mnt/nfs1"

  [[mount.endpoint]]
  address = "100.64.0.11"
"#,
        )
        .unwrap()
    }

    async fn wait_for_pid(path: &Path) -> i32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(pid) = text.trim().parse() {
                        return pid;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    struct ReapedLease {
        pid: std::sync::atomic::AtomicI32,
        released: Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for ReapedLease {
        fn drop(&mut self) {
            let pid = self.pid.load(std::sync::atomic::Ordering::SeqCst);
            assert!(pid > 0);
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                -1,
                "lease released before reap"
            );
            self.released
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cancellation_reaps_term_resistant_child_before_lease_release() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lease = Arc::new(ReapedLease {
            pid: 0.into(),
            released: released.clone(),
        });
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; echo $$ > \"$1\"; while :; do :; done",
                "fixture",
            ])
            .arg(&pidfile);
        let task = tokio::spawn(command_output(
            command,
            Duration::from_secs(30),
            lease.clone(),
        ));
        lease.pid.store(
            wait_for_pid(&pidfile).await,
            std::sync::atomic::Ordering::SeqCst,
        );
        drop(lease);
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !released.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn timeout_kills_and_reaps_term_resistant_child() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; echo $$ > \"$1\"; while :; do :; done",
                "fixture",
            ])
            .arg(&pidfile);
        let task = tokio::spawn(command_output(
            command,
            Duration::from_millis(200),
            Arc::new(()),
        ));
        let pid = wait_for_pid(&pidfile).await;
        assert!(task.await.unwrap().is_err());
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[tokio::test]
    async fn output_flood_is_rejected_and_child_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "echo $$ > \"$1\"; while :; do printf '12345678901234567890123456789012'; done",
                "fixture",
            ])
            .arg(&pidfile);
        let result = command_output(command, Duration::from_secs(2), Arc::new(())).await;
        assert!(format!("{:#}", result.unwrap_err()).contains("output exceeded limit"));
        let pid = wait_for_pid(&pidfile).await;
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[tokio::test]
    async fn inherited_pipes_cannot_hold_command_open() {
        // The helper forks a short-lived pipe holder. It is in our dedicated
        // group, which the timeout kills along with the unreaped leader.
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "/bin/sleep 0.2 & exit 0"]);
        let start = std::time::Instant::now();
        assert!(
            command_output(command, Duration::from_millis(30), Arc::new(()))
                .await
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn stalled_reap_reports_promptly_but_retains_lease() {
        let lease: OperationLease = Arc::new(());
        let retained = Arc::downgrade(&lease);
        let (send, receive) = tokio::sync::oneshot::channel();
        let (release, reaped) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(finish_cleanup(
            async {
                reaped.await.unwrap();
            },
            send,
            Err(anyhow::anyhow!("timeout")),
            lease,
        ));
        let result = tokio::time::timeout(Duration::from_secs(1), receive)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().is::<CleanupPending>());
        assert!(retained.upgrade().is_some());
        release.send(()).unwrap();
        worker.await.unwrap();
        assert!(retained.upgrade().is_none());
    }

    #[tokio::test]
    async fn shutdown_drain_waits_for_registered_work() {
        let activity = CommandActivity::begin();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), drain_commands())
                .await
                .is_err()
        );
        drop(activity);
        tokio::time::timeout(Duration::from_secs(5), drain_commands())
            .await
            .unwrap();
    }

    #[test]
    fn possible_path_aliases_are_refused_without_selecting_a_target() {
        assert!(paths_may_alias(
            Path::new("/local/NAS"),
            Path::new("/local/nas")
        ));
        assert!(path_may_be_beneath(
            Path::new("/local/nas/child"),
            Path::new("/local/NAS")
        ));
        assert!(path_may_be_beneath(
            Path::new("/local/cafe\u{301}/child"),
            Path::new("/local/café")
        ));
        assert!(!path_may_be_beneath(
            Path::new("/local/nas-other"),
            Path::new("/local/nas")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_preparation_refuses_a_case_alias_of_an_opened_directory() {
        use std::os::unix::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        // Only canonicalize a fresh, local fixture, never a configured mount.
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let stored = root.join("NAS");
        let alias = root.join("nas");
        std::fs::create_dir(&stored).unwrap();
        prepare_directory(&stored).unwrap();
        if let Ok(alias_meta) = std::fs::metadata(&alias) {
            assert_eq!(alias_meta.ino(), std::fs::metadata(&stored).unwrap().ino());
            assert!(prepare_directory(&alias).is_err());
        } else {
            prepare_directory(&alias).unwrap();
            assert_ne!(
                std::fs::metadata(&alias).unwrap().ino(),
                std::fs::metadata(&stored).unwrap().ino()
            );
        }
    }

    #[test]
    fn smb_argv_defaults_to_a_soft_mount() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("smb1").unwrap(),
            "192.168.1.5",
            "/Users/u/mnt/smb1",
            None,
        )
        .unwrap();
        assert_eq!(
            argv,
            vec![
                "/sbin/mount_smbfs",
                "-N",
                "-o",
                "soft",
                "//chris@192.168.1.5/media%20library",
                "/Users/u/mnt/smb1",
            ]
        );
    }

    #[test]
    fn smb_argv_embeds_escaped_password() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("smb1").unwrap(),
            "192.168.1.5",
            "/Users/u/mnt/smb1",
            Some("p@ss:w/rd"),
        )
        .unwrap();
        assert!(argv.contains(&"//chris:p%40ss%3Aw%2Frd@192.168.1.5/media%20library".to_string()));
    }

    #[test]
    fn smb_source_without_username() {
        assert_eq!(smb_source(None, None, "host", "share"), "//host/share");
        // password without username is ignored
        assert_eq!(
            smb_source(None, Some("pw"), "host", "share"),
            "//host/share"
        );
    }

    #[test]
    fn ipv6_literals_are_bracketed() {
        assert_eq!(
            smb_source(None, None, "fd7a::1", "share"),
            "//[fd7a::1]/share"
        );
        let cfg = specs();
        let argv = mount_argv(cfg.mount("nfs1").unwrap(), "fd7a::1", "/mp", None).unwrap();
        assert!(argv.contains(&"[fd7a::1]:/mnt/tank/scratch".to_string()));
    }

    #[tokio::test]
    async fn command_failure_omits_argv_and_all_helper_output() {
        let argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '%s' \"$1\"; printf '%s' \"$2\" >&2; exit 1".into(),
            "fixture".into(),
            "//user:synthetic%40secret@host/share".into(),
            "synthetic@secret plus partial fragment %40sec".into(),
        ];
        let error = run(&argv, Duration::from_secs(2), Arc::new(()))
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("failed"));
        assert!(!message.contains("synthetic"));
        assert!(!message.contains("%40sec"));
        assert!(!message.contains("host/share"));
        assert!(!message.contains("printf"));
    }

    #[test]
    fn nfs_argv_defaults_to_soft_mount_options() {
        let cfg = specs();
        let argv = mount_argv(
            cfg.mount("nfs1").unwrap(),
            "100.64.0.11",
            "/Users/u/mnt/nfs1",
            None,
        )
        .unwrap();
        assert_eq!(
            argv,
            vec![
                "/sbin/mount_nfs",
                "-o",
                "soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp",
                "100.64.0.11:/mnt/tank/scratch",
                "/Users/u/mnt/nfs1",
            ]
        );
    }

    #[test]
    fn explicit_options_keep_safety_defaults() {
        let mut cfg = specs();
        let m = &mut cfg.mounts[1];
        m.options = Some("soft,vers=4".into());
        assert!(m.protocol == Protocol::Nfs);
        let argv = mount_argv(m, "h", "/mp", None).unwrap();
        assert!(argv.contains(&"soft,intr,deadtimeout=60,retrycnt=0,nfc,tcp,vers=4".to_string()));
    }

    #[test]
    fn explicit_empty_options_keep_soft_mounts() {
        let mut cfg = specs();
        let m = &mut cfg.mounts[0];
        m.options = Some(String::new());
        let argv = mount_argv(m, "h", "/mp", None).unwrap();
        assert!(argv.contains(&"-o".to_string()));
        assert!(argv.contains(&"soft".to_string()));
    }

    #[test]
    fn unsafe_options_and_mismatched_ports_are_rejected() {
        let mut cfg = specs();
        let spec = &mut cfg.mounts[1];
        for options in [
            "bg",
            "hard",
            "udp",
            "proto=udp",
            "proto=udp6",
            "proto=tcp6",
            "proto=tcp",
            "netid=tcp6",
            "netid=tcp",
            "localhost",
            "inet",
            "inet6",
            "noinet",
            "noinet6",
            "inet,inet6",
            "mntudp",
            "mntudp=1",
            "mountport=2049",
            "port=9999",
            "deadtimeout=0",
            "retrycnt=99",
            "nosoft",
            "bg =1",
            "port =9999",
            "port=9999,port=2049",
        ] {
            spec.options = Some(options.into());
            assert!(mount_argv(spec, "host", "/mp", None).is_err(), "{options}");
        }
        spec.options = Some("vers=4,ro,rsize=65536,port=2049".into());
        assert!(mount_argv(spec, "host", "/mp", None).is_ok());
        spec.gate = Some(crate::config::Gate {
            probe: crate::config::Probe::Tcp(9999),
        });
        assert!(mount_argv(spec, "host", "/mp", None).is_err());
        spec.options = Some("port=9999".into());
        assert!(mount_argv(spec, "host", "/mp", None).is_ok());
        spec.endpoints[0].address = "::1".into();
        spec.options = Some("port=9999,proto=tcp6".into());
        assert!(mount_argv(spec, "::1", "/mp", None).is_err());
        let spec = &mut cfg.mounts[0];
        spec.gate = Some(crate::config::Gate {
            probe: crate::config::Probe::Tcp(9999),
        });
        assert!(mount_argv(spec, "host", "/mp", None).is_err());
    }

    #[test]
    fn tilde_expansion() {
        let p = expand_tilde("~/mnt/x");
        assert!(p.is_absolute());
        assert!(p.ends_with("mnt/x"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert!(expand_tilde("~").is_absolute());
    }

    #[test]
    fn normalize_trims_trailing_slashes() {
        let a = normalize_mountpoint("/tmp/mountd-test-x");
        let b = normalize_mountpoint("/tmp/mountd-test-x/");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_does_not_follow_a_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let via_link = normalize_mountpoint(&format!("{}/share", link.display()));
        let via_real = normalize_mountpoint(&format!("{}/share", real.display()));
        assert_ne!(via_link, via_real);
        // the mountpoint component itself need not exist
        assert!(via_link.ends_with("share"));
    }

    #[test]
    fn normalize_resolves_through_missing_intermediate_directories() {
        // Both of these must agree about the same prefix even though the
        // middle directory does not exist yet — the nesting check in config
        // validation compares them as strings.
        let shallow = normalize_mountpoint("/tmp/mountd-test-nest");
        let deep = normalize_mountpoint("/tmp/mountd-test-nest/inner");
        assert!(
            deep.starts_with(&shallow),
            "{deep:?} should sit under {shallow:?}"
        );
    }

    #[test]
    fn normalize_leaves_a_wholly_unknown_path_alone() {
        let p = normalize_mountpoint("/nonexistent-parent-xyz/share");
        assert_eq!(p, PathBuf::from("/nonexistent-parent-xyz/share"));
    }

    #[test]
    fn normalize_handles_the_root_directory() {
        assert_eq!(normalize_mountpoint("/"), PathBuf::from("/"));
    }

    #[test]
    fn polite_ladder_never_forces() {
        let polite = unmount_ladder("/mp", false);
        assert_eq!(polite.len(), 2);
        assert!(!polite.iter().any(|a| a.contains(&"-f".to_string())));
        assert!(!polite.iter().any(|a| a.contains(&"force".to_string())));
    }

    #[test]
    fn forced_ladder_adds_the_forceful_rungs_last() {
        let forced = unmount_ladder("/mp", true);
        assert_eq!(forced.len(), 4);
        // polite rungs still come first
        assert_eq!(forced[0][1], "unmount");
        assert!(!forced[0].contains(&"force".to_string()));
        assert!(forced[2].contains(&"force".to_string()));
        assert!(forced[3].contains(&"-f".to_string()));
    }

    #[tokio::test]
    async fn mountpoint_rejects_leaf_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let target = base.join("target");
        std::fs::create_dir(&target).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(ensure_mountpoint(&link, Arc::new(())).await.is_err());
        assert!(ensure_mountpoint(&link.join("nested"), Arc::new(()))
            .await
            .is_err());
        assert!(ensure_mountpoint(&target, Arc::new(())).await.is_ok());
        assert!(ensure_mountpoint(&base.join("new"), Arc::new(()))
            .await
            .is_ok());
    }

    #[test]
    fn url_escaping() {
        assert_eq!(url_escape("plain-name_1.2~"), "plain-name_1.2~");
        assert_eq!(url_escape("has space/slash"), "has%20space%2Fslash");
    }
}
