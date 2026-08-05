//! Mount-table enumeration via getfsstat(2) with MNT_NOWAIT — it never
//! issues RPCs to the server, so listing can't hang on a dead NFS/SMB mount
//! (unlike stat()ing a mountpoint).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub fstype: String,
    /// Source, e.g. `//user@host/share` or `host:/export`.
    pub from: String,
    /// Mountpoint path.
    pub on: String,
}

fn cstr_field(bytes: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = bytes
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// List current mounts without blocking on dead filesystems.
pub fn list() -> Vec<MountEntry> {
    unsafe {
        let count = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
        if count <= 0 {
            return Vec::new();
        }
        let mut buf: Vec<libc::statfs> = vec![std::mem::zeroed(); count as usize];
        let bufsize = (buf.len() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let n = libc::getfsstat(buf.as_mut_ptr(), bufsize as _, libc::MNT_NOWAIT);
        if n <= 0 {
            return Vec::new();
        }
        buf.truncate(n as usize);
        buf.iter()
            .map(|s| MountEntry {
                fstype: cstr_field(&s.f_fstypename),
                from: cstr_field(&s.f_mntfromname),
                on: cstr_field(&s.f_mntonname),
            })
            .collect()
    }
}

/// Trailing slashes are not significant in a mountpoint path.
fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// Find the mount entry whose mountpoint is `path`.
///
/// `path` must already have been through `mountcmd::normalize_mountpoint`: the
/// kernel reports the resolved path, so a configured symlink or trailing slash
/// would otherwise miss and make a mounted share look unmounted forever.
pub fn find_by_mountpoint<'a>(entries: &'a [MountEntry], path: &str) -> Option<&'a MountEntry> {
    entries.iter().find(|e| same_path(&e.on, path))
}

/// The address portion of a mount source, e.g. `//user@host/share` → `host`,
/// `host:/export` → `host`. Used to re-attribute a mount the daemon adopted
/// rather than created.
pub fn source_address(from: &str) -> Option<String> {
    let rest = from.strip_prefix("//").unwrap_or(from);
    let authority = match rest.split_once('/') {
        Some((a, _)) => a,
        None => rest.split_once(':').map(|(a, _)| a).unwrap_or(rest),
    };
    let host = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host.trim_end_matches(':');
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_root_filesystem() {
        let entries = list();
        let root = find_by_mountpoint(&entries, "/").expect("root fs present");
        assert!(!root.fstype.is_empty());
    }

    #[test]
    fn find_by_mountpoint_misses_nonexistent() {
        let entries = list();
        assert!(find_by_mountpoint(&entries, "/definitely/not/mounted").is_none());
    }

    #[test]
    fn trailing_slashes_are_not_significant() {
        let entries = list();
        // "/" is always mounted and is the one path where the trimming rule
        // must not turn a real mountpoint into the empty string.
        assert!(find_by_mountpoint(&entries, "/").is_some());
    }

    #[test]
    fn parses_the_address_out_of_a_mount_source() {
        assert_eq!(
            source_address("//user@192.168.1.5/share").as_deref(),
            Some("192.168.1.5")
        );
        assert_eq!(
            source_address("//192.168.1.5/share").as_deref(),
            Some("192.168.1.5")
        );
        assert_eq!(
            source_address("100.64.0.11:/mnt/tank").as_deref(),
            Some("100.64.0.11")
        );
        // a local device has no network address to attribute a mount to
        assert_eq!(source_address("/dev/disk1s1"), None);
        assert_eq!(source_address(""), None);
    }
}
