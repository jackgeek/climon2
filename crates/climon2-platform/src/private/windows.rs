//! Private files and folders on Windows (#339; ADR 0003, "Private files and
//! folders"). Windows has no file modes and no `O_NOFOLLOW`, so each file and
//! folder gets the private descriptor when it is created: owned by the
//! process token's user SID, with a protected DACL holding one ACE, that SID
//! with full control (inherited by a folder's children). The owner is set
//! explicitly because an elevated process's objects are otherwise owned by
//! `BUILTIN\Administrators`.
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_SUCCESS, FALSE, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, GetTokenInformation, OWNER_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
use windows_sys::core::BOOL;

/// A new file only this user can open, created with the private descriptor
/// and never through a link or over any existing name: every file climon2
/// creates is created here.
pub fn create_new_file(path: &Path) -> io::Result<File> {
    let descriptor = Descriptor::private(FILE)?;
    let name = wide(path.as_os_str())?;
    // The name is NUL-terminated and the attributes outlive the call.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &descriptor.attributes(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // CreateFileW returned a new handle this call owns.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Windows has no umask: every new file already has the private descriptor.
pub use self::create_new_file as create_private_file;

/// Creates the folder with the private descriptor, its ACE inherited by
/// everything created in it, when missing. Then it must be a directory that
/// is not a reparse point of any tag, owned by the user's SID and holding
/// exactly the private DACL; otherwise it is refused with the "unsafe runtime
/// directory" error and left as it is.
pub fn private_directory(path: &Path) -> io::Result<()> {
    create_directory(path)?;
    check_directory(path)
}

fn create_directory(path: &Path) -> io::Result<()> {
    let descriptor = Descriptor::private(FOLDER)?;
    let name = wide(path.as_os_str())?;
    // The name is NUL-terminated and the attributes outlive the call.
    if unsafe { CreateDirectoryW(name.as_ptr(), &descriptor.attributes()) } == FALSE {
        return already_exists(io::Error::last_os_error());
    }
    Ok(())
}

/// A folder that already exists is checked like a new one.
fn already_exists(error: io::Error) -> io::Result<()> {
    if error.kind() != io::ErrorKind::AlreadyExists {
        return Err(error);
    }
    Ok(())
}

fn check_directory(path: &Path) -> io::Result<()> {
    let folder = open_itself(path)?;
    require_plain_directory(&folder, path)?;
    require_private(&folder, path)
}

/// The name itself, never a link's target, so both checks read one object.
fn open_itself(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

/// A directory that is not a reparse point of any tag.
fn require_plain_directory(folder: &File, path: &Path) -> io::Result<()> {
    let kind = folder.metadata()?.file_attributes()
        & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT);
    if kind != FILE_ATTRIBUTE_DIRECTORY {
        return Err(unsafe_directory(path, "not a plain directory"));
    }
    Ok(())
}

/// Owned by the user's SID and holding exactly the private DACL. Both sides
/// are written by the same conversion, which spells well-known SIDs by their
/// alias (`LA` for the built-in Administrator, which CI runs as).
fn require_private(folder: &File, path: &Path) -> io::Result<()> {
    if found_sddl(folder)? != private_folder_sddl()? {
        return Err(unsafe_directory(
            path,
            "not exactly the private owner and DACL",
        ));
    }
    Ok(())
}

fn unsafe_directory(path: &Path, problem: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("unsafe runtime directory {}: {problem}", path.display()),
    )
}

/// The owner and DACL `folder` has, in SDDL.
fn found_sddl(folder: &File) -> io::Result<String> {
    sddl(&security_of(folder)?)
}

/// The private descriptor a folder is created with, in SDDL.
fn private_folder_sddl() -> io::Result<String> {
    sddl(&Descriptor::private(FOLDER)?.0)
}

/// The owner and DACL `file` has.
fn security_of(file: &File) -> io::Result<Local> {
    let mut descriptor = ptr::null_mut();
    // The handle is open; the descriptor is allocated for this call to free.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_AND_DACL,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(Local(descriptor))
}

/// The owner and DACL of `descriptor` in SDDL.
fn sddl(descriptor: &Local) -> io::Result<String> {
    let mut text = ptr::null_mut();
    // The descriptor is valid; the text is allocated for this call to free.
    check(unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            SDDL_REVISION_1,
            OWNER_AND_DACL,
            &mut text,
            ptr::null_mut(),
        )
    })?;
    Ok(Local(text.cast()).text())
}

/// A new file's ACE has no inheritance flags.
const FILE: &str = "";
/// A folder's ACE is inherited by the files and folders created in it.
const FOLDER: &str = "OICI";
/// The parts of a descriptor the private descriptor sets and the check reads.
const OWNER_AND_DACL: u32 = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;

/// The private descriptor in SDDL; `inheritance` is the ACE's flags.
fn private_sddl(inheritance: &str) -> io::Result<String> {
    let user = user_sid()?;
    Ok(format!("O:{user}D:P(A;{inheritance};FA;;;{user})"))
}

/// The process token's user SID, in SDDL.
fn user_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // The current-process pseudo-handle needs no closing; the token does.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    // OpenProcessToken returned a new handle this call owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    // A TOKEN_USER and the largest SID fit, aligned for the structure.
    let mut buffer = [0u64; 16];
    // Written by the call; its starting value is never read.
    let mut length = 0u32;
    // The buffer is writable for its whole stated size.
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size_of_val(&buffer) as u32,
            &mut length,
        )
    })?;
    // GetTokenInformation filled the buffer with a TOKEN_USER.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text = ptr::null_mut();
    // The SID points into the buffer; the text is allocated for this call to free.
    check(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    Ok(Local(text.cast()).text())
}

/// The private descriptor, built from SDDL, for one creation.
struct Descriptor(Local);

impl Descriptor {
    fn private(inheritance: &str) -> io::Result<Self> {
        let sddl = wide(private_sddl(inheritance)?.as_ref())?;
        let mut descriptor = ptr::null_mut();
        // The SDDL is NUL-terminated; the descriptor is allocated for this call to free.
        check(unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        })?;
        Ok(Self(Local(descriptor)))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: FALSE,
        }
    }
}

/// Memory the system allocated for the caller to release with `LocalFree`.
struct Local(*mut c_void);

impl Local {
    /// The NUL-terminated UTF-16 text this memory holds.
    fn text(&self) -> String {
        let start = self.0.cast::<u16>();
        // The system wrote a NUL-terminated string here.
        let length = (0..).take_while(|&i| unsafe { *start.add(i) } != 0).count();
        // Those `length` units are initialised and stay allocated while `self` lives.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(start, length) })
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        // The pointer came from a system allocation not yet freed.
        unsafe { LocalFree(self.0) };
    }
}

/// `name` NUL-terminated in UTF-16; a NUL inside it is refused.
fn wide(name: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<u16> = name.encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name contains a NUL",
        ));
    }
    Ok(units.into_iter().chain([0]).collect())
}

/// A BOOL result: zero is failure, read from the thread's last error.
fn check(result: BOOL) -> io::Result<()> {
    if result == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
