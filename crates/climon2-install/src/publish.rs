use crate::payload::COMPANIONS;
use crate::transaction::Stage;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn commit(stage: &Stage, home: &Path, payload: &crate::Payload) -> io::Result<()> {
    version(stage, &home.join(payload.version().to_string()))?;
    #[cfg(feature = "test-hooks")]
    crate::hooks::checkpoint("version-published")?;
    fs::rename(stage.launcher(), home.join("climon2"))?;
    #[cfg(feature = "test-hooks")]
    crate::hooks::checkpoint("launcher-published")?;
    fs::rename(stage.current(), home.join("current"))
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
    let source_metadata = fs::metadata(source)?;
    if source_metadata.permissions().mode() != metadata.permissions().mode() {
        return Ok(false);
    }
    Ok(fs::read(source)? == fs::read(destination)?)
}
