use climon2_core::{SessionId, VersionName, protocol::TerminalSize};
use std::{collections::BTreeMap, ffi::OsString, io, path::PathBuf};

pub(crate) struct Options {
    pub id: SessionId,
    pub version: VersionName,
    pub home: PathBuf,
    pub socket: PathBuf,
    pub cwd: PathBuf,
    pub cli_pid: u32,
    pub readiness_fd: u32,
    pub size: TerminalSize,
    pub command: OsString,
    pub arguments: Vec<OsString>,
    pub log: Option<String>,
}

impl Options {
    pub fn parse(arguments: Vec<OsString>) -> io::Result<Self> {
        let (fields, command, arguments) = fields(arguments)?;
        Self::from_fields(fields, command, arguments)
    }

    fn from_fields(
        mut fields: BTreeMap<OsString, OsString>,
        command: OsString,
        arguments: Vec<OsString>,
    ) -> io::Result<Self> {
        let (id, version, cli_pid) = identity(&mut fields)?;
        let (home, socket, cwd) = locations(&mut fields)?;
        let (readiness_fd, size) = transport(&mut fields)?;
        let log = fields
            .remove(std::ffi::OsStr::new("--log"))
            .map(|value| value.into_string().map_err(|_| invalid()))
            .transpose()?;
        if !fields.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            id,
            version,
            home,
            socket,
            cwd,
            cli_pid,
            readiness_fd,
            size,
            command,
            arguments,
            log,
        })
    }
}

fn fields(
    arguments: Vec<OsString>,
) -> io::Result<(BTreeMap<OsString, OsString>, OsString, Vec<OsString>)> {
    let mut arguments = arguments.into_iter();
    let fields = option_fields(&mut arguments)?;
    Ok((
        fields,
        arguments.next().ok_or_else(invalid)?,
        arguments.collect(),
    ))
}

fn option_fields(
    arguments: &mut std::vec::IntoIter<OsString>,
) -> io::Result<BTreeMap<OsString, OsString>> {
    let mut fields = BTreeMap::new();
    loop {
        let key = arguments.next().ok_or_else(invalid)?;
        if key == "--" {
            break;
        }
        let value = arguments.next().ok_or_else(invalid)?;
        if fields.insert(key, value).is_some() {
            return Err(invalid());
        }
    }
    Ok(fields)
}

fn identity(
    fields: &mut BTreeMap<OsString, OsString>,
) -> io::Result<(SessionId, VersionName, u32)> {
    let id = number(fields, "--session-id")?;
    let version = number(fields, "--version")?;
    let cli_pid = number(fields, "--cli-pid")?;
    if cli_pid == 0 {
        return Err(invalid());
    }
    Ok((id, version, cli_pid))
}

fn locations(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    Ok((
        path(fields, "--home")?,
        path(fields, "--socket")?,
        path(fields, "--cwd")?,
    ))
}

fn transport(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<(u32, TerminalSize)> {
    Ok((
        number(fields, "--readiness-fd")?,
        TerminalSize {
            cols: number(fields, "--cols")?,
            rows: number(fields, "--rows")?,
        }
        .initial(),
    ))
}

fn invalid() -> io::Error {
    super::io_error("invalid daemon invocation")
}

fn take(fields: &mut BTreeMap<OsString, OsString>, key: &str) -> io::Result<OsString> {
    fields.remove(std::ffi::OsStr::new(key)).ok_or_else(invalid)
}

fn text(fields: &mut BTreeMap<OsString, OsString>, key: &str) -> io::Result<String> {
    take(fields, key)?.into_string().map_err(|_| invalid())
}

fn path(fields: &mut BTreeMap<OsString, OsString>, key: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(take(fields, key)?);
    if !path.is_absolute() {
        return Err(invalid());
    }
    Ok(path)
}

fn number<T: std::str::FromStr>(
    fields: &mut BTreeMap<OsString, OsString>,
    key: &str,
) -> io::Result<T> {
    text(fields, key)?.parse().map_err(|_| invalid())
}
