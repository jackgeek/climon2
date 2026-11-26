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

/// Only systemd's mount of `/run/machine-id` makes the machine ID transient,
/// and so absent: not a bind mount of a real file, not a mount with another
/// root or file system, and not a mount that a later one at the same place
/// hides. A missing `/etc/machine-id` is absent too, and the PID namespace is
/// the inode of its file.
#[test]
fn issue133_only_a_stable_machine_id_is_part_of_the_origin() {
    let other_root = "612 600 8:1 /machine-id /etc/machine-id ro - ext4 /dev/sda1 rw";
    let other_kind = "612 600 0:5 /etc/machine-id /etc/machine-id ro - tmpfs tmpfs rw";
    for (mounts, transient) in [
        (vec![ROOT, TRANSIENT], true),
        (vec![ROOT, BIND, TRANSIENT], true),
        (vec!["not a mount", TRANSIENT], true),
        (vec![ROOT], false),
        (vec![], false),
        (vec![ROOT, BIND], false),
        (vec![ROOT, TRANSIENT, BIND], false),
        (vec![other_root], false),
        (vec![other_kind], false),
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
