//! `cargo xtask dist` on Windows: the release zip
//! (docs/for-humans/windows/distribution.md, "The archive").
//!
//! It builds the shipped packages for x86_64 MSVC, takes `conpty.dll` and
//! `OpenConsole.exe` from the pinned package the harness also uses, lays the
//! files out as the archive holds them and checks each one: every program and
//! library must be x86-64 code of the expected kind that does not import the
//! Visual C++ runtime, and climon2's own builds carry no test hooks. It then
//! zips the folder and checks the zip lists exactly the shipped files, so no
//! `.pdb` or other stray file can ship.
//!
//! This is release tooling: it runs in the Windows Dist job and is outside
//! CRAP and mutation, as the Unix dist is.
use crate::{
    pe::{self, Kind},
    quality, release,
};
use climon2_core::{VersionName, layout::ExecutableHome, native};
use climon2_test_term::ConPty;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const TARGET: &str = "x86_64-pc-windows-msvc";
const SUFFIX: &str = "windows-x86_64-msvc";
/// The packages a Windows release builds so far, each with its binary: the
/// launcher, then the version folder's programs. The installer joins them
/// with #329 and the monitor with #327.
const PACKAGES: &[(&str, &str)] = &[
    ("climon2-launcher", "c2"),
    ("climon2-cli", "climon2-cli"),
    ("climon2-daemon", "climon2-daemon"),
    ("climon2-client", "climon2-client"),
];
/// The notice for the ConPTY files, shipped beside them.
const NOTICE: &str = "pins/THIRD-PARTY-NOTICES.txt";

/// Where a shipped file comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    /// This release's build of a climon2 package.
    Built,
    /// The pinned ConPTY package.
    ConPty,
    /// A checked-in file, by its path in the workspace.
    Workspace(&'static str),
}

/// One file of the archive.
#[derive(Debug, Eq, PartialEq)]
struct Shipped {
    /// Its path inside the archive's folder, with `/` between components.
    path: String,
    source: Source,
    /// The kind it must have, for a program or a library.
    kind: Option<Kind>,
}

impl Shipped {
    fn new(path: String, source: Source, kind: Option<Kind>) -> Self {
        Self { path, source, kind }
    }

    /// Its file name, which is also its name at its source.
    fn name(&self) -> &str {
        self.path
            .rsplit('/')
            .next()
            .expect("a path has a last component")
    }
}

/// Every file of `version`'s archive.
fn shipped(version: &str) -> Vec<Shipped> {
    let program = |path: String, source| Shipped::new(path, source, Some(Kind::Executable));
    let library = |path: String, source| Shipped::new(path, source, Some(Kind::Library));
    let (launcher, programs) = PACKAGES.split_first().expect("the launcher");
    let mut files = vec![
        program(format!("{}.exe", launcher.1), Source::Built),
        Shipped::new("LICENSE".into(), Source::Workspace("LICENSE"), None),
        Shipped::new("README.md".into(), Source::Workspace("README.md"), None),
    ];
    for (_, binary) in programs {
        files.push(program(format!("{version}/{binary}.exe"), Source::Built));
    }
    files.extend([
        library(
            format!("{version}/{}", climon2_core::layout::client_library_name()),
            Source::Built,
        ),
        library(format!("{version}/conpty.dll"), Source::ConPty),
        program(format!("{version}/OpenConsole.exe"), Source::ConPty),
        Shipped::new(
            format!("{version}/THIRD-PARTY-NOTICES.txt"),
            Source::Workspace(NOTICE),
            None,
        ),
    ]);
    files
}

/// The folders the shipped files are copied from.
struct Sources {
    workspace: PathBuf,
    built: PathBuf,
    conpty: PathBuf,
}

impl Sources {
    fn of(&self, file: &Shipped) -> PathBuf {
        match file.source {
            Source::Built => self.built.join(file.name()),
            Source::ConPty => self.conpty.join(file.name()),
            Source::Workspace(path) => self.workspace.join(path),
        }
    }
}

