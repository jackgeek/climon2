//! The Windows installer (docs/for-humans/windows/distribution.md, "The installer" and
//! "Upgrading"). `install.exe [--dir <directory>] [--no-modify-path]` places
//! the release beside it into an executable home, `%LOCALAPPDATA%\climon2`
//! unless `--dir` names another, then adds that home to the user's `PATH`
//! unless told not to.
//!
//! One installer at a time holds `install.lock`. The version folder is staged
//! and renamed into place: an identical one is kept, a different one no
//! program runs from is swapped for the new one, and a different one in use
//! is refused. The launcher is replaced unless it is identical or Windows
//! refuses. `current` is replaced last.
#[path = "user_path/windows.rs"]
mod user_path;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// The launcher's file name, in the archive and in the executable home.
pub const LAUNCHER: &str = "c2.exe";
/// The version this installer installs: its own.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The lock file in the executable home. It is never renamed or deleted.
const LOCK: &str = "install.lock";
/// The executable home's staging folder, reserved for the installer.
const STAGE: &str = ".climon2-install-stage";
const USAGE: &str = "usage: install [--dir <directory>] [--no-modify-path]";

/// Every file of a version folder, in installation order. The monitor joins
/// them with #327.
pub fn version_files() -> Vec<String> {
    [
        "climon2-cli.exe",
        "climon2-daemon.exe",
        "climon2-client.exe",
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([climon2_core::layout::client_library_name()])
    .chain(["conpty.dll", "OpenConsole.exe", "THIRD-PARTY-NOTICES.txt"].map(str::to_owned))
    .collect()
}

/// Run the installer command with arguments excluding argv[0].
pub fn run(args: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let options = Options::parse(args)?;
    let home = executable_home(options.dir)?;
    // Held until the end, so two installers never edit PATH at once either.
    let _lock = lock(&home)?;
    install(&home)?;
    finish(&home, options.modify_path)
}

struct Options {
    dir: Option<PathBuf>,
    modify_path: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut options = Self {
            dir: None,
            modify_path: true,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            options.take(arg, &mut args)?;
        }
        Ok(options)
    }

    /// Takes one option, and the value it needs, each option at most once.
    fn take(&mut self, arg: OsString, rest: &mut impl Iterator<Item = OsString>) -> io::Result<()> {
        match arg.to_str() {
            Some("--no-modify-path") if self.modify_path => self.modify_path = false,
            Some("--dir") if self.dir.is_none() => {
                let dir = rest.next().filter(|dir| !dir.is_empty());
                self.dir = Some(dir.ok_or_else(usage)?.into());
            }
            _ => return Err(usage()),
        }
        Ok(())
    }
}

fn usage() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, USAGE)
}

/// The absolute executable home, created when missing.
fn executable_home(dir: Option<PathBuf>) -> io::Result<PathBuf> {
    let home = dir.map_or_else(default_home, std::path::absolute)?;
    fs::create_dir_all(&home)?;
    Ok(home)
}

fn default_home() -> io::Result<PathBuf> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    if !base.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LOCALAPPDATA must name an absolute directory without --dir",
        ));
    }
    Ok(base.join("climon2"))
}

/// Holds `install.lock` until dropped, or fails at once if another installer does.
fn lock(home: &Path) -> io::Result<File> {
    let path = home.join(LOCK);
    // The lock file holds nothing, so creating it over itself loses nothing.
    let file = File::create(&path)?;
    file.try_lock().map_err(|error| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "cannot lock {}; is another installer running? ({error})",
                path.display()
            ),
        )
    })?;
    Ok(file)
}

/// The release beside this installer, staged and put in place.
fn install(home: &Path) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let release = executable.parent().expect("a program is in a folder");
    let stage = Stage::new(home)?;
    stage.place_version(release, home)?;
    stage.place_launcher(release, home)?;
    stage.place_current(home)
}

fn finish(home: &Path, modify_path: bool) -> io::Result<()> {
    if modify_path {
        user_path::add(home)
    } else {
        hint(home)
    }
}

/// The staging folder, removed with whatever is left in it when dropped.
struct Stage(PathBuf);

