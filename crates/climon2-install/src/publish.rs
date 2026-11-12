use crate::payload::COMPANIONS;
use crate::transaction::Stage;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn commit(stage: &Stage, home: &Path, payload: &crate::Payload) -> io::Result<()> {
    short_command(home)?;
    version(stage, &home.join(payload.version().to_string()))?;
    #[cfg(feature = "test-hooks")]
    crate::hooks::checkpoint("version-published")?;
    launcher(stage, home)?;
    fs::rename(stage.current(), home.join("current"))
}

fn launcher(stage: &Stage, home: &Path) -> io::Result<()> {
    fs::rename(stage.launcher(), home.join("climon2"))?;
    #[cfg(feature = "test-hooks")]
    crate::hooks::checkpoint("launcher-published")?;
    Ok(())
}

fn short_command(home: &Path) -> io::Result<()> {
    let path = home.join("c2");
    create_short_command(&path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot install {}: {error}", path.display()),
        )
    })
}

fn create_short_command(path: &Path) -> io::Result<()> {
    #[cfg(feature = "test-hooks")]
    crate::hooks::checkpoint("short-command")?;
    match std::os::unix::fs::symlink("climon2", path) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => existing_short_command(path),
        result => result,
    }
}

fn existing_short_command(path: &Path) -> io::Result<()> {
    if fs::read_link(path)?.as_os_str() != "climon2" {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "expected a relative symlink to climon2",
        ));
    }
    Ok(())
}

fn version(stage: &Stage, destination: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(stage.version(), destination)
        }
        Err(error) => Err(error),
        Ok(metadata) => replace_version(stage, destination, metadata),
    }
}

fn replace_version(stage: &Stage, destination: &Path, metadata: fs::Metadata) -> io::Result<()> {
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "installed version is not a directory",
        ));
    }
    if identical(&stage.version(), destination)? {
        return Ok(());
    }
    crate::platform::swap(&stage.version(), destination)
}

fn identical(source: &Path, destination: &Path) -> io::Result<bool> {
    if fs::read_dir(destination)?.count() != COMPANIONS.len() {
        return Ok(false);
    }
    identical_companions(source, destination)
}

fn identical_companions(source: &Path, destination: &Path) -> io::Result<bool> {
    for name in COMPANIONS {
        if !identical_file(&source.join(name), &destination.join(name))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn identical_file(source: &Path, destination: &Path) -> io::Result<bool> {
    let Some(metadata) = regular_file(destination)? else {
        return Ok(false);
    };
    matching_file(source, destination, &metadata)
}

fn regular_file(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file().then_some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn matching_file(source: &Path, destination: &Path, metadata: &fs::Metadata) -> io::Result<bool> {
    if fs::metadata(source)?.permissions().mode() != metadata.permissions().mode() {
        return Ok(false);
    }
    same_contents(source, destination)
}

fn same_contents(source: &Path, destination: &Path) -> io::Result<bool> {
    Ok(fs::read(source)? == fs::read(destination)?)
}
