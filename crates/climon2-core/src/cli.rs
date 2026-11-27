//! CLI argument data without terminal, process or environment access.
use crate::{lifetime::Lifetime, selector::Selector, session::NestedStart};
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
        nested: NestedStart,
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
    /// `home release [--force]`: remove the home's owner record (ADR 0011);
    /// `--force` releases it while it holds sessions, this host's or another's.
    HomeRelease {
        force: bool,
    },
    /// A command the home owner check can refuse, given `--force` as its first
    /// argument: a home another host owns, or whose owner record cannot be read,
    /// is used anyway, unclaimed (ADR 0011).
    Forced(Box<CliArgs>),
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

pub const USAGE: &str = "usage: c2 exec|x [--force] [--headless|-h] [--nested|-n] <command> [args...] | attach|a [--force] <session-id> | kill|k [--force] <session-id> | list|l [--force] [--json] | monitor|m [--force] | home|h [release [--force]]";

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(USAGE)
    }
}

impl std::error::Error for UsageError {}

impl CliArgs {
    /// The command itself, and whether `--force` was given before its arguments.
    pub fn unforced(self) -> (Self, bool) {
        match self {
            Self::Forced(command) => (*command, true),
            command => (command, false),
        }
    }

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
            Some(command) => command.parse_forcible(remaining),
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

/// A command: its full name, its single-letter shortcut (a pure synonym), its
/// parser, and whether it takes `--force` first: attach, kill, list and
/// monitor, which the home owner check can refuse. `exec` takes it among its
/// own options.
struct Subcommand {
    name: &'static str,
    shortcut: &'static str,
    parse: Parser,
    forcible: bool,
}

impl Subcommand {
    /// `--force` only as the first argument, once: a second is the command's
    /// own, as `exec`'s options are.
    fn parse_forcible(&self, arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
        match arguments.split_first() {
            Some((first, rest)) if self.forcible && first == "--force" => {
                (self.parse)(rest.to_vec()).map(|command| forced(command, true))
            }
            _ => (self.parse)(arguments),
        }
    }

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
        forcible: false,
    },
    Subcommand {
        name: "attach",
        shortcut: "a",
        parse: |arguments| parse_selector(arguments, CliArgs::Attach),
        forcible: true,
    },
    Subcommand {
        name: "kill",
        shortcut: "k",
        parse: |arguments| parse_selector(arguments, CliArgs::Kill),
        forcible: true,
    },
    Subcommand {
        name: "list",
        shortcut: "l",
        parse: parse_list,
        forcible: true,
    },
    Subcommand {
        name: "monitor",
        shortcut: "m",
        parse: parse_monitor,
        forcible: true,
    },
    Subcommand {
        name: "home",
        shortcut: "h",
        parse: parse_home,
        forcible: false,
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
    let options = exec_options(&mut args);
    let exec = CliArgs::Exec {
        lifetime: if options.contains(&ExecOption::Headless) {
            Lifetime::Headless
        } else {
            Lifetime::Ordinary
        },
        nested: if options.contains(&ExecOption::Nested) {
            NestedStart::Deliberate
        } else {
            NestedStart::Warned
        },
        command: command(args)?,
    };
    Ok(forced(exec, options.contains(&ExecOption::Force)))
}

/// `command`, given `--force` or not (ADR 0011).
fn forced(command: CliArgs, force: bool) -> CliArgs {
    if force {
        CliArgs::Forced(Box::new(command))
    } else {
        command
    }
}

/// exec's options. `--force` (ADR 0011) is one of them, so it goes in any
/// order with the others, as the other commands' only option it comes first.
#[derive(Clone, Copy, PartialEq)]
enum ExecOption {
    Headless,
    Nested,
    Force,
}

/// Each of exec's options: its long form and its short form. `--force` has
/// no short form, so its long form stands in for one.
const EXEC_OPTIONS: [(&str, &str, ExecOption); 3] = [
    ("--headless", "-h", ExecOption::Headless),
    ("--nested", "-n", ExecOption::Nested),
    ("--force", "--force", ExecOption::Force),
];

impl ExecOption {
    fn of(arg: &OsString) -> Option<Self> {
        EXEC_OPTIONS
            .iter()
            .find(|(long, short, _)| arg == long || arg == short)
            .map(|(_, _, option)| *option)
    }
}

/// exec's leading options, in any order, each at most once: a repeated
/// option is the command.
fn exec_options(args: &mut Peekable<vec::IntoIter<OsString>>) -> Vec<ExecOption> {
    let mut options = Vec::new();
    while let Some(option) = args
        .peek()
        .and_then(ExecOption::of)
        .filter(|option| !options.contains(option))
    {
        options.push(option);
        args.next();
    }
    options
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

/// `home` takes nothing, or `release` and then optionally `--force`.
fn parse_home(arguments: Vec<OsString>) -> Result<CliArgs, UsageError> {
    match arguments.split_first() {
        None => Ok(CliArgs::Home),
        Some((action, flags)) if action == "release" => parse_release(flags),
        Some(_) => Err(UsageError),
    }
}

fn parse_release(flags: &[OsString]) -> Result<CliArgs, UsageError> {
    match flags {
        [] => Ok(CliArgs::HomeRelease { force: false }),
        [flag] if flag == "--force" => Ok(CliArgs::HomeRelease { force: true }),
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