pub fn run(root: &Path) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    release::validate_versions(root, version, PACKAGES)?;
    let conpty = ConPty::provide_files(&root.join("target/conpty"))?;
    let build = release::production_directory(root)?;
    for (package, _) in PACKAGES {
        release::build(root, &build, package, Some(TARGET))?;
    }
    let sources = Sources {
        workspace: root.to_owned(),
        built: build.join(TARGET).join("release"),
        conpty,
    };
    let scratch = tempfile::Builder::new()
        .prefix("dist-")
        .tempdir_in(root.join("target"))?;
    let name = format!("climon2-{version}-{SUFFIX}");
    let files = shipped(version);
    lay_out(&sources, &scratch.path().join(&name), &files)?;
    verify_library(&scratch.path().join(&name), version)?;
    let archive = zip(scratch.path(), &name, &files)?;
    let archive_name = format!("{name}.zip");
    fs::write(
        scratch.path().join("SHA256SUMS"),
        checksum_line(&archive_name, &fs::read(&archive)?),
    )?;
    let destination = root.join("target/dist");
    fs::create_dir_all(&destination)?;
    for file in [archive_name.as_str(), "SHA256SUMS"] {
        fs::rename(scratch.path().join(file), destination.join(file))?;
    }
    eprintln!("Release artifacts: {}", destination.display());
    Ok(())
}

/// Copies each of `files` from `sources` into `payload` and checks it there.
fn lay_out(sources: &Sources, payload: &Path, files: &[Shipped]) -> io::Result<()> {
    for file in files {
        let destination = payload.join(&file.path);
        fs::create_dir_all(destination.parent().expect("inside the payload"))?;
        fs::copy(sources.of(file), &destination)?;
        if file.source == Source::Built {
            release::unhooked(&destination)?;
        }
        if let Some(kind) = file.kind {
            check(&destination, kind)?;
        }
    }
    Ok(())
}

/// Fails unless the file at `path` may ship as a `kind`.
fn check(path: &Path, kind: Kind) -> io::Result<()> {
    pe::parse(&fs::read(path)?)
        .and_then(|image| shippable(&image, kind))
        .map_err(|reason| io::Error::other(format!("{}: {reason}", path.display())))
}

/// Why `image` may not ship as a `kind`, if it may not: it must be x86-64
/// code of that kind that links the C runtime statically, so it may import
/// neither of the Visual C++ runtime's DLLs. The CI runner has them
/// installed, so only this check would see a file that needs them.
fn shippable(image: &pe::Image, kind: Kind) -> Result<(), String> {
    if image.machine != pe::AMD64 {
        return Err(format!(
            "machine {:#06x} is not x86-64 ({:#06x})",
            image.machine,
            pe::AMD64
        ));
    }
    if image.kind != kind {
        return Err(format!("is {:?}, not {kind:?}", image.kind));
    }
    if let Some(runtime) = image.imports.iter().find(|name| is_c_runtime(name)) {
        return Err(format!(
            "imports {runtime}; every shipped file must link the C runtime statically"
        ));
    }
    Ok(())
}

/// `vcruntime140*.dll` and `msvcp140*.dll`, in any letter case.
fn is_c_runtime(dll: &str) -> bool {
    let dll = dll.to_ascii_lowercase();
    dll.ends_with(".dll") && (dll.starts_with("vcruntime140") || dll.starts_with("msvcp140"))
}

/// The client library in `payload` exports only the frozen bootstrap. It is
/// not loaded here: Windows would keep a loaded library's file locked, and the
/// Test job's library tests load the same build.
fn verify_library(payload: &Path, version: &str) -> io::Result<()> {
    let home = ExecutableHome::new(&payload.canonicalize()?).map_err(io::Error::other)?;
    let version: VersionName = version.parse().map_err(io::Error::other)?;
    let library = home.client_library(&version);
    let image = pe::parse(&fs::read(&library)?).map_err(io::Error::other)?;
    let bootstrap = native::BOOTSTRAP.to_string_lossy();
    if image.exports != [&*bootstrap] {
        return Err(io::Error::other(format!(
            "{} must export only {bootstrap}: {:?}",
            library.display(),
            image.exports
        )));
    }
    Ok(())
}

