//! CLI argument data without terminal, process or environment access.
use crate::{lifetime::Lifetime, selector::Selector};
use std::{ffi::OsString, fmt, iter::Peekable, vec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandArgs {
    pub command: OsString,
    pub args: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliArgs {
    Exec {
        lifetime: Lifetime,
        command: CommandArgs,
    },
    List {
        json: bool,
    },
    Attach(Selector),
    Kill(Selector),
    Monitor,
    /// `home`: the climon2 home, its owner and whether this host owns it.
    Home,
    /// `home release`: remove the home's owner record (ADR 0011).
    HomeRelease,
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

pub const USAGE: &str = "usage: c2 exec|x [--headless|-h] <command> [args...] | attach|a <session-id> | kill|k <session-id> | list|l [--json] | monitor|m | home|h [release]";

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
        match SUBCOMMANDS
            .iter()
            .find(|command| name.is_some_and(|name| command.is_named(name)))
        {
            Some(command) => (command.parse)(remaining),
            None => parse_flag(name, &remaining),
        }
    }
}

/// Each command's full name and its single-letter shortcut, in usage order:
/// the one list the parser and the usage line's test share.
pub fn commands() -> impl Iterator<Item = (&'static str, &'static str)> {
    SUBCOMMANDS
        .iter()
        .map(|command| (command.name, command.shortcut))
}

type Parser = fn(Vec<OsString>) -> Result<CliArgs, UsageError>;

/// A command: its full name, its single-letter shortcut (a pure synonym) and
/// its parser.
struct Subcommand {
    name: &'static str,
    shortcut: &'static str,
    parse: Parser,
}

impl Subcommand {
    fn is_named(&self, name: &str) -> bool {
        name == self.name || name == self.shortcut
    }
}

/// Every command, in usage order. `r` is reserved for `relay`.
const SUBCOMMANDS: [Subcommand; 6] = [
    Subcommand {
        name: "exec",
        shortcut: "x",
        parse: parse_exec,
    },
    Subcommand {
        name: "attach",
        shortcut: "a",
        parse: |arguments| parse_selector(arguments, CliArgs::Attach),
    },
    Subcommand {
        name: "kill",
        shortcut: "k",
        parse: |arguments| parse_selector(arguments, CliArgs::Kill),
    },
    Subcommand {
        name: "list",
        shortcut: "l",
        parse: parse_list,
    },
    Subcommand {
        name: "monitor",
        shortcut: "m",
        parse: parse_monitor,
    },
    Subcommand {
        name: "home",
        shortcut: "h",
        parse: parse_home,
    },
];

/// Help and version flags accept no further arguments.
fn parse_flag(name: Option<&str>, remaining: &[OsString]) -> Result<CliArgs, UsageError> {
    match (name, remaining.is_empty()) {
        (Some("--help"), true) => Ok(CliArgs::Help),
        (Some("--version" | "-V"), true) => Ok(CliArgs::Version),
        _ => Err(UsageError),
    }
}

fn parse_exec(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    let mut args = arguments.into_iter().peekable();
    let lifetime = lifetime(&mut args);
    Ok(CliArgs::Exec {
        lifetime,
        command: command(args)?,
    })
}

/// `--headless`, or its short form `-h`, is exec's only option, and only as
/// the first argument.
fn lifetime(args: &mut Peekable<vec::IntoIter<OsString>>) -> Lifetime {
    args.next_if(|arg| arg == "--headless" || arg == "-h")
        .map_or(Lifetime::Ordinary, |_| Lifetime::Headless)
}

/// An optional `--`, then a nonempty command and its untouched arguments.
fn command(mut args: impl Iterator<Item = OsString>) -> Result<CommandArgs, UsageError> {
    let mut command = args.next().ok_or(UsageError)?;
    if command == "--" {
        command = args.next().ok_or(UsageError)?;
    }
    if command.is_empty() {
        return Err(UsageError);
    }
    Ok(CommandArgs {
        command,
        args: args.collect(),
    })
}

fn parse_list(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    match arguments.as_slice() {
        [] => Ok(CliArgs::List { json: false }),
        [flag] if flag == "--json" => Ok(CliArgs::List { json: true }),
        _ => Err(UsageError),
    }
}

/// `monitor` takes no arguments.
fn parse_monitor(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    arguments
        .is_empty()
        .then_some(CliArgs::Monitor)
        .ok_or(UsageError)
}

/// `home` takes nothing, or `release` alone.
fn parse_home(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    match arguments.as_slice() {
        [] => Ok(CliArgs::Home),
        [action] if action == "release" => Ok(CliArgs::HomeRelease),
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
