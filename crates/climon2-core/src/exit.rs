//! Platform exit seams classify outcomes; core preserves raw command u32 codes.
use crate::ContractError;
use std::num::NonZeroU32;

pub const USAGE_ERROR: u32 = 2;
pub const CLIMON2_ERROR: u32 = 125;
pub const LAUNCH_NOT_FOUND: u32 = 127;
pub const LAUNCH_CANNOT_RUN: u32 = 126;
pub const TERMINAL_GONE: u32 = 129;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchFailure {
    NotFound,
    /// Includes invalid cwd as well as a command that exists but cannot run.
    CannotRun,
}

impl LaunchFailure {
    pub fn code(self) -> u32 {
        match self {
            Self::NotFound => LAUNCH_NOT_FOUND,
            Self::CannotRun => LAUNCH_CANNOT_RUN,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    Command(u32),
    Signal(NonZeroU32),
    Launch(LaunchFailure),
    OwnFailure,
    Usage,
    TerminalGone,
}

impl ExitStatus {
    pub fn code(self) -> Result<u32, ContractError> {
        match self {
            Self::Command(code) => Ok(code),
            Self::Signal(signal) => signal
                .get()
                .checked_add(128)
                .ok_or_else(|| ContractError("signal exit status exceeds u32".into())),
            Self::Launch(failure) => Ok(failure.code()),
            Self::OwnFailure => Ok(CLIMON2_ERROR),
            Self::Usage => Ok(USAGE_ERROR),
            Self::TerminalGone => Ok(TERMINAL_GONE),
        }
    }
}
