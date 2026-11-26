use climon2_core::{
    SessionId, VersionName, config::Assembly, lifetime::Lifetime, protocol::TerminalSize,
};
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
    /// The notification assembly limits `exec` validated, fixed for the
    /// session's lifetime.
    pub assembly: Assembly,
    pub lifetime: Lifetime,
    /// `--home-owner forced`, from `exec --force`: the home is used even when
    /// another host owns it or its record cannot be read (ADR 0011).
    pub force: bool,
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
        let (readiness_fd, size, (max_history_rows, assembly)) = transport(&mut fields)?;
        let (lifetime, force, log) = policy(fields)?;
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
            assembly,
            lifetime,
            force,
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

/// The session's lifetime policy, whether the home owner check is forced,
/// then the optional log filter.
fn policy(
    mut fields: BTreeMap<OsString, OsString>,
) -> io::Result<(Lifetime, bool, Option<String>)> {
    let lifetime = number(&mut fields, "--lifetime")?;
    let force = forced(&mut fields)?;
    Ok((lifetime, force, only_log(fields)?))
}

/// `--home-owner` is optional, and `forced` is its only value.
fn forced(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<bool> {
    match fields.remove(std::ffi::OsStr::new("--home-owner")) {
        None => Ok(false),
        Some(value) if value == "forced" => Ok(true),
        Some(_) => Err(invalid()),
    }
}

/// `--log` is the last optional field; anything else left over is invalid.
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

fn transport(
    fields: &mut BTreeMap<OsString, OsString>,
) -> io::Result<(u32, TerminalSize, (u32, Assembly))> {
    let limits = limits(fields)?;
    let size = size(fields)?;
    Ok((number(fields, "--readiness-fd")?, size, limits))
}

/// The notification assembly limits, each within its setting's range.
fn assembly(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<Assembly> {
    let records = number(fields, "--assembly-records")?;
    let bytes = number(fields, "--assembly-bytes")?;
    let lifetime_ms = number(fields, "--assembly-ms")?;
    Assembly::new(records, bytes, lifetime_ms).ok_or_else(invalid)
}

/// The session's retained history rows and notification assembly limits,
/// fixed for its lifetime.
fn limits(fields: &mut BTreeMap<OsString, OsString>) -> io::Result<(u32, Assembly)> {
    let rows = number(fields, "--scrollback-max-rows")?;
    let rows = (rows <= 100_000).then_some(rows).ok_or_else(invalid)?;
    Ok((rows, assembly(fields)?))
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

    const ASSEMBLY: [&str; 6] = [
        "--assembly-records",
        "32",
        "--assembly-bytes",
        "65536",
        "--assembly-ms",
        "5000",
    ];

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
        values.extend(ASSEMBLY.map(OsString::from));
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

    /// `arguments` with the value of `key` replaced by `value`, or without
    /// `key` when `value` is `None`.
    fn replaced(key: &str, value: Option<&str>) -> Vec<OsString> {
        let mut values = arguments(Some("10000"));
        let at = values.iter().position(|value| value == key).unwrap();
        match value {
            Some(value) => values[at + 1] = value.into(),
            None => drop(values.drain(at..at + 2)),
        }
        values
    }

    /// LIM-01 (exec-provided assembly limits): the daemon requires the
    /// three notification assembly limits `exec` validated, each in its
    /// setting's range, and keeps them.
    #[test]
    fn daemon_requires_valid_assembly_limits() {
        let parsed = Options::parse(arguments(Some("10000"))).unwrap().assembly;
        assert_eq!(parsed, Assembly::new(32, 65_536, 5_000).unwrap());
        let edges = [
            ("--assembly-records", "4294967295"),
            ("--assembly-bytes", "18446744073709551615"),
            ("--assembly-ms", "3600000"),
        ];
        for (key, value) in edges {
            assert!(Options::parse(replaced(key, Some(value))).is_ok(), "{key}");
        }
        for key in ["--assembly-records", "--assembly-bytes", "--assembly-ms"] {
            for value in [None, Some("0"), Some("-1"), Some("x")] {
                assert!(
                    Options::parse(replaced(key, value)).is_err(),
                    "{key} {value:?}"
                );
            }
        }
        assert!(Options::parse(replaced("--assembly-ms", Some("3600001"))).is_err());
    }
}
