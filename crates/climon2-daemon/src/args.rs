use climon2_core::{SessionId, VersionName, lifetime::Lifetime, protocol::TerminalSize};
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
    pub max_history_rows: u32,
    pub lifetime: Lifetime,
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
        let (readiness_fd, size, max_history_rows) = transport(&mut fields)?;
        let (lifetime, log) = policy(fields)?;
        Ok(Self {
            id,
            version,
            home,
            socket,
            cwd,
            cli_pid,
            readiness_fd,
            size,
            max_history_rows,
            lifetime,
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
    while let Some((key, value)) = option_field(arguments)? {
        insert_unique(&mut fields, key, value)?;
    }
    Ok(fields)
}

/// The next `KEY VALUE` pair, or `None` at the `--` that ends the options.
fn option_field(
    arguments: &mut std::vec::IntoIter<OsString>,
) -> io::Result<Option<(OsString, OsString)>> {
    let key = arguments.next().ok_or_else(invalid)?;
    if key == "--" {
        return Ok(None);
    }
    Ok(Some((key, arguments.next().ok_or_else(invalid)?)))
}

fn insert_unique(
    fields: &mut BTreeMap<OsString, OsString>,
    key: OsString,
    value: OsString,
) -> io::Result<()> {
    if fields.insert(key, value).is_some() {
        return Err(invalid());
    }
    Ok(())
}

/// The session's lifetime policy, then the optional log filter.
fn policy(mut fields: BTreeMap<OsString, OsString>) -> io::Result<(Lifetime, Option<String>)> {
    let lifetime = number(&mut fields, "--lifetime")?;
    Ok((lifetime, only_log(fields)?))
}

/// `--log` is the only optional field; anything else left over is invalid.
fn only_log(mut fields: BTreeMap<OsString, OsString>) -> io::Result<Option<String>> {
    let log = fields
        .remove(std::ffi::OsStr::new("--log"))
        .map(|value| value.into_string().map_err(|_| invalid()))
        .transpose()?;
    if !fields.is_empty() {
        return Err(invalid());
    }
    Ok(log)
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

fn transport(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<(u32, TerminalSize, u32)> {
    let rows = scrollback_rows(fields)?;
    let size = size(fields)?;
    Ok((number(fields, "--readiness-fd")?, size, rows))
}

fn scrollback_rows(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<u32> {
    let rows = number(fields, "--scrollback-max-rows")?;
    (rows <= 100_000).then_some(rows).ok_or_else(invalid)
}

fn size(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<TerminalSize> {
    let size = TerminalSize {
        cols: number(fields, "--cols")?,
        rows: number(fields, "--rows")?,
    }
    .initial();
    Ok(TerminalSize {
        cols: size.cols.clamp(2, 500),
        rows: size.rows.min(200),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(max_rows: Option<&str>) -> Vec<OsString> {
        let mut args = vec![
            "--session-id",
            "01234567-89ab-4cde-8fab-0123456789ab",
            "--version",
            "0.1.0",
            "--cli-pid",
            "1",
            "--home",
            "/home/test",
            "--socket",
            "/tmp/session.sock",
            "--rows",
            "24",
            "--cols",
            "80",
            "--cwd",
            "/home/test",
            "--lifetime",
            "ordinary",
        ];
        let mut values: Vec<_> = args.drain(..).map(OsString::from).collect();
        if let Some(rows) = max_rows {
            values.extend([
                OsString::from("--scrollback-max-rows"),
                OsString::from(rows),
            ]);
        }
        values.extend([
            OsString::from("--readiness-fd"),
            OsString::from("3"),
            OsString::from("--"),
            OsString::from("shell"),
        ]);
        values
    }

    #[test]
    fn daemon_requires_a_valid_scrollback_limit() {
        for value in [None, Some("-1"), Some("100001"), Some("not-a-number")] {
            assert!(Options::parse(arguments(value)).is_err(), "{value:?}");
        }
        assert_eq!(
            Options::parse(arguments(Some("0")))
                .unwrap()
                .max_history_rows,
            0
        );
        assert_eq!(
            Options::parse(arguments(Some("100000")))
                .unwrap()
                .max_history_rows,
            100_000
        );
    }

    fn with_lifetime(lifetime: &str) -> Vec<OsString> {
        let mut values = arguments(Some("10000"));
        let key = values
            .iter()
            .position(|value| value == "--lifetime")
            .unwrap();
        values[key + 1] = lifetime.into();
        values
    }

    /// `--lifetime` spells the startup policy exactly as the CLI writes it.
    #[test]
    fn daemon_requires_a_known_lifetime() {
        assert_eq!(
            Options::parse(with_lifetime("ordinary")).unwrap().lifetime,
            Lifetime::Ordinary
        );
        assert_eq!(
            Options::parse(with_lifetime("headless")).unwrap().lifetime,
            Lifetime::Headless
        );
        for value in ["", "Headless", "persistent"] {
            assert!(Options::parse(with_lifetime(value)).is_err(), "{value:?}");
        }
    }
}
