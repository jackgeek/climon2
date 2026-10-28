//! CLI argument data without terminal, process or environment access.
use std::{ffi::OsString, fmt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandArgs {
    pub command: OsString,
    pub args: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliArgs {
    Exec(CommandArgs),
    List { json: bool },
    Help,
    Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageError;

impl UsageError {
    pub fn exit_code(&self) -> u32 {
        crate::exit::USAGE_ERROR
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("usage: climon2 exec <command> [args...] | list [--json]")
    }
}

impl std::error::Error for UsageError {}

impl CliArgs {
    /// Arguments exclude argv[0]. Once the command is read, no trailing argument
    /// is interpreted by climon2; empty and non-Unicode arguments stay intact.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, UsageError> {
        let mut arguments = arguments.into_iter();
        let subcommand = arguments.next().ok_or(UsageError)?;
        let remaining: Vec<_> = arguments.collect();
        match subcommand.to_str() {
            Some("exec") => parse_exec(remaining),
            Some("list") => parse_list(remaining),
            name => parse_flag(name, &remaining),
        }
    }
}

/// Help and version flags accept no further arguments.
fn parse_flag(name: Option<&str>, remaining: &[OsString]) -> Result<CliArgs, UsageError> {
    match (name, remaining.is_empty()) {
        (Some("--help" | "-h"), true) => Ok(CliArgs::Help),
        (Some("--version" | "-V"), true) => Ok(CliArgs::Version),
        _ => Err(UsageError),
    }
}

fn parse_exec(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    let mut args = arguments.into_iter();
    let mut command = args.next().ok_or(UsageError)?;
    if command == "--" {
        command = args.next().ok_or(UsageError)?;
    }
    if command.is_empty() {
        return Err(UsageError);
    }
    Ok(CliArgs::Exec(CommandArgs {
        command,
        args: args.collect(),
    }))
}

fn parse_list(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    match arguments.as_slice() {
        [] => Ok(CliArgs::List { json: false }),
        [flag] if flag == "--json" => Ok(CliArgs::List { json: true }),
        _ => Err(UsageError),
    }
}
