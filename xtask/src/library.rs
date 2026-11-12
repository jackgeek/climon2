//! Release checks specific to the client library: the exported symbol set and
//! the exact release its revision-1 description reports.

use climon2_core::native;
use std::{
    ffi::{CStr, CString, c_void},
    io,
    os::unix::ffi::OsStrExt,
    path::Path,
    process::Command,
};

/// The client library exports only the frozen bootstrap and describes `version`.
pub(crate) fn verify(path: &Path, version: &str) -> io::Result<()> {
    verify_exports(path)?;
    let release = release(path)?;
    if release != version.as_bytes() {
        return Err(io::Error::other(format!(
            "{} describes release {:?}, not {version}",
            path.display(),
            String::from_utf8_lossy(&release)
        )));
    }
    Ok(())
}

/// Every architecture slice exports exactly the frozen bootstrap symbol.
fn verify_exports(path: &Path) -> io::Result<()> {
    let symbols =
        crate::quality::output(Command::new("nm").args(["-gUj", "-arch", "all"]).arg(path))?;
    let bootstrap = format!("_{}", native::BOOTSTRAP.to_string_lossy());
    let listing = String::from_utf8_lossy(&symbols.stdout);
    let slices = slices(&listing);
    if slices.is_empty()
        || slices
            .iter()
            .any(|symbols| *symbols != [bootstrap.as_str()])
    {
        return Err(io::Error::other(format!(
            "{} must export only {bootstrap} from every slice: {slices:?}",
            path.display()
        )));
    }
    Ok(())
}

/// Symbol names per architecture; a thin file has one unlabelled slice.
fn slices(listing: &str) -> Vec<Vec<&str>> {
    let mut slices: Vec<Vec<&str>> = Vec::new();
    for line in listing.lines() {
        if line.ends_with("):") && line.contains(" (for architecture ") {
            slices.push(Vec::new());
        } else if !line.is_empty() {
            if slices.is_empty() {
                slices.push(Vec::new());
            }
            slices.last_mut().unwrap().push(line);
        }
    }
    slices
}

fn dlerror() -> io::Error {
    // SAFETY: dlerror returns null or a NUL-terminated message owned by dyld.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        return io::Error::other("unknown loader error");
    }
    // SAFETY: nonnull dlerror results are NUL-terminated strings.
    io::Error::other(
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Loads the library (never unloaded) and reads revision 1's release text.
fn release(path: &Path) -> io::Result<Vec<u8>> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: the path is NUL-terminated; the handle is deliberately kept.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Err(dlerror());
    }
    // SAFETY: live handle and NUL-terminated symbol name.
    let symbol = unsafe { libc::dlsym(handle, native::BOOTSTRAP.as_ptr()) };
    if symbol.is_null() {
        return Err(dlerror());
    }
    // SAFETY: the frozen bootstrap symbol has the `Bootstrap` signature.
    let bootstrap = unsafe { std::mem::transmute::<*mut c_void, native::Bootstrap>(symbol) };
    let mut description = std::ptr::null();
    // SAFETY: the output is writable storage for one pointer.
    let status = unsafe { bootstrap(native::REVISION, &mut description) };
    if status != native::SUCCESS || description.is_null() {
        return Err(io::Error::other(format!(
            "revision 1 bootstrap failed: {status}"
        )));
    }
    // SAFETY: success returns an immutable description that lives until exit.
    let text = unsafe { (*description).release };
    if text.data.is_null() || text.length > 256 {
        return Err(io::Error::other("malformed release text"));
    }
    // SAFETY: a nonnull release covers `length` readable bytes.
    Ok(unsafe { std::slice::from_raw_parts(text.data, text.length as usize) }.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, process::Command};

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const UNIVERSAL: &[&str] = &["arm64", "x86_64"];

    /// Compiles and ad-hoc signs the test-only C stand-in for the library.
    fn fixture(
        directory: &Path,
        name: &str,
        release: &str,
        archs: &[&str],
        defects: &[&str],
    ) -> PathBuf {
        let root = crate::workspace_root();
        let output = directory.join(name);
        let mut clang = Command::new("clang");
        clang
            .args([
                "-dynamiclib",
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-I",
            ])
            .arg(root.join("docs/protocol/monitor-client"))
            .arg(format!("-DRELEASE=\"{release}\""));
        for arch in archs {
            clang.args(["-arch", arch]);
        }
        for defect in defects {
            clang.arg(format!("-D{defect}"));
        }
        crate::quality::run(
            clang
                .arg("-o")
                .arg(&output)
                .arg(root.join("xtask/tests/fixtures/client-library.c")),
        )
        .unwrap();
        crate::quality::run(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(&output),
        )
        .unwrap();
        output
    }

    fn scratch() -> tempfile::TempDir {
        let target = crate::workspace_root().join("target");
        std::fs::create_dir_all(&target).unwrap();
        tempfile::Builder::new()
            .prefix("library-check-")
            .tempdir_in(target)
            .unwrap()
    }

    /// #75: a universal, unhooked library of this version that exports only
    /// the bootstrap passes every release check, so each rejection below is
    /// caused by its one defect.
    #[test]
    fn release_checks_accept_the_exact_universal_library() {
        let scratch = scratch();
        let library = fixture(scratch.path(), "library.dylib", VERSION, UNIVERSAL, &[]);
        crate::dist::verify_macos_binary(&library).unwrap();
        crate::dist::unhooked(&library).unwrap();
        verify(&library, VERSION).unwrap();
    }

    /// #75: release checks reject a library built for one architecture only.
    #[test]
    fn release_checks_reject_a_wrong_architecture() {
        let scratch = scratch();
        let library = fixture(scratch.path(), "thin.dylib", VERSION, &["x86_64"], &[]);
        let error = crate::dist::verify_macos_binary(&library).unwrap_err();
        assert!(
            error.to_string().contains("is not exactly arm64 + x86_64"),
            "{error}"
        );
    }

    /// #75: release checks reject a library carrying the test-hook marker.
    #[test]
    fn release_checks_reject_test_hook_markers() {
        let scratch = scratch();
        let library = fixture(
            scratch.path(),
            "hooked.dylib",
            VERSION,
            UNIVERSAL,
            &["HOOK_MARKER"],
        );
        let error = crate::dist::unhooked(&library).unwrap_err();
        assert!(error.to_string().contains("contains test hooks"), "{error}");
    }

    /// #75: release checks reject a library exporting more than the bootstrap.
    #[test]
    fn release_checks_reject_extra_exports() {
        let scratch = scratch();
        let library = fixture(
            scratch.path(),
            "extra.dylib",
            VERSION,
            UNIVERSAL,
            &["EXTRA_EXPORT"],
        );
        let error = verify(&library, VERSION).unwrap_err();
        assert!(
            error.to_string().contains("_climon2_client_extra_export"),
            "{error}"
        );
    }

    /// #75: release checks reject a library whose description reports another
    /// release, read through the frozen bootstrap.
    #[test]
    fn release_checks_reject_a_wrong_version() {
        let scratch = scratch();
        let library = fixture(scratch.path(), "other.dylib", "9.9.9", UNIVERSAL, &[]);
        let error = verify(&library, VERSION).unwrap_err();
        assert!(error.to_string().contains("9.9.9"), "{error}");
    }
}
