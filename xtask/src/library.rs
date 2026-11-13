//! Release checks specific to the client library: the exported symbol set,
//! on Linux the dynamic section, and that the shared loader accepts it as
//! exactly its version's library.

use climon2_core::{VersionName, layout::client_library_name, native};
use climon2_platform::library::Loader;
use std::{io, path::Path};

/// `version`'s client library in the executable home `home` exports only the
/// frozen bootstrap, on Linux depends on nothing a loader could share across
/// releases, and passes the checks of the shared loader the monitor uses: it
/// describes `version` with a complete, valid revision-1 description.
pub(crate) fn verify(home: &Path, version: &str) -> io::Result<()> {
    let library = home.join(version).join(client_library_name());
    verify_exports(&library)?;
    platform::verify_dynamic_section(&library)?;
    let version: VersionName = version.parse().map_err(io::Error::other)?;
    Loader::new(home).load(&version).map_err(io::Error::other)?;
    Ok(())
}

/// Every architecture slice exports exactly the frozen bootstrap symbol.
fn verify_exports(path: &Path) -> io::Result<()> {
    let bootstrap = native::BOOTSTRAP.to_string_lossy();
    let slices = platform::exported_symbols(path)?;
    if slices.is_empty() || slices.iter().any(|symbols| *symbols != [&*bootstrap]) {
        return Err(io::Error::other(format!(
            "{} must export only {bootstrap} from every slice: {slices:?}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod platform {
    use crate::quality::{TOOL_TIMEOUT, output_within};
    use std::{io, path::Path, process::Command};

    /// The C names of the defined external symbols of every architecture
    /// slice; Mach-O spells each C name with a leading underscore.
    pub(super) fn exported_symbols(path: &Path) -> io::Result<Vec<Vec<String>>> {
        let symbols = output_within(
            Command::new("nm").args(["-gUj", "-arch", "all"]).arg(path),
            TOOL_TIMEOUT,
        )?;
        Ok(slices(&String::from_utf8_lossy(&symbols.stdout)))
    }

    /// Symbol names per architecture; a thin file has one unlabelled slice.
    fn slices(listing: &str) -> Vec<Vec<String>> {
        let mut slices: Vec<Vec<String>> = Vec::new();
        for line in listing.lines() {
            if line.ends_with("):") && line.contains(" (for architecture ") {
                slices.push(Vec::new());
            } else if !line.is_empty() {
                if slices.is_empty() {
                    slices.push(Vec::new());
                }
                let name = line.strip_prefix('_').unwrap_or(line);
                slices.last_mut().unwrap().push(name.to_owned());
            }
        }
        slices
    }

    /// The Linux loader-identity rules have no Mach-O counterpart to check.
    pub(super) fn verify_dynamic_section(_: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use crate::quality::{TOOL_TIMEOUT, output_within};
    use std::{io, path::Path, process::Command};

    /// The glibc libraries that Rust's standard library can need on x86-64
    /// GNU/Linux, including the dynamic loader.
    const GLIBC: &[&str] = &[
        "ld-linux-x86-64.so.2",
        "libc.so.6",
        "libdl.so.2",
        "libm.so.6",
        "libpthread.so.0",
        "librt.so.1",
        "libutil.so.1",
    ];

    /// The defined dynamic symbols of the one ELF object, as one slice.
    pub(super) fn exported_symbols(path: &Path) -> io::Result<Vec<Vec<String>>> {
        let symbols = output_within(
            Command::new("nm").args(["-D", "--defined-only"]).arg(path),
            TOOL_TIMEOUT,
        )?;
        let names = String::from_utf8_lossy(&symbols.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().last())
            .map(str::to_owned)
            .collect();
        Ok(vec![names])
    }

    /// No soname, RPATH or RUNPATH, and no shared library beyond glibc's: the
    /// loader can then never bind one release's library or dependency for
    /// another's (library embedding).
    pub(super) fn verify_dynamic_section(path: &Path) -> io::Result<()> {
        let dynamic = output_within(
            Command::new("readelf")
                .args(["--dynamic", "--wide"])
                .arg(path),
            TOOL_TIMEOUT,
        )?;
        for line in String::from_utf8_lossy(&dynamic.stdout).lines() {
            let Some((tag, value)) = entry(line) else {
                continue;
            };
            let problem = match tag {
                "SONAME" | "RPATH" | "RUNPATH" => format!("has a {tag}: {value}"),
                "NEEDED" if !GLIBC.contains(&value) => {
                    format!("needs {value}, which is not part of glibc")
                }
                _ => continue,
            };
            return Err(io::Error::other(format!("{} {problem}", path.display())));
        }
        Ok(())
    }

    /// A `readelf --dynamic` line with a bracketed value, such as
    /// `0x...1 (NEEDED)  Shared library: [libc.so.6]`, as its tag and value.
    fn entry(line: &str) -> Option<(&str, &str)> {
        let tag = line.split_once('(')?.1.split_once(')')?.0;
        let value = line.rsplit_once('[')?.1.strip_suffix(']')?;
        Some((tag, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::{TOOL_TIMEOUT, output_within};
    use climon2_core::layout::client_library_name;
    use std::{fs, path::PathBuf, process::Command};

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    fn scratch() -> tempfile::TempDir {
        let target = crate::workspace_root().join("target");
        fs::create_dir_all(&target).unwrap();
        tempfile::Builder::new()
            .prefix("library-check-")
            .tempdir_in(target)
            .unwrap()
    }

    /// The library of this version in the executable home `home`.
    fn library(home: &Path) -> PathBuf {
        home.join(VERSION).join(client_library_name())
    }

    /// Compiles the test-only C stand-in for the library into this version's
    /// folder of the executable home `<directory>/<name>`, which it returns:
    /// a universal, ad-hoc signed dylib unless `archs` names other slices.
    /// `options` add defects.
    #[cfg(target_os = "macos")]
    fn stand_in(
        directory: &Path,
        name: &str,
        release: &str,
        archs: &[&str],
        options: &[&str],
    ) -> PathBuf {
        let root = crate::workspace_root();
        let home = directory.join(name);
        let output = library(&home);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        let mut clang = Command::new("clang");
        clang
            .args(["-dynamiclib", "-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(root.join("docs/protocol/monitor-client"))
            .arg(format!("-DRELEASE=\"{release}\""))
            .args(options);
        for arch in archs {
            clang.args(["-arch", arch]);
        }
        output_within(
            clang
                .arg("-o")
                .arg(&output)
                .arg(root.join("crates/climon2-fixtures/stand-in/client-library.c")),
            TOOL_TIMEOUT,
        )
        .unwrap();
        output_within(
            Command::new("codesign")
                .args(["--force", "-s", "-"])
                .arg(&output),
            TOOL_TIMEOUT,
        )
        .unwrap();
        home
    }

    /// Compiles the test-only C stand-in for the library into this version's
    /// folder of the executable home `<directory>/<name>`, which it returns:
    /// an x86-64 shared object that, like rustc's cdylib, exports only
    /// `climon2_client_*` symbols. `options` add defects.
    #[cfg(target_os = "linux")]
    fn stand_in(directory: &Path, name: &str, release: &str, options: &[&str]) -> PathBuf {
        let root = crate::workspace_root();
        let home = directory.join(name);
        let output = library(&home);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        let exports = directory.join(format!("{name}.exports"));
        fs::write(&exports, "{ global: climon2_client_*; local: *; };\n").unwrap();
        output_within(
            Command::new("cc")
                .args([
                    "-shared", "-fPIC", "-std=c11", "-Wall", "-Wextra", "-Werror",
                ])
                .arg("-I")
                .arg(root.join("docs/protocol/monitor-client"))
                .arg(format!("-DRELEASE=\"{release}\""))
                .arg(format!("-Wl,--version-script={}", exports.display()))
                .args(options)
                .arg("-o")
                .arg(&output)
                .arg(root.join("crates/climon2-fixtures/stand-in/client-library.c")),
            TOOL_TIMEOUT,
        )
        .unwrap();
        home
    }

    /// An executable home holding the exact library of this version, with no
    /// defect.
    fn exact(directory: &Path, name: &str, options: &[&str]) -> PathBuf {
        #[cfg(target_os = "macos")]
        return stand_in(directory, name, VERSION, &["arm64", "x86_64"], options);
        #[cfg(target_os = "linux")]
        return stand_in(directory, name, VERSION, options);
    }

    /// The architecture check `dist` applies to every packaged artifact.
    fn verify_architecture(path: &Path) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        return crate::dist::verify_macos_binary(path);
        #[cfg(target_os = "linux")]
        return crate::dist::verify_linux_binary(path);
    }

    /// #75: the exact, unhooked library of this version, exporting only the
    /// bootstrap, passes every release check, so each rejection below is
    /// caused by its one defect.
    #[test]
    fn release_checks_accept_the_exact_library() {
        let scratch = scratch();
        let home = exact(scratch.path(), "exact", &[]);
        verify_architecture(&library(&home)).unwrap();
        crate::dist::unhooked(&library(&home)).unwrap();
        verify(&home, VERSION).unwrap();
    }

    /// #75 (evidence policy: test hooks never ship): release checks reject a
    /// library carrying the test-hook marker.
    #[test]
    fn release_checks_reject_test_hook_markers() {
        let scratch = scratch();
        let home = exact(scratch.path(), "hooked", &["-DHOOK_MARKER"]);
        let error = crate::dist::unhooked(&library(&home)).unwrap_err();
        assert!(error.to_string().contains("contains test hooks"), "{error}");
    }

    /// DOC-01 exported interface: release checks reject a library exporting
    /// more than the frozen bootstrap.
    #[test]
    fn release_checks_reject_extra_exports() {
        let scratch = scratch();
        let home = exact(scratch.path(), "extra", &["-DEXTRA_EXPORT"]);
        let error = verify(&home, VERSION).unwrap_err();
        assert!(
            error.to_string().contains("climon2_client_extra_export"),
            "{error}"
        );
    }

    /// #75 packaging (release checks: exact version): release checks reject a
    /// library whose description, read through the frozen bootstrap, reports
    /// another release.
    #[test]
    fn release_checks_reject_a_wrong_version() {
        let scratch = scratch();
        #[cfg(target_os = "macos")]
        let home = stand_in(scratch.path(), "other", "9.9.9", &["arm64", "x86_64"], &[]);
        #[cfg(target_os = "linux")]
        let home = stand_in(scratch.path(), "other", "9.9.9", &[]);
        let error = verify(&home, VERSION).unwrap_err();
        assert!(error.to_string().contains("9.9.9"), "{error}");
    }

    /// #84: release checks load the library through the shared loader, so a
    /// library whose description the monitor would refuse, here for an
    /// incomplete function table, fails them too.
    #[test]
    fn release_checks_reject_a_description_the_loader_refuses() {
        let scratch = scratch();
        let home = exact(scratch.path(), "incomplete", &["-DINCOMPLETE_TABLE"]);
        let error = verify(&home, VERSION).unwrap_err();
        assert!(
            error.to_string().contains("function table is incomplete"),
            "{error}"
        );
    }

    /// #75: release checks reject a library built for one architecture only.
    #[cfg(target_os = "macos")]
    #[test]
    fn release_checks_reject_a_wrong_architecture() {
        let scratch = scratch();
        let home = stand_in(scratch.path(), "thin", VERSION, &["x86_64"], &[]);
        let error = crate::dist::verify_macos_binary(&library(&home)).unwrap_err();
        assert!(
            error.to_string().contains("is not exactly arm64 + x86_64"),
            "{error}"
        );
    }

    /// #75 (Linux amendment): release checks reject a library whose ELF header
    /// names another architecture (AArch64).
    #[cfg(target_os = "linux")]
    #[test]
    fn release_checks_reject_a_wrong_architecture() {
        const AARCH64: u16 = 183;
        let scratch = scratch();
        let library = library(&exact(scratch.path(), "aarch64", &[]));
        let mut bytes = fs::read(&library).unwrap();
        bytes[18..20].copy_from_slice(&AARCH64.to_le_bytes());
        fs::write(&library, bytes).unwrap();
        let error = crate::dist::verify_linux_binary(&library).unwrap_err();
        assert!(
            error.to_string().contains("is not an x86_64 Linux ELF"),
            "{error}"
        );
    }

    /// #75 packaging (Linux amendment; library embedding's release-build
    /// rules): release checks reject a library with a soname, an RPATH or a
    /// RUNPATH, through which the loader could bind another release.
    #[cfg(target_os = "linux")]
    #[test]
    fn release_checks_reject_a_soname_rpath_or_runpath() {
        let scratch = scratch();
        for (name, options, tag) in [
            (
                "soname",
                &["-Wl,-soname,libclimon2_client.so"][..],
                "SONAME",
            ),
            (
                "rpath",
                &["-Wl,-rpath,/opt/climon2", "-Wl,--disable-new-dtags"][..],
                "RPATH",
            ),
            (
                "runpath",
                &["-Wl,-rpath,/opt/climon2", "-Wl,--enable-new-dtags"][..],
                "RUNPATH",
            ),
        ] {
            let home = exact(scratch.path(), name, options);
            let error = verify(&home, VERSION).unwrap_err().to_string();
            assert!(error.contains(&format!("has a {tag}")), "{name}: {error}");
        }
    }

    /// #75 packaging (Linux amendment; library embedding's release-build
    /// rules): release checks reject a library that needs a shared library
    /// beyond glibc's, such as the GCC runtime.
    #[cfg(target_os = "linux")]
    #[test]
    fn release_checks_reject_a_dependency_beyond_glibc() {
        let scratch = scratch();
        let home = exact(
            scratch.path(),
            "gcc",
            &["-Wl,--no-as-needed", "-lgcc_s", "-Wl,--as-needed"],
        );
        let error = verify(&home, VERSION).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs libgcc_s.so.1, which is not part of glibc"),
            "{error}"
        );
    }
}
