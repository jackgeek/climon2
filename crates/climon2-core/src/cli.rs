//! CLI argument data without terminal, process or environment access.
use crate::selector::Selector;
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
    Attach(Selector),
    Kill(Selector),
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

pub const USAGE: &str =
    "usage: c2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json]";

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(USAGE)
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
        let name = subcommand.to_str();
        match SUBCOMMANDS.iter().find(|(known, _)| Some(*known) == name) {
            Some((_, parse)) => parse(remaining),
            None => parse_flag(name, &remaining),
        }
    }
}

type Parser = fn(Vec<OsString>) -> Result<CliArgs, UsageError>;

const SUBCOMMANDS: [(&str, Parser); 4] = [
    ("exec", parse_exec),
    ("list", parse_list),
    ("attach", |arguments| {
        parse_selector(arguments, CliArgs::Attach)
    }),
    ("kill", |arguments| parse_selector(arguments, CliArgs::Kill)),
];

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

/// `attach` and `kill` take exactly one selector.
fn parse_selector(
    arguments: Vec<OsString>,
    operation: fn(Selector) -> CliArgs,
) -> Result<CliArgs, UsageError> {
    match arguments.as_slice() {
        [selector] => Selector::parse(selector).map(operation),
        _ => Err(UsageError),
    }
}
