//! What `c2 monitor` is told (spec #488, CM6): nothing, which starts the
//! monitor, or a verb for the hub monitor's links. Pure data: nothing here
//! opens a socket or reads a PIN.
use crate::cli::UsageError;
use std::ffi::OsString;

const PAIR: &str = "pair";
const PAIR_SHORTCUT: &str = "p";

/// The arguments of `c2 monitor`, and of the monitor executable the CLI
/// hands them to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MonitorArgs {
    /// Bare `c2 monitor`: the monitor itself.
    Start,
    /// `c2 monitor pair <host:port>`: pair with the relay at `address`, as
    /// the user typed it. The PIN is read from the terminal or stdin, never
    /// from an argument (P13, CM10).
    Pair { address: String },
}

impl MonitorArgs {
    /// Nothing, or `pair` (or `p`) and one nonempty address. Anything else
    /// is a usage error.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, UsageError> {
        let arguments: Vec<_> = arguments.into_iter().collect();
        match arguments.as_slice() {
            [] => Ok(Self::Start),
            [verb, address] if verb == PAIR || verb == PAIR_SHORTCUT => pair(address),
            _ => Err(UsageError),
        }
    }

    /// These arguments as [`MonitorArgs::parse`] reads them back.
    pub fn arguments(&self) -> Vec<OsString> {
        match self {
            Self::Start => Vec::new(),
            Self::Pair { address } => vec![PAIR.into(), address.into()],
        }
    }
}

/// `pair`'s address: Unicode, not empty and not a flag, so that no flag is
/// ever taken for one.
fn pair(address: &OsString) -> Result<MonitorArgs, UsageError> {
    address
        .to_str()
        .filter(|address| !address.is_empty() && !address.starts_with('-'))
        .map(|address| MonitorArgs::Pair {
            address: address.to_owned(),
        })
        .ok_or(UsageError)
}
