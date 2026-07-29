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

/// Find the mount entry whose mountpoint is exactly `path`.
pub fn find_by_mountpoint<'a>(entries: &'a [MountEntry], path: &str) -> Option<&'a MountEntry> {
    entries.iter().find(|e| e.on == path)
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
}
