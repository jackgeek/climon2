//! Release checks specific to the client library: the exported symbol set,
//! on Linux the dynamic section, and that the shared loader accepts it as
//! exactly its version's library.

use climon2_core::{VersionName, layout::ExecutableHome, native};
use climon2_platform::library::Loader;
use std::{io, path::Path};

/// `version`'s client library in the executable home `home` exports only the
/// frozen bootstrap, on Linux depends on nothing a loader could share across
/// releases, and passes the checks of the shared loader the monitor uses: it
/// describes `version` with a complete, valid revision-1 description.
pub(crate) fn verify(home: &ExecutableHome, version: &VersionName) -> io::Result<()> {
    let library = home.client_library(version);
    verify_exports(&library)?;
    platform::verify_dynamic_section(&library)?;
    Loader::new(home.clone())
        .load(version)
        .map_err(io::Error::other)?;
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
    use climon2_fixtures::stand_in;
    use std::{fs, path::PathBuf};

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// Both Mac architectures' slices, as a release library has; Linux
    /// releases are x86-64 only.
    #[cfg(target_os = "macos")]
    const UNIVERSAL: &[&str] = &["-arch", "arm64", "-arch", "x86_64"];
    #[cfg(target_os = "linux")]
    const UNIVERSAL: &[&str] = &[];

    fn version() -> VersionName {
        VERSION.parse().unwrap()
    }

    fn scratch() -> tempfile::TempDir {
        let target = crate::workspace_root().join("target");
        fs::create_dir_all(&target).unwrap();
        tempfile::Builder::new()
            .prefix("library-check-")
            .tempdir_in(target)
            .unwrap()
    }

    /// The executable home `<directory>/<name>`.
    fn home(directory: &Path, name: &str) -> ExecutableHome {
        ExecutableHome::new(&directory.join(name)).unwrap()
    }

    /// The library of this version in `home`.
    fn library(home: &ExecutableHome) -> PathBuf {
        home.client_library(&version())
    }

    /// The executable home `<directory>/<name>`, holding as this version's
    /// library the test-only stand-in describing `release`. `options` add
    /// defects or select architectures.
    fn holding(directory: &Path, name: &str, release: &str, options: &[&str]) -> ExecutableHome {
        let home = home(directory, name);
        let library = library(&home);
        fs::create_dir_all(library.parent().unwrap()).unwrap();
        stand_in::compile(&library, release, options).unwrap();
        home
    }

    /// An executable home holding the exact library of this version, with
    /// both Mac slices on macOS, plus `options`.
    fn exact(directory: &Path, name: &str, options: &[&str]) -> ExecutableHome {
        holding(directory, name, VERSION, &[UNIVERSAL, options].concat())
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
        verify(&home, &version()).unwrap();
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
        let error = verify(&home, &version()).unwrap_err();
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
        let home = holding(scratch.path(), "other", "9.9.9", UNIVERSAL);
        let error = verify(&home, &version()).unwrap_err();
        assert!(error.to_string().contains("9.9.9"), "{error}");
    }

    /// #84: release checks load the library through the shared loader, so a
    /// library whose description the monitor would refuse, here for an
    /// incomplete function table, fails them too.
    #[test]
    fn release_checks_reject_a_description_the_loader_refuses() {
        let scratch = scratch();
        let home = exact(scratch.path(), "incomplete", &["-DINCOMPLETE_TABLE"]);
        let error = verify(&home, &version()).unwrap_err();
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
        let home = holding(scratch.path(), "thin", VERSION, &["-arch", "x86_64"]);
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
        let scratch = scratch();
        let library = library(&exact(scratch.path(), "aarch64", &[]));
        stand_in::name_aarch64(&library).unwrap();
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
            let error = verify(&home, &version()).unwrap_err().to_string();
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
        let error = verify(&home, &version()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs libgcc_s.so.1, which is not part of glibc"),
            "{error}"
        );
    }
}
