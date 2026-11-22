//! #316: the private descriptor denies a second, non-administrator account
//! the runtime directory, and also a session log, a metadata file and the
//! `logs` folder (docs/for-humans/windows/quality.md, "Facts only CI can show").
//!
//! CI's Windows jobs create that account and name it in
//! `CLIMON2_SECOND_ACCOUNT` and `CLIMON2_SECOND_ACCOUNT_PASSWORD`. Without
//! them the test is skipped, with the reason, everywhere but CI, where it
//! fails.
#![cfg(windows)]
use climon2_platform::{
    logging::{LogFile, Role},
    private, runtime,
};
use std::{
    ffi::OsStr,
    fs, io,
    io::Write,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    process::Command,
    ptr,
};
use windows_sys::Win32::{
    Foundation::FALSE,
    Security::{
        ImpersonateLoggedOnUser, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
        RevertToSelf,
    },
};

const ACCOUNT: &str = "CLIMON2_SECOND_ACCOUNT";
const PASSWORD: &str = "CLIMON2_SECOND_ACCOUNT_PASSWORD";
/// Set in the child that runs the test's body.
const CHILD: &str = "CLIMON2_TEST_CROSS_ACCOUNT_CHILD";
const ID: &str = "12345678-1234-4234-8234-123456789abc";
const ERROR_ACCESS_DENIED: i32 = 5;

/// The second account's name and password, or `None`, after saying why,
/// where it was not created.
fn second_account() -> Option<(String, String)> {
    match (std::env::var(ACCOUNT), std::env::var(PASSWORD)) {
        (Ok(account), Ok(password)) => Some((account, password)),
        _ if std::env::var_os("GITHUB_ACTIONS").is_some() => {
            panic!("CI must create the second account and set {ACCOUNT} and {PASSWORD}")
        }
        _ => {
            eprintln!(
                "skipped: no second account; {ACCOUNT} and {PASSWORD} name the \
                 non-administrator account CI's Windows jobs create"
            );
            None
        }
    }
}

/// Everything lives under a folder in the system drive's root, which
/// inherits that root's grant of change access to Authenticated Users, so
/// the second account is kept out by the private descriptor alone and not
/// by a profile folder above it. An ordinary file there shows the account
/// can read what that descriptor does not cover.
#[test]
fn a_second_account_is_denied_the_runtime_directory_logs_and_metadata() {
    let Some((account, password)) = second_account() else {
        return;
    };
    if std::env::var_os(CHILD).is_some() {
        return denied_everything_private(&account, &password);
    }
    let drive = PathBuf::from(format!("{}\\", std::env::var("SystemDrive").unwrap()));
    let parent = tempfile::Builder::new()
        .prefix("climon2-cross-account-")
        .tempdir_in(drive)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "a_second_account_is_denied_the_runtime_directory_logs_and_metadata",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("LOCALAPPDATA", parent.path().join("local"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed"), "{stdout}");
}

/// In the child, whose `LOCALAPPDATA` is `<parent>\local`.
fn denied_everything_private(account: &str, password: &str) {
    let parent = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let ordinary = parent.join("ordinary.txt");
    fs::write(&ordinary, "anyone may read this").unwrap();
    let home = runtime::canonical_home(&parent.join("home")).unwrap();
    let runtime = runtime::RuntimeDirectory::prepare(&home).unwrap();
    let socket = runtime.socket_path(&ID.parse().unwrap()).unwrap();
    let _listener = runtime::bind(&socket).unwrap();
    let log = LogFile::session(&home, &ID.parse().unwrap(), Role::Daemon, None, None).unwrap();
    let metadata = publish_metadata(&home);
    assert!(runtime.path().starts_with(&parent), "{:?}", runtime.path());

    let results = as_account(account, password, || {
        [
            ("ordinary file", read(&ordinary)),
            ("runtime directory", list(runtime.path())),
            (
                "file in the runtime directory",
                create(&runtime.path().join("planted")),
            ),
            ("session log", read(log.path())),
            ("metadata file", read(&metadata)),
            ("logs folder", list(&home.join("logs"))),
        ]
    });

    let [(_, ordinary), denied @ ..] = results;
    assert!(
        ordinary.is_ok(),
        "the account cannot read an ordinary file: {ordinary:?}"
    );
    for (what, result) in denied {
        let error = result
            .err()
            .unwrap_or_else(|| panic!("the account opened the {what}"));
        assert_eq!(
            error.raw_os_error(),
            Some(ERROR_ACCESS_DENIED),
            "{what}: {error}"
        );
    }
}

/// Metadata as the daemon publishes it: a private temporary file renamed
/// into the private `sessions` folder.
fn publish_metadata(home: &Path) -> PathBuf {
    let sessions = home.join("sessions");
    private::private_directory(&sessions).unwrap();
    let temporary = sessions.join(format!(".{ID}.tmp"));
    private::create_private_file(&temporary)
        .unwrap()
        .write_all(b"{}")
        .unwrap();
    let path = sessions.join(format!("{ID}.pty"));
    fs::rename(&temporary, &path).unwrap();
    path
}

fn read(path: &Path) -> io::Result<()> {
    fs::read(path).map(drop)
}

fn list(path: &Path) -> io::Result<()> {
    fs::read_dir(path).map(drop)
}

fn create(path: &Path) -> io::Result<()> {
    fs::File::create_new(path).map(drop)
}

/// Runs `work` on this thread as `account`, logged on as from the network,
/// which every account may be.
fn as_account<T>(account: &str, password: &str, work: impl FnOnce() -> T) -> T {
    let wide = |text: &str| -> Vec<u16> { OsStr::new(text).encode_wide().chain([0]).collect() };
    let (account, domain, password) = (wide(account), wide("."), wide(password));
    let mut token = ptr::null_mut();
    assert_ne!(
        unsafe {
            LogonUserW(
                account.as_ptr(),
                domain.as_ptr(),
                password.as_ptr(),
                LOGON32_LOGON_NETWORK,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        },
        FALSE,
        "logon: {}",
        io::Error::last_os_error()
    );
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    assert_ne!(
        unsafe { ImpersonateLoggedOnUser(token.as_raw_handle()) },
        FALSE
    );
    let result = work();
    assert_ne!(unsafe { RevertToSelf() }, FALSE);
    result
}
