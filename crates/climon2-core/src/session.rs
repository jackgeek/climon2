//! Session environment is a pure projection of exact inherited OS strings.
use crate::SessionId;
use std::{
    ffi::{OsStr, OsString},
    fmt,
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
    /// A nonempty enclosing session ID indicates nesting. Only the two session
    /// variables are overwritten; TERM is defaulted only if completely absent.
    pub fn for_session(
        id: &SessionId,
        inherited: &[(OsString, OsString)],
    ) -> Result<Self, DepthOverflow> {
        let enclosing = enclosing_depth(inherited);
        let depth = match enclosing {
            Some(value) => {
                NonZeroU32::new(value.checked_add(1).ok_or(DepthOverflow)?).ok_or(DepthOverflow)?
            }
            None => NonZeroU32::MIN,
        };
        let nested_warning = enclosing.map(|value| {
            format!(
                "Starting climon2 session nested for the {} time",
                ordinal(value)
            )
        });
        let mut values: Vec<_> = inherited
            .iter()
            .filter(|(key, _)| !is_session_key(key))
            .cloned()
            .collect();
        values.push(("CLIMON2_SESSION_ID".into(), id.as_str().into()));
        values.push(("CLIMON2_SESSION_DEPTH".into(), depth.to_string().into()));
        if lookup(inherited, "TERM").is_none() {
            values.push(("TERM".into(), "xterm-256color".into()));
        }
        Ok(Self {
            values,
            depth,
            nested_warning,
        })
    }
}

fn is_session_key(key: &OsStr) -> bool {
    key == "CLIMON2_SESSION_ID" || key == "CLIMON2_SESSION_DEPTH"
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

pub fn ordinal(value: u32) -> String {
    let suffix = match value % 100 {
        11..=13 => "th",
        _ => match value % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    format!("{value}{suffix}")
}
