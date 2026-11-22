//! #339: private files and folders on Windows (ADR 0003, "Private files and
//! folders"). A new file is never created through a link or over an existing
//! name, and a managed folder that is a link, or whose owner or access is not
//! exactly the user's, is refused and left as it was.
#![cfg(windows)]
use climon2_platform::{logging, private};
use std::{
    ffi::OsStr,
    fs, io,
    os::windows::fs::{symlink_dir, symlink_file},
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
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

    assert_eq!(descriptor(&path), private_descriptor(FILE));
}

#[test]
fn a_prepared_folder_carries_exactly_the_private_descriptor_and_preparing_again_keeps_it() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("logs");

    prepare(&folder);
    prepare(&folder);

    assert_eq!(descriptor(&folder), private_descriptor(FOLDER));
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
    prepare(&folder);
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
    prepare(&target);
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

#[test]
fn a_general_log_is_a_private_file_in_private_folders() {
    let home = tempfile::tempdir().unwrap();

    let log = logging::LogFile::general(home.path()).unwrap();

    let folder = private_descriptor(FOLDER);
    assert_eq!(descriptor(&home.path().join("logs")), folder);
    assert_eq!(
        descriptor(&home.path().join("logs").join("general")),
        folder
    );
    assert_eq!(descriptor(log.path()), private_descriptor(FILE));
}

#[test]
fn a_junction_or_directory_link_standing_in_for_logs_is_refused_and_pruning_leaves_its_target_alone()
 {
    let elsewhere = tempfile::tempdir().unwrap();
    let general = elsewhere.path().join("general");
    fs::create_dir(&general).unwrap();
    let old = general.join("20000101T000000Z-1.log");
    fs::write(&old, "kept").unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    for link in [junction as fn(&Path, &Path), directory_link] {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        link(elsewhere.path(), &logs);

        let error = logging::LogFile::general(home.path()).err().unwrap();
        logging::prune(home.path(), SystemTime::now());

        assert_unsafe(&error, &logs, "not a plain directory");
        assert_eq!(fs::read_to_string(&old).unwrap(), "kept");
        assert_eq!(fs::read_dir(&general).unwrap().count(), 1);
    }
}

/// Prepares `folder`, which must succeed; a refusal shows what was found.
fn prepare(folder: &Path) {
    if let Err(error) = private::private_directory(folder) {
        panic!(
            "{error}; found {}, expected {}",
            descriptor(folder),
            private_descriptor(FOLDER)
        );
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

/// A file's ACE has no inheritance flags.
const FILE: &str = "";
/// A folder's ACE is inherited by objects and containers created in it.
const FOLDER: &str = "OICI";

/// The private descriptor, owned by the current user with one ACE granting
/// that user full control, as .NET writes it in SDDL: the form `Get-Acl`
/// uses, which spells a well-known SID such as the built-in Administrator's
/// by its alias.
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

fn powershell(script: &str, environment: &[(&str, &OsStr)]) -> String {
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        // A PowerShell 7 parent's module path would load its own modules here.
        .env_remove("PSModulePath")
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
