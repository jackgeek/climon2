//! #133 / ADR 0005: climon2's own machine ID on Linux, where systemd's is
//! transient, through the injectable sources `origin::current_at` reads.
#![cfg(target_os = "linux")]
use climon2_platform::origin::{Sources, current_at};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

/// systemd's machine ID, as `/etc/machine-id` shows it.
const ETC: &str = "8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f";
/// systemd's transient ID: `/run/machine-id` mounted over `/etc/machine-id`.
const TRANSIENT: &str =
    "35 25 0:30 /machine-id /etc/machine-id ro,relatime shared:12 - tmpfs tmpfs rw,mode=755";
/// A container's bind mount of its host's real file.
const BIND: &str = "612 600 8:1 /etc/machine-id /etc/machine-id ro,relatime - ext4 /dev/sda1 rw";
const ROOT: &str = "25 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw";

/// Sources in `directory`: `/etc/machine-id` holds `machine` (missing when
/// `None`), the mounts are `mounts` and the host name is `hostname`.
fn fixture(directory: &Path, machine: Option<&str>, mounts: &[&str], hostname: &str) -> Sources {
    let sources = Sources {
        machine_id: directory.join("machine-id"),
        mountinfo: directory.join("mountinfo"),
        hostname: directory.join("hostname"),
        pid_namespace: directory.join("pid"),
    };
    let _ = fs::remove_file(&sources.machine_id);
    if let Some(machine) = machine {
        fs::write(&sources.machine_id, format!("{machine}\n")).unwrap();
    }
    fs::write(&sources.mountinfo, mounts.join("\n") + "\n").unwrap();
    fs::write(&sources.hostname, format!("{hostname}\n")).unwrap();
    fs::write(&sources.pid_namespace, "").unwrap();
    sources
}

fn machine(sources: &Sources, home: &Path) -> Option<String> {
    current_at(sources, home)
        .unwrap()
        .machine_id
        .map(String::from)
}

fn kept(home: &Path, hostname: &str) -> std::path::PathBuf {
    home.join("machines").join(format!("{hostname}.id"))
}

/// Only systemd's mount of `/run/machine-id` makes the machine ID transient:
/// not a bind mount of a real file, not a missing `/etc/machine-id`, and not
/// a mount that a later one at the same place hides.
#[test]
fn issue133_only_systemds_run_machine_id_mount_is_transient() {
    let other_root = "612 600 8:1 /machine-id /etc/machine-id ro - ext4 /dev/sda1 rw";
    let other_kind = "612 600 0:5 /etc/machine-id /etc/machine-id ro - tmpfs tmpfs rw";
    for (mounts, transient) in [
        (vec![ROOT, TRANSIENT], true),
        (vec![ROOT, BIND, TRANSIENT], true),
        (vec![ROOT], false),
        (vec![], false),
        (vec![ROOT, BIND], false),
        (vec![ROOT, TRANSIENT, BIND], false),
        (vec![other_root], false),
        (vec![other_kind], false),
        (vec!["not a mount", TRANSIENT], true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        fs::create_dir(&home).unwrap();
        let sources = fixture(directory.path(), Some(ETC), &mounts, "host");
        let found = machine(&sources, &home).unwrap();
        assert_eq!(found != ETC, transient, "{mounts:?}");
        assert_eq!(kept(&home, "host").exists(), transient, "{mounts:?}");
    }
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir(&home).unwrap();
    let missing = fixture(directory.path(), None, &[TRANSIENT], "host");
    assert_eq!(machine(&missing, &home), None);
    assert!(!home.join("machines").exists());
    let unreadable = Sources {
        mountinfo: directory.path().join("no-mountinfo"),
        ..fixture(directory.path(), Some(ETC), &[], "host")
    };
    assert_eq!(machine(&unreadable, &home).as_deref(), Some(ETC));
    let no_namespace = Sources {
        pid_namespace: directory.path().join("no-namespace"),
        ..unreadable
    };
    assert!(current_at(&no_namespace, &home).is_err());
}

/// The first reader generates the ID, owner-only, and every later one reads
/// it back; another host name sharing the home gets an ID of its own; an ID
/// already kept is used as it is.
#[test]
fn issue133_own_machine_id_is_generated_once_per_host_name() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir(&home).unwrap();
    let here = fixture(
        directory.path(),
        Some(ETC),
        &[TRANSIENT],
        "host-1.example_a",
    );
    let first = machine(&here, &home).unwrap();
    assert_eq!(first.len(), 32);
    assert!(first.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    assert_ne!(first, ETC);
    assert_eq!(machine(&here, &home).as_deref(), Some(first.as_str()));
    let path = kept(&home, "host-1.example_a");
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{first}\n"));
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&home.join("machines")), 0o700);

    let elsewhere = fixture(directory.path(), Some(ETC), &[TRANSIENT], "host-2");
    let second = machine(&elsewhere, &home).unwrap();
    assert_ne!(second, first);
    assert_eq!(machine(&here, &home).as_deref(), Some(first.as_str()));
    let mut names: Vec<_> = fs::read_dir(home.join("machines"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["host-1.example_a.id", "host-2.id"]);

    let known = "0123456789abcdef0123456789abcdef";
    fs::write(kept(&home, "host-3"), format!("{known}\n")).unwrap();
    let third = fixture(directory.path(), Some(ETC), &[TRANSIENT], "host-3");
    assert_eq!(machine(&third, &home).as_deref(), Some(known));
}