/// Zips the folder `name` in `scratch` into `<name>.zip` beside it, and fails
/// unless the zip holds exactly `files`.
fn zip(scratch: &Path, name: &str, files: &[Shipped]) -> io::Result<PathBuf> {
    let archive = scratch.join(format!("{name}.zip"));
    quality::run(
        Command::new(system_tool("tar.exe"))
            .args(["--format", "zip", "-c", "-f"])
            .arg(&archive)
            .arg("-C")
            .arg(scratch)
            .arg(name),
    )?;
    let listing = quality::output(
        Command::new(system_tool("tar.exe"))
            .arg("-tf")
            .arg(&archive),
    )?;
    let listed: BTreeSet<_> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|entry| !entry.ends_with('/'))
        .map(str::to_owned)
        .collect();
    let expected: BTreeSet<_> = files
        .iter()
        .map(|file| format!("{name}/{}", file.path))
        .collect();
    if listed != expected {
        return Err(io::Error::other(format!(
            "{} holds {:?} it should not and lacks {:?}",
            archive.display(),
            listed.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&listed).collect::<Vec<_>>()
        )));
    }
    Ok(archive)
}

/// `name`'s line in `SHA256SUMS`, as `sha256sum` prints it.
fn checksum_line(name: &str, contents: &[u8]) -> String {
    format!("{:x}  {name}\n", Sha256::digest(contents))
}

