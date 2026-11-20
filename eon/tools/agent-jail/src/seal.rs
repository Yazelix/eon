//! The harness layer never follows a symbolic link in the workspace.
//!
//! The jail hook checks a file tool's path for links, but the harness opens
//! the path later, in its own process, and an inner-layer command could swap
//! a checked file for a link to the credential in between. So the kernel
//! enforces the rule instead: before it starts the harness, init moves into
//! a user and mount namespace of its own and remounts every mount at or
//! under the workspace `nosymfollow`. The harness inherits that mount
//! namespace without capabilities in it, so it can neither follow a
//! workspace link nor clear the flag. The inner layer is a separate mount
//! namespace, which would inherit the flag; init first binds the workspace,
//! unflagged, at [`crate::INNER_WORKSPACE`] (in the private directory, which
//! the hook never allows and the inner layer masks once it has bound it back
//! over the workspace), so commands still see ordinary links.

use rustix::mount::MountFlags;

use crate::Result;

/// Mounts at or under `root` in a `/proc/self/mountinfo` text, with the
/// per-mount flags a bind remount must carry over (a user namespace may not
/// clear locked flags such as `nosuid`).
pub fn mounts_under(mountinfo: &str, root: &str) -> Vec<(String, MountFlags, bool)> {
    let mut found = Vec::new();
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let (Some(point), Some(options)) = (fields.get(4), fields.get(5)) else {
            continue;
        };
        let point = unescape(point);
        if point != root && !point.starts_with(&format!("{root}/")) {
            continue;
        }
        let mut flags = MountFlags::empty();
        let mut nosymfollow = false;
        for option in options.split(',') {
            flags |= match option {
                "ro" => MountFlags::RDONLY,
                "nosuid" => MountFlags::NOSUID,
                "nodev" => MountFlags::NODEV,
                "noexec" => MountFlags::NOEXEC,
                "noatime" => MountFlags::NOATIME,
                "nodiratime" => MountFlags::NODIRATIME,
                "relatime" => MountFlags::RELATIME,
                "strictatime" => MountFlags::STRICTATIME,
                "nosymfollow" => {
                    nosymfollow = true;
                    MountFlags::empty()
                }
                _ => MountFlags::empty(),
            };
        }
        found.push((point, flags, nosymfollow));
    }
    found
}

/// mountinfo escapes space, tab, newline and backslash as `\NNN` octal.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let octal = bytes.get(index + 1..index + 4).filter(|digits| {
            bytes[index] == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        });
        if let Some(digits) = octal {
            out.push(
                digits
                    .iter()
                    .fold(0u8, |value, digit| value * 8 + (digit - b'0')),
            );
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Seals `root`: a private user and mount namespace, an unflagged recursive
/// bind of `root` at `inner` for the inner layer, then a `nosymfollow` bind
/// remount of every mount at or under `root`. Init calls this first, while
/// it is still single-threaded.
pub fn seal(root: &str, inner: &str) -> Result {
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    // `unshare` is deprecated only because UnshareFlags::FILES can strand
    // other threads' descriptors; these flags cannot, and init has no
    // other thread yet.
    #[allow(deprecated)]
    rustix::thread::unshare(
        rustix::thread::UnshareFlags::NEWUSER | rustix::thread::UnshareFlags::NEWNS,
    )
    .map_err(|err| format!("unshare: {err}"))?;
    let write = |path: &str, text: String| {
        std::fs::write(path, text).map_err(|err| format!("{path}: {err}"))
    };
    write("/proc/self/setgroups", "deny".into())?;
    write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    std::fs::create_dir_all(inner).map_err(|err| format!("{inner}: {err}"))?;
    rustix::mount::mount_bind_recursive(root, inner)
        .map_err(|err| format!("{inner}: bind: {err}"))?;
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|err| format!("/proc/self/mountinfo: {err}"))?;
    let mounts = mounts_under(&mountinfo, root);
    if mounts.is_empty() {
        return Err(format!("{root} is not a mount"));
    }
    for (point, flags, _) in mounts {
        rustix::mount::mount_remount(
            point.as_str(),
            flags | MountFlags::BIND | MountFlags::NOSYMFOLLOW,
            "",
        )
        .map_err(|err| format!("{point}: nosymfollow: {err}"))?;
    }
    Ok(())
}

/// Every mount at or under `root` is `nosymfollow` (guard condition 9).
pub fn sealed(mountinfo: &str, root: &str) -> std::result::Result<(), String> {
    let mounts = mounts_under(mountinfo, root);
    if mounts.is_empty() {
        return Err(format!("{root} is not a mount"));
    }
    match mounts.iter().find(|(_, _, nosymfollow)| !nosymfollow) {
        Some((point, _, _)) => Err(format!("{point} follows symbolic links")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
1 0 0:1 / / ro,nosuid,nodev,relatime - tmpfs tmpfs rw
2 1 259:5 /run/ws /bench/workspace rw,nosuid,nodev,noatime,nosymfollow master:1 - ext4 /dev/x rw
3 2 259:5 /run/ws/.claude /bench/workspace/.claude ro,nosuid,nodev,noatime - ext4 /dev/x rw
4 1 259:5 /run/other /bench/workspace2 rw,nosuid,nodev - ext4 /dev/x rw
5 2 259:5 /run/a\\040b /bench/workspace/a\\040b rw,nosuid,nodev,noatime,nosymfollow - ext4 /dev/x rw";

    #[test]
    fn mounts_under_the_workspace_keep_their_locked_flags() {
        let mounts = mounts_under(MOUNTINFO, "/bench/workspace");
        let points: Vec<&str> = mounts.iter().map(|(point, _, _)| point.as_str()).collect();
        assert_eq!(
            points,
            [
                "/bench/workspace",
                "/bench/workspace/.claude",
                "/bench/workspace/a b"
            ]
        );
        assert_eq!(
            mounts[1].1,
            MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOATIME
        );
    }

    #[test]
    fn a_mount_that_follows_links_fails_the_seal_check() {
        assert_eq!(
            sealed(MOUNTINFO, "/bench/workspace"),
            Err("/bench/workspace/.claude follows symbolic links".into())
        );
        let all = MOUNTINFO.replace(
            "ro,nosuid,nodev,noatime -",
            "ro,nosuid,nodev,noatime,nosymfollow -",
        );
        assert_eq!(sealed(&all, "/bench/workspace"), Ok(()));
        assert!(sealed(MOUNTINFO, "/bench/elsewhere").is_err());
    }
}