impl Stage {
    /// An empty staging folder, after removing one an interrupted install left.
    fn new(home: &Path) -> io::Result<Self> {
        let root = home.join(STAGE);
        if root.try_exists()? {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    /// Copies the release's version folder into the stage and puts it in
    /// place. A version folder it replaces waits in the stage to be removed.
    fn place_version(&self, release: &Path, home: &Path) -> io::Result<()> {
        let staged = self.0.join("version");
        fs::create_dir(&staged)?;
        for name in version_files() {
            copy(&release.join(VERSION).join(&name), &staged.join(&name))?;
        }
        place_version(&staged, &home.join(VERSION), &self.0.join("replaced"))
    }

    fn place_launcher(&self, release: &Path, home: &Path) -> io::Result<()> {
        let staged = self.0.join(LAUNCHER);
        copy(&release.join(LAUNCHER), &staged)?;
        place_launcher(&staged, &home.join(LAUNCHER))
    }

    fn place_current(&self, home: &Path) -> io::Result<()> {
        let staged = self.0.join("current");
        fs::write(&staged, format!("{VERSION}\n"))?;
        fs::rename(staged, home.join("current"))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy(from: &Path, to: &Path) -> io::Result<()> {
    fs::copy(from, to).map(drop).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot copy {}: {error}", from.display()),
        )
    })
}

/// Puts the staged version folder in place of the installed one: keeps an
/// identical one, and swaps a different one unless a program runs from it.
fn place_version(staged: &Path, installed: &Path, aside: &Path) -> io::Result<()> {
    if !installed.try_exists()? {
        return fs::rename(staged, installed);
    }
    if identical(staged, installed)? {
        return Ok(());
    }
    swap(staged, installed, aside)
}

/// Whether both folders hold the same file names with the same contents.
fn identical(staged: &Path, installed: &Path) -> io::Result<bool> {
    let names = names(staged)?;
    if names != self::names(installed)? {
        return Ok(false);
    }
    same_files(staged, installed, &names)
}

fn same_files(first: &Path, second: &Path, names: &BTreeSet<OsString>) -> io::Result<bool> {
    for name in names {
        if !same_contents(&first.join(name), &second.join(name))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn names(folder: &Path) -> io::Result<BTreeSet<OsString>> {
    fs::read_dir(folder)?
        .map(|entry| Ok(entry?.file_name()))
        .collect()
}

fn same_contents(first: &Path, second: &Path) -> io::Result<bool> {
    Ok(fs::read(first)? == fs::read(second)?)
}

/// Replaces the installed version folder with the staged one, unless a
/// program runs from it. Windows cannot swap two folders in one step.
fn swap(staged: &Path, installed: &Path, aside: &Path) -> io::Result<()> {
    refuse_in_use(installed)?;
    fs::rename(installed, aside)?;
    fs::rename(staged, installed)
}

/// Fails if a program runs from `folder`: Windows refuses to open a running
/// program's file for writing, and any refusal counts as running.
fn refuse_in_use(folder: &Path) -> io::Result<()> {
    for name in names(folder)? {
        let path = folder.join(name);
        if is_program(&path) && File::options().write(true).open(&path).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!(
                    "{} is running and the version being installed differs from it; \
                     end that version's sessions, then install again",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn is_program(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Replaces the installed launcher unless it is identical. Every launcher runs
/// every version, so this is best effort: Windows refuses to replace a running
/// program, and a launcher that cannot be replaced is kept.
fn place_launcher(staged: &Path, installed: &Path) -> io::Result<()> {
    if installed.try_exists()? && same_contents(staged, installed)? {
        return Ok(());
    }
    fs::rename(staged, installed).or_else(|error| {
        writeln!(
            io::stdout(),
            "climon2: kept {}, which is in use and could not be replaced ({error}); \
             it runs this version too",
            installed.display()
        )
    })
}

/// `--no-modify-path`: the entry the user may add to `PATH` themselves.
fn hint(home: &Path) -> io::Result<()> {
    writeln!(
        io::stdout(),
        "climon2: PATH was not changed. To run c2, add this entry to your user PATH:\n{}",
        home.display()
    )
}