/// A tool Windows ships in System32, by absolute path.
fn system_tool(name: &str) -> PathBuf {
    let root = env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/for-humans/windows/distribution.md, "The archive", with `install.exe` and
    /// the monitor still to come (#329, #327).
    #[test]
    fn the_archive_holds_the_agreed_files_and_no_debug_symbols() {
        let files = shipped("1.2.3");
        let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "c2.exe",
                "LICENSE",
                "README.md",
                "1.2.3/climon2-cli.exe",
                "1.2.3/climon2-daemon.exe",
                "1.2.3/climon2-client.exe",
                "1.2.3/climon2_client.dll",
                "1.2.3/conpty.dll",
                "1.2.3/OpenConsole.exe",
                "1.2.3/THIRD-PARTY-NOTICES.txt",
            ]
        );
        let kinds: Vec<_> = files.iter().map(|file| file.kind).collect();
        let (program, library) = (Some(Kind::Executable), Some(Kind::Library));
        assert_eq!(
            kinds,
            [
                program, None, None, program, program, program, library, library, program, None
            ]
        );
    }

    /// The archive folder for `files`, with each file's path as its contents.
    fn payload(scratch: &Path, files: &[Shipped]) -> PathBuf {
        let payload = scratch.join("climon2-1.2.3-windows-x86_64-msvc");
        for file in files {
            let path = payload.join(&file.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, &file.path).unwrap();
        }
        payload
    }

    #[test]
    fn a_zip_with_a_debug_symbol_file_or_without_a_shipped_file_is_refused() {
        let scratch = tempfile::tempdir().unwrap();
        let name = "climon2-1.2.3-windows-x86_64-msvc";
        let files = shipped("1.2.3");
        let payload = payload(scratch.path(), &files);
        let archive = zip(scratch.path(), name, &files).unwrap();
        assert_eq!(archive, scratch.path().join(format!("{name}.zip")));

        fs::write(payload.join("1.2.3/climon2_client.pdb"), b"symbols").unwrap();
        let error = zip(scratch.path(), name, &files).unwrap_err().to_string();
        assert!(
            error.ends_with(&format!(
                "holds [\"{name}/1.2.3/climon2_client.pdb\"] it should not and lacks []"
            )),
            "{error}"
        );

        fs::remove_file(payload.join("1.2.3/climon2_client.pdb")).unwrap();
        fs::remove_file(payload.join("1.2.3/OpenConsole.exe")).unwrap();
        let error = zip(scratch.path(), name, &files).unwrap_err().to_string();
        assert!(
            error.ends_with(&format!(
                "holds [] it should not and lacks [\"{name}/1.2.3/OpenConsole.exe\"]"
            )),
            "{error}"
        );
    }

    /// This test executable: an x86-64 program built as every Windows build
    /// is, with the C runtime linked statically.
    fn this_program() -> Vec<u8> {
        fs::read(env::current_exe().unwrap()).unwrap()
    }

    fn refusal(bytes: &[u8], kind: Kind) -> String {
        shippable(&pe::parse(bytes).unwrap(), kind).unwrap_err()
    }

    #[test]
    fn a_program_of_another_machine_or_kind_is_refused() {
        let program = this_program();
        shippable(&pe::parse(&program).unwrap(), Kind::Executable).unwrap();
        assert_eq!(
            refusal(&program, Kind::Library),
            "is Executable, not Library"
        );
        let coff = u32::from_le_bytes(program[0x3c..0x40].try_into().unwrap()) as usize + 4;
        let mut x86 = program;
        x86[coff..coff + 2].copy_from_slice(&0x014cu16.to_le_bytes());
        assert_eq!(
            refusal(&x86, Kind::Executable),
            "machine 0x014c is not x86-64 (0x8664)"
        );
    }

    /// `program` with its import of `from` renamed to `to`.
    fn renamed_import(program: &[u8], from: &str, to: &str) -> Vec<u8> {
        assert!(to.len() <= from.len());
        let mut needle = from.as_bytes().to_vec();
        needle.push(0);
        let at = program
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap_or_else(|| panic!("{from} is not in the program"));
        let mut renamed = program.to_vec();
        renamed[at..at + from.len()].fill(0);
        renamed[at..at + to.len()].copy_from_slice(to.as_bytes());
        renamed
    }

    #[test]
    fn a_program_importing_the_visual_cpp_runtime_is_refused() {
        let program = this_program();
        let imports = pe::parse(&program).unwrap().imports;
        assert!(
            !imports.iter().any(|name| is_c_runtime(name)),
            "{imports:?}"
        );
        let long = imports
            .iter()
            .find(|name| name.len() >= "api-ms-win-crt-heap-l1-1-0.dll".len())
            .expect("an import with a long name");
        for runtime in [
            "vcruntime140.dll",
            "VCRUNTIME140_1.dll",
            "msvcp140.dll",
            "MSVCP140_2.dll",
        ] {
            assert_eq!(
                refusal(&renamed_import(&program, long, runtime), Kind::Executable),
                format!("imports {runtime}; every shipped file must link the C runtime statically")
            );
        }
        let universal = renamed_import(&program, long, "api-ms-win-crt-heap-l1-1-0.dll");
        shippable(&pe::parse(&universal).unwrap(), Kind::Executable).unwrap();
    }

    #[test]
    fn the_pinned_conpty_files_may_ship() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/conpty");
        let folder = ConPty::provide_files(&root).unwrap();
        check(&folder.join("conpty.dll"), Kind::Library).unwrap();
        check(&folder.join("OpenConsole.exe"), Kind::Executable).unwrap();
        let error = check(&folder.join("conpty.dll"), Kind::Executable)
            .unwrap_err()
            .to_string();
        assert!(
            error.ends_with("conpty.dll: is Library, not Executable"),
            "{error}"
        );
    }

    #[test]
    fn the_checksum_line_is_in_sha256sum_format() {
        assert_eq!(
            checksum_line("climon2-1.2.3-windows-x86_64-msvc.zip", b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  \
             climon2-1.2.3-windows-x86_64-msvc.zip\n"
        );
    }
}
