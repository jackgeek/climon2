//! #133 / ADR 0011: only a stable Linux machine ID is part of an origin,
//! through the injectable sources `origin::current_at` reads.
#![cfg(target_os = "linux")]
use climon2_platform::origin::{Sources, current_at};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

/// systemd's machine ID, as `/etc/machine-id` shows it.
const ETC: &str = "8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f";
/// systemd's transient ID: `/run/machine-id` mounted over `/etc/machine-id`.
const TRANSIENT: &str =
    "35 25 0:30 /machine-id /etc/machine-id ro,relatime shared:12 - tmpfs tmpfs rw,mode=755";
/// A container's bind mount of its host's real file.
const BIND: &str = "612 600 8:1 /etc/machine-id /etc/machine-id ro,relatime - ext4 /dev/sda1 rw";
const ROOT: &str = "25 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw";

/// Sources in `directory`: `/etc/machine-id` holds `machine` (missing when
/// `None`) and the mounts are `mounts`.
fn fixture(directory: &Path, machine: Option<&str>, mounts: &[&str]) -> Sources {
    let sources = Sources {
        machine_id: directory.join("machine-id"),
        mountinfo: directory.join("mountinfo"),
        pid_namespace: directory.join("pid"),
    };
    let _ = fs::remove_file(&sources.machine_id);
    if let Some(machine) = machine {
        fs::write(&sources.machine_id, format!("{machine}\n")).unwrap();
    }
    fs::write(&sources.mountinfo, mounts.join("\n") + "\n").unwrap();
    fs::write(&sources.pid_namespace, "").unwrap();
    sources
}

/// The machine ID is not stable, and so absent, when the last mount that
/// holds `/etc/machine-id` (the file, `/etc` or the root) is a tmpfs or an
/// overlay: systemd's transient `/run/machine-id` mount, or the root of a live
/// system. A bind mount of a real file over a volatile root, or a real `/etc`,
/// keeps it, and a later mount at the same place decides. A missing or
/// malformed `/etc/machine-id` is absent too, and the PID namespace is the
/// inode of its file.
#[test]
fn issue133_only_a_stable_machine_id_is_part_of_the_origin() {
    let overlay_root = "1 0 0:30 / / rw,relatime - overlay overlay rw,lowerdir=/l";
    let tmpfs_root = "1 0 0:31 / / rw - tmpfs tmpfs rw";
    let tmpfs_etc = "40 25 0:32 / /etc rw - tmpfs tmpfs rw";
    let real_etc = "40 25 8:2 / /etc rw - ext4 /dev/sda2 rw";
    let other_root = "612 600 8:1 /machine-id /etc/machine-id ro - ext4 /dev/sda1 rw";
    let tmpfs_file = "612 600 0:5 /etc/machine-id /etc/machine-id ro - tmpfs tmpfs rw";
    let elsewhere = "50 25 0:33 / /etc/machine-id.d rw - tmpfs tmpfs rw";
    for (mounts, transient) in [
        (vec![ROOT, TRANSIENT], true),
        (vec![ROOT, BIND, TRANSIENT], true),
        (vec!["not a mount", TRANSIENT], true),
        (vec![overlay_root], true),
        (vec![tmpfs_root], true),
        (vec![ROOT, tmpfs_etc], true),
        (vec![ROOT, tmpfs_file], true),
        (vec![ROOT, TRANSIENT, overlay_root], true),
        (vec![ROOT], false),
        (vec![], false),
        (vec![ROOT, BIND], false),
        (vec![ROOT, TRANSIENT, BIND], false),
        (vec![overlay_root, BIND], false),
        (vec![tmpfs_root, real_etc], false),
        (vec![ROOT, other_root], false),
        (vec![ROOT, elsewhere], false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let sources = fixture(directory.path(), Some(ETC), &mounts);
        let origin = current_at(&sources).unwrap();
        let expected = (!transient).then(|| ETC.to_owned());
        assert_eq!(origin.machine_id.map(String::from), expected, "{mounts:?}");
        let inode = fs::metadata(&sources.pid_namespace).unwrap().ino();
        assert_eq!(origin.pid_namespace, Some(inode));
    }
    let directory = tempfile::tempdir().unwrap();
    for machine in [None, Some(""), Some("not a machine id")] {
        let sources = fixture(directory.path(), machine, &[ROOT]);
        assert_eq!(
            current_at(&sources).unwrap().machine_id,
            None,
            "{machine:?}"
        );
    }
    let unreadable = Sources {
        mountinfo: directory.path().join("no-mountinfo"),
        ..fixture(directory.path(), Some(ETC), &[])
    };
    let machine = current_at(&unreadable).unwrap().machine_id;
    assert_eq!(machine.map(String::from).as_deref(), Some(ETC));
    let no_namespace = Sources {
        pid_namespace: directory.path().join("no-namespace"),
        ..unreadable
    };
    assert!(current_at(&no_namespace).is_err());
}