/// A kept ID that is not 32 lowercase hex digits and a newline, or that is a
/// symlink, is refused and left as it is; so are host names that are not one
/// safe file name, and a home where the ID cannot be written. Each leaves the
/// origin without a machine ID.
#[test]
fn issue133_an_unusable_own_machine_id_leaves_no_machine_id() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir_all(home.join("machines")).unwrap();
    let path = kept(&home, "host");
    let valid = "0123456789abcdef0123456789abcdef";
    for contents in [
        "0123456789ABCDEF0123456789ABCDEF\n".to_owned(),
        "0123456789abcdef0123456789abcde\n".to_owned(),
        "0123456789abcdef0123456789abcdef0\n".to_owned(),
        "0123456789abcdef0123456789abcdeg\n".to_owned(),
        valid.to_owned(),
        format!("{valid}\n\n"),
        String::new(),
    ] {
        fs::write(&path, &contents).unwrap();
        let sources = fixture(directory.path(), Some(ETC), &[TRANSIENT], "host");
        assert_eq!(machine(&sources, &home), None, "{contents:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
    }
    fs::remove_file(&path).unwrap();
    let target = directory.path().join("target");
    fs::write(&target, format!("{valid}\n")).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let sources = fixture(directory.path(), Some(ETC), &[TRANSIENT], "host");
    assert_eq!(machine(&sources, &home), None);
    assert!(fs::symlink_metadata(&path).unwrap().is_symlink());

    for hostname in ["", ".", "..", ".hidden", "a/b", "a b", "h\u{e9}"] {
        let fresh = directory.path().join(format!("fresh-{}", hostname.len()));
        let _ = fs::remove_dir_all(&fresh);
        fs::create_dir(&fresh).unwrap();
        let sources = fixture(directory.path(), Some(ETC), &[TRANSIENT], hostname);
        assert_eq!(machine(&sources, &fresh), None, "{hostname:?}");
        assert!(!fresh.join("machines").exists(), "{hostname:?}");
    }

    let blocked = directory.path().join("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("machines"), "").unwrap();
    let sources = fixture(directory.path(), Some(ETC), &[TRANSIENT], "host");
    assert_eq!(machine(&sources, &blocked), None);
    let missing_home = directory.path().join("no-home");
    assert_eq!(machine(&sources, &missing_home), None);
}
