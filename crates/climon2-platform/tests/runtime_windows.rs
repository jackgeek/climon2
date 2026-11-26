//! #316: the runtime directory on Windows (ADR 0003, "Runtime directory",
//! "Access control" and "Stale sockets"): `%LOCALAPPDATA%\c2\<hash8>`,
//! prepared by the private-folder check, with a 107-byte socket-path budget,
//! and a stale socket recognised by its AF_UNIX reparse tag.
//!
//! Tests that need their own `LOCALAPPDATA` run again in a child process with
//! it set, so no test changes this process's environment.
#![cfg(windows)]
use climon2_core::{failure::Failure, layout::home_hash8};
use climon2_platform::{connection, runtime};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    io::{Read, Write},
    os::windows::fs::symlink_file,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Set in the child that runs a test's body.
const CHILD: &str = "CLIMON2_TEST_RUNTIME_CHILD";

/// Runs `body` in a child process running only `test`, with `LOCALAPPDATA`
/// set to `root` of a new folder that is short enough for a socket path
/// whatever `TEMP` is (#422).
fn with_local_app_data(test: &str, root: impl FnOnce(&Path) -> OsString, body: impl FnOnce()) {
    if std::env::var_os(CHILD).is_some() {
        return body();
    }
    let folder = climon2_test_term::local_app_data();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture"])
        .env(CHILD, "1")
        .env("LOCALAPPDATA", root(folder.path()))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    // The child must have run the test, not filtered it out.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed"), "{stdout}");
}

fn local_app_data() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
}

fn canonical_home(parent: &Path) -> PathBuf {
    runtime::canonical_home(&parent.join("home")).unwrap()
}

fn hash_of(home: &Path) -> String {
    home_hash8(home.as_os_str().as_encoded_bytes())
}

const ID: &str = "12345678-1234-4234-8234-123456789abc";

#[test]
fn the_runtime_directory_is_the_home_hash_in_c2_under_local_app_data_and_private() {
    with_local_app_data(
        "the_runtime_directory_is_the_home_hash_in_c2_under_local_app_data_and_private",
        |folder| folder.into(),
        || {
            let parent = tempfile::tempdir().unwrap();
            let home = canonical_home(parent.path());

            let directory = runtime::RuntimeDirectory::prepare(&home).unwrap();
            runtime::RuntimeDirectory::prepare(&home).unwrap();

            let expected = local_app_data().join("c2").join(hash_of(&home));
            assert_eq!(directory.path(), expected);
            assert_eq!(descriptor(&expected), private_descriptor("OICI"));
            let socket = directory.socket_path(&ID.parse().unwrap()).unwrap();
            assert_eq!(socket, expected.join(format!("{ID}.sock")));
            // The socket file takes only the directory's inherited ACE.
            let _listener = runtime::bind(&socket).unwrap();
            assert_eq!(dacl(&socket), inherited_private_dacl());
        },
    );
}

/// A directory left with the inherited descriptor, as one made by hand is,
/// is refused with the unsafe-runtime-directory error and not repaired.
#[test]
fn an_unsafe_runtime_directory_is_refused_and_left_unrepaired() {
    with_local_app_data(
        "an_unsafe_runtime_directory_is_refused_and_left_unrepaired",
        |folder| folder.into(),
        || {
            let parent = tempfile::tempdir().unwrap();
            let home = canonical_home(parent.path());
            let directory = local_app_data().join("c2").join(hash_of(&home));
            fs::create_dir_all(&directory).unwrap();
            let inherited = descriptor(&directory);

            let error = runtime::RuntimeDirectory::prepare(&home).err().unwrap();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(
                error.to_string(),
                format!(
                    "unsafe runtime directory {}: not exactly the private owner and DACL",
                    directory.display()
                )
            );
            assert_eq!(descriptor(&directory), inherited);
        },
    );
}

#[test]
fn a_relative_local_app_data_is_refused() {
    with_local_app_data(
        "a_relative_local_app_data_is_refused",
        |_| "relative".into(),
        || {
            let parent = tempfile::tempdir().unwrap();
            let home = canonical_home(parent.path());

            let error = runtime::RuntimeDirectory::prepare(&home).err().unwrap();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(!Path::new("relative").exists());
        },
    );
}

/// A session's socket path is `LOCALAPPDATA` plus 54 bytes,
/// `\c2\<hash8>\<uuid>.sock`; past 107 bytes it is refused, which `exec`
/// reports as its own failure, 125.
#[test]
fn a_socket_path_over_107_bytes_is_refused_and_fails_with_125() {
    for length in [107_usize, 108] {
        let padded = |folder: &Path| {
            // `LOCALAPPDATA` is `<folder>\r...`, with at least one `r`: joining
            // an empty name would leave a trailing `\` the runtime root drops.
            // `<root>\c2\<hash8>\<uuid>.sock` then adds 1 + 2 + 1 + 8 + 1 + 41
            // bytes.
            let fixed = folder.as_os_str().len() + 1;
            let padding = (length - 54)
                .checked_sub(fixed)
                .filter(|padding| *padding >= 1)
                .unwrap_or_else(|| {
                    panic!(
                        "{} is over 51 bytes, too long for a {length}-byte socket path",
                        folder.display()
                    )
                });
            folder.join("r".repeat(padding)).into()
        };
        with_local_app_data(
            "a_socket_path_over_107_bytes_is_refused_and_fails_with_125",
            padded,
            socket_path_of_its_length,
        );
    }
}

