//! Session environment is a pure projection of exact inherited OS strings.
use crate::{
    SessionId,
    exit::{INTERRUPTED, TERMINAL_GONE},
    lifetime::Lifetime,
};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{self, Read},
    num::NonZeroU32,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DepthOverflow;

impl fmt::Display for DepthOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("session depth exceeds u32")
    }
}

impl std::error::Error for DepthOverflow {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEnvironment {
    pub values: Vec<(OsString, OsString)>,
    pub depth: NonZeroU32,
    /// Stderr text without a trailing newline; absent at top level.
    pub nested_warning: Option<String>,
}

impl SessionEnvironment {
    /// A nonempty enclosing session ID indicates nesting. The session variables
    /// and the command's terminal identity (`TERM=xterm-256color`,
    /// `COLORTERM=truecolor`) replace any inherited value, the client startup
    /// envelope is dropped, and everything else passes through exactly.
    pub fn for_session(
        id: &SessionId,
        inherited: &[(OsString, OsString)],
    ) -> Result<Self, DepthOverflow> {
        let enclosing = enclosing_depth(inherited);
        let depth = NonZeroU32::MIN
            .checked_add(enclosing.unwrap_or(0))
            .ok_or(DepthOverflow)?;
        let nested_warning = enclosing.map(|value| {
            format!(
                "Starting climon2 session nested for the {} time",
                ordinal(value)
            )
        });
        let mut values: Vec<_> = inherited
            .iter()
            .filter(|(key, _)| !is_replaced_key(key))
            .cloned()
            .collect();
        values.push(("CLIMON2_SESSION_ID".into(), id.as_str().into()));
        values.push(("CLIMON2_SESSION_DEPTH".into(), depth.to_string().into()));
        values.extend(
            TERMINAL_IDENTITY
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into())),
        );
        Ok(Self {
            values,
            depth,
            nested_warning,
        })
    }
}

impl SessionEnvironment {
    /// What `exec` shows before it starts this session: nothing at top level
    /// or for a deliberate nested start; otherwise the nesting warning, and
    /// for an ordinary session, whose client clears the screen, the prompt.
    pub fn notice(&self, lifetime: Lifetime, start: NestedStart) -> Notice<'_> {
        let Some(warning) = self.nested_warning.as_deref() else {
            return Notice::Quiet;
        };
        match (start, lifetime) {
            (NestedStart::Deliberate, _) => Notice::Quiet,
            (NestedStart::Warned, Lifetime::Headless) => Notice::Warn(warning),
            (NestedStart::Warned, Lifetime::Ordinary) => Notice::Ask(warning),
        }
    }
}

/// How `exec` starts a session nested in another: warned by default, or
/// deliberately, with `--nested`, which neither warns nor waits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NestedStart {
    Warned,
    Deliberate,
}

/// What `exec` shows on stderr before it starts a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notice<'a> {
    Quiet,
    /// The warning, then the session starts.
    Warn(&'a str),
    /// The warning and [`PROMPT`]; the session starts only on Enter.
    Ask(&'a str),
}

/// The line under the nesting warning while `exec` waits for a key.
pub const PROMPT: &str = "Press Enter to start it, or Ctrl+C to cancel.";

/// The key that answered [`PROMPT`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    /// Enter: the session starts.
    Start,
    /// Ctrl+C: nothing starts.
    Cancel,
    /// The terminal went away before either.
    Gone,
}

impl Answer {
    /// Reads keys from a raw terminal until Enter or Ctrl+C, ignoring any
    /// other; the end of input is [`Answer::Gone`].
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        let mut keys = [0u8; 64];
        loop {
            let count = input.read(&mut keys)?;
            if count == 0 {
                return Ok(Self::Gone);
            }
            if let Some(answer) = keys[..count].iter().find_map(Self::of_key) {
                return Ok(answer);
            }
        }
    }

    fn of_key(key: &u8) -> Option<Self> {
        match key {
            b'\r' | b'\n' => Some(Self::Start),
            0x03 => Some(Self::Cancel),
            _ => None,
        }
    }

    /// `exec`'s exit status when nothing starts: 130 as for Ctrl+C in a
    /// client, or 129 as for a client's terminal that went away.
    pub fn status(self) -> Option<u32> {
        match self {
            Self::Start => None,
            Self::Cancel => Some(INTERRUPTED),
            Self::Gone => Some(TERMINAL_GONE),
        }
    }
}

/// The terminal identity every session's command sees.
pub const TERMINAL_IDENTITY: [(&str, &str); 2] =
    [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")];

const REPLACED_KEYS: [&str; 5] = [
    "CLIMON2_SESSION_ID",
    "CLIMON2_SESSION_DEPTH",
    "TERM",
    "COLORTERM",
    crate::startup_envelope::ENVIRONMENT_VARIABLE,
];

fn is_replaced_key(key: &OsStr) -> bool {
    REPLACED_KEYS.iter().any(|replaced| key == *replaced)
}

fn lookup<'a>(environment: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    environment
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_os_str())
}

fn enclosing_depth(environment: &[(OsString, OsString)]) -> Option<u32> {
    let id = lookup(environment, "CLIMON2_SESSION_ID")?;
    if id.is_empty() {
        return None;
    }
    Some(
        lookup(environment, "CLIMON2_SESSION_DEPTH")
            .and_then(parse_depth)
            .unwrap_or(1),
    )
}

fn parse_depth(value: &OsStr) -> Option<u32> {
    let text = value.to_str()?;
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<NonZeroU32>().ok().map(NonZeroU32::get)
}

const ORDINAL_SUFFIXES: [&str; 10] = ["th", "st", "nd", "rd", "th", "th", "th", "th", "th", "th"];

pub fn ordinal(value: u32) -> String {
    let suffix = if (11..=13).contains(&(value % 100)) {
        "th"
    } else {
        ORDINAL_SUFFIXES[(value % 10) as usize]
    };
    format!("{value}{suffix}")
}
