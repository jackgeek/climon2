//! What `c2 relay` is told: its one flag, the address a relay listens on and
//! where that address comes from (spec #488). Pure data: nothing here opens
//! a socket.
use crate::{cli::UsageError, config::RelayConfig};
use std::{
    ffi::OsString,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
};

/// Where a relay listens when neither the flag nor the setting says:
/// loopback, so nothing is reachable from the network until its user
/// chooses an address.
pub const DEFAULT_LISTEN: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7222));

const LISTEN_FLAG: &str = "--listen";

/// A listen value: an IP literal and a port, such as `127.0.0.1:7222`,
/// `[::1]:7222` or `0.0.0.0:0`. A host name or a missing port is none, so
/// what a relay binds is always exactly what was written.
pub fn listen_address(text: &str) -> Option<SocketAddr> {
    text.parse().ok()
}

/// The arguments of `c2 relay`, and of the relay executable the CLI hands
/// them to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayArgs {
    /// `--listen <ip>:<port>`, when given.
    pub listen: Option<SocketAddr>,
}

impl RelayArgs {
    /// Nothing, or `--listen` and its value. Anything else is a usage error.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, UsageError> {
        let arguments: Vec<_> = arguments.into_iter().collect();
        match arguments.as_slice() {
            [] => Ok(Self { listen: None }),
            [flag, value] if flag == LISTEN_FLAG => value
                .to_str()
                .and_then(listen_address)
                .map(|address| Self {
                    listen: Some(address),
                })
                .ok_or(UsageError),
            _ => Err(UsageError),
        }
    }

    /// These arguments as [`RelayArgs::parse`] reads them back.
    pub fn arguments(&self) -> Vec<OsString> {
        match self.listen {
            Some(address) => vec![LISTEN_FLAG.into(), address.to_string().into()],
            None => Vec::new(),
        }
    }

    /// The address to listen on: the flag, else the `relay.listen` setting,
    /// else [`DEFAULT_LISTEN`].
    pub fn listen(&self, configured: &RelayConfig) -> SocketAddr {
        self.listen.or(configured.listen).unwrap_or(DEFAULT_LISTEN)
    }
}
