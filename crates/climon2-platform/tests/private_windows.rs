//! #339: private files and folders on Windows (ADR 0003, "Private files and
//! folders"). A new file is never created through a link or over an existing
//! name, and a managed folder that is a link, or whose owner or access is not
//! exactly the user's, is refused and left as it was.
#![cfg(windows)]
use climon2_platform::private;
use std::{
    ffi::OsStr,
    fs, io,
    os::windows::fs::{symlink_dir, symlink_file},
    path::Path,
    process::Command,
};

#[test]
fn a_dangling_file_link_at_the_final_name_is_refused_and_nothing_appears_at_its_target() {
    let folder = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("planted.log");
    let link = folder.path().join("daemon.log");
    symlink_file(&target, &link).unwrap();

    let error = private::create_new_file(&link).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(!target.exists(), "the link's target was created");
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
}

#[test]
fn an_existing_file_is_refused_and_kept() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("daemon.log");
    fs::write(&path, "earlier").unwrap();

    let error = private::create_new_file(&path).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "earlier");
}

#[test]
fn a_name_holding_a_nul_is_refused_and_nothing_is_created_at_its_prefix() {
    let folder = tempfile::tempdir().unwrap();
    let mut name = folder.path().join("daemon").into_os_string();
    name.push("\0.log");

    let error = private::create_new_file(Path::new(&name)).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
}

#[test]
fn a_created_file_carries_exactly_the_private_descriptor() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("daemon.log");

    drop(private::create_new_file(&path).unwrap());

    let user = user_sid();
    assert_eq!(descriptor(&path), format!("O:{user}D:P(A;;FA;;;{user})"));
}

#[test]
fn a_prepared_folder_carries_exactly_the_private_descriptor_and_preparing_again_keeps_it() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("logs");

    private::private_directory(&folder).unwrap();
    private::private_directory(&folder).unwrap();

    let user = user_sid();
    assert_eq!(
        descriptor(&folder),
        format!("O:{user}D:P(A;OICI;FA;;;{user})")
    );
}

#[test]
fn a_folder_whose_parent_is_missing_is_not_prepared() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("missing").join("logs");

    let error = private::private_directory(&folder).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!parent.path().join("missing").exists());
}

#[test]
fn a_folder_with_the_inherited_descriptor_is_refused_and_left_unrepaired() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("logs");
    fs::create_dir(&folder).unwrap();
    let inherited = descriptor(&folder);

    let error = private::private_directory(&folder).unwrap_err();

    assert_unsafe(&error, &folder, "not exactly the private owner and DACL");
    assert_eq!(descriptor(&folder), inherited);
}

#[test]
fn a_folder_with_an_extra_ace_is_refused_and_left_unrepaired() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("logs");
    private::private_directory(&folder).unwrap();
    let granted = Command::new("icacls.exe")
        .arg(&folder)
        .args(["/grant", "*S-1-1-0:(R)"])
        .output()
        .unwrap();
    assert!(granted.status.success(), "{granted:?}");
    let widened = descriptor(&folder);

    let error = private::private_directory(&folder).unwrap_err();

    assert_unsafe(&error, &folder, "not exactly the private owner and DACL");
    assert_eq!(descriptor(&folder), widened);
}

#[test]
fn a_file_standing_in_for_a_folder_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("logs");
    fs::write(&folder, "").unwrap();

    let error = private::private_directory(&folder).unwrap_err();

    assert_unsafe(&error, &folder, "not a plain directory");
}

#[test]
fn a_junction_or_directory_link_standing_in_for_a_folder_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("target");
    private::private_directory(&target).unwrap();
    for (name, link) in [
        ("junction", junction as fn(&Path, &Path)),
        ("link", directory_link),
    ] {
        let folder = parent.path().join(name);
        link(&target, &folder);

        let error = private::private_directory(&folder).unwrap_err();

        assert_unsafe(&error, &folder, "not a plain directory");
    }
}

fn assert_unsafe(error: &io::Error, folder: &Path, problem: &str) {
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        error.to_string(),
        format!("unsafe runtime directory {}: {problem}", folder.display())
    );
}

/// Makes `link` a junction to `target`; junctions need no privilege.
fn junction(target: &Path, link: &Path) {
    let made = Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .args([link, target])
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
}

/// Makes `link` a directory symbolic link to `target`.
fn directory_link(target: &Path, link: &Path) {
    symlink_dir(target, link).unwrap();
}

/// The current user's SID, as Windows PowerShell's .NET names it.
fn user_sid() -> String {
    powershell(
        "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
        &[],
    )
}

/// The owner and DACL of `path` in SDDL, as `Get-Acl` reads them.
fn descriptor(path: &Path) -> String {
    powershell(
        "(Get-Acl -LiteralPath $env:CLIMON2_ACL_PATH).GetSecurityDescriptorSddlForm('Owner, Access')",
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
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