/// In the child: the socket path is as long as `LOCALAPPDATA` makes it, 107
/// or 108 bytes.
fn socket_path_of_its_length() {
    let length = local_app_data().as_os_str().len() + 54;
    assert!([107, 108].contains(&length), "{length}");
    let admitted = length == 107;
    let parent = tempfile::tempdir().unwrap();
    let home = canonical_home(parent.path());
    let directory = runtime::RuntimeDirectory::prepare(&home).unwrap();
    assert_eq!(directory.path().as_os_str().len() + 42, length);

    let result = directory.socket_path(&ID.parse().unwrap());

    match result {
        Ok(path) => {
            assert!(admitted);
            assert_eq!(path.as_os_str().len(), length);
            drop(runtime::bind(&path).unwrap());
        }
        Err(error) => {
            assert!(!admitted);
            assert_eq!(Failure::from(error).code, 125);
        }
    }
}

/// `bind` holds the same budget for any path.
#[test]
fn bind_admits_a_107_byte_path_and_refuses_108_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().as_os_str().len() + 1;
    let path = |length: usize| directory.path().join("s".repeat(length - prefix));

    drop(runtime::bind(&path(107)).unwrap());
    let error = runtime::bind(&path(108)).err().unwrap();

    assert_eq!(error.to_string(), "socket path exceeds 107 bytes");
    assert!(!path(108).exists());
}

/// On the real profile folder, as every session will use it
/// (docs/for-humans/windows/quality.md, "Facts only CI can show").
#[test]
fn a_session_socket_binds_and_connects_in_the_real_runtime_directory() {
    let parent = tempfile::tempdir().unwrap();
    let home = canonical_home(parent.path());
    let directory = runtime::RuntimeDirectory::prepare(&home).unwrap();
    let path = directory.socket_path(&ID.parse().unwrap()).unwrap();

    let listener = runtime::bind(&path).unwrap();
    let mut client = connection::connect(&path, Duration::from_secs(1)).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    client.write_all(b"x").unwrap();
    let mut received = [0];
    peer.read_exact(&mut received).unwrap();

    assert_eq!(&received, b"x");
    assert!(directory.path().starts_with(local_app_data().join("c2")));
    drop((listener, client, peer));
    fs::remove_dir_all(directory.path()).unwrap();
}

#[test]
fn a_stale_socket_or_a_missing_path_may_be_removed_as_a_socket() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    drop(runtime::bind(&path).unwrap());
    assert!(
        fs::symlink_metadata(&path).unwrap().is_file(),
        "std calls it a file"
    );

    runtime::validate_stale_socket(&path).unwrap();
    runtime::validate_stale_socket(&directory.path().join("missing.sock")).unwrap();
    runtime::validate_stale_socket(&directory.path().join("gone").join("missing.sock")).unwrap();
}

/// Anything else at a socket path, a link to a socket included, is never
/// removed as one.
#[test]
fn anything_but_a_socket_file_is_refused_as_a_stale_socket() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    drop(runtime::bind(&socket).unwrap());
    let file = directory.path().join("file.sock");
    fs::write(&file, "").unwrap();
    let link = directory.path().join("link.sock");
    symlink_file(&socket, &link).unwrap();
    let junction = directory.path().join("junction.sock");
    let made = Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .args([&junction, &directory.path().to_path_buf()])
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");

    for path in [&file, &link, &junction, &directory.path().to_path_buf()] {
        let error = runtime::validate_stale_socket(path).unwrap_err();
        assert_eq!(
            error.to_string(),
            "stale socket path is not a socket",
            "{path:?}"
        );
    }
}

/// The private descriptor, as `Get-Acl` writes it in SDDL (see
/// private_windows.rs).
fn private_descriptor(ace_flags: &str) -> String {
    powershell(
        "$user = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value;          $sddl = \"O:${user}D:P(A;$env:CLIMON2_ACE_FLAGS;FA;;;$user)\";          [Security.AccessControl.RawSecurityDescriptor]::new($sddl).GetSddlForm('Owner, Access')",
        &[("CLIMON2_ACE_FLAGS", OsStr::new(ace_flags))],
    )
}

/// The owner and DACL of `path` in SDDL, as `Get-Acl` reads them.
fn descriptor(path: &Path) -> String {
    powershell(
        "(Get-Acl -LiteralPath $env:CLIMON2_ACL_PATH).GetSecurityDescriptorSddlForm('Owner, Access')",
        &[("CLIMON2_ACL_PATH", path.as_os_str())],
    )
}

/// The DACL a file created in a private folder inherits: the user's full
/// control, marked inherited.
fn inherited_private_dacl() -> String {
    powershell(
        "$user = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value;          [Security.AccessControl.RawSecurityDescriptor]::new(\"D:(A;ID;FA;;;$user)\").GetSddlForm('Access')",
        &[],
    )
}

/// The DACL of `path` in SDDL, as `Get-Acl` reads it.
fn dacl(path: &Path) -> String {
    powershell(
        "(Get-Acl -LiteralPath $env:CLIMON2_ACL_PATH).GetSecurityDescriptorSddlForm('Access')",
        &[("CLIMON2_ACL_PATH", path.as_os_str())],
    )
}

fn powershell(script: &str, environment: &[(&str, &OsStr)]) -> String {
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env_remove("PSModulePath")
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
