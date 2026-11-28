//! Everything that can refuse the relay, checked in order before the
//! terminal changes: a terminal to own, the one configuration snapshot, the
//! home's lock and the listener. A relay that started holds the lock and the
//! bound listener until it ends.
use climon2_core::{
    config::{ConfigSource, RelayConfig},
    layout::HomeLayout,
    relay::RelayArgs,
};
use climon2_platform::{
    home_lock::{self, Held, Taken},
    link_socket::{self, LinkListener},
    private, terminal,
};
use std::{io, net::SocketAddr, path::Path};

/// The lock's file in a home's links folder.
const LOCK: &str = "relay.lock";

/// A validated start: nothing has touched the terminal yet.
pub(crate) struct Startup {
    /// Bound once, for the relay's whole life. Nothing accepts from it yet:
    /// connections are taken when pairing and links exist.
    _listener: LinkListener,
    address: SocketAddr,
    /// One relay per climon2 home.
    _lock: Held,
}

impl Startup {
    /// `home` is the canonical climon2 home.
    pub(crate) fn read(arguments: RelayArgs, home: &Path) -> io::Result<Self> {
        terminal::require_ttys().map_err(|_| io::Error::other("c2 relay needs a terminal"))?;
        Self::in_home(arguments, home)
    }

    /// The start once a terminal is known to be there.
    fn in_home(arguments: RelayArgs, home: &Path) -> io::Result<Self> {
        let home = HomeLayout::at(home).map_err(io::Error::other)?;
        let config = configuration(&home)?;
        let mut lock = lock(&home)?;
        let (listener, address) = listen(arguments.listen(&config))?;
        lock.note(&running(std::process::id(), address))?;
        Ok(Self {
            _listener: listener,
            address,
            _lock: lock,
        })
    }

    /// The address the listener is bound to: with port 0 asked for, the port
    /// the system picked.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
}

/// The one configuration snapshot: the `relay` section. Nothing reads the
/// file again.
fn configuration(home: &HomeLayout) -> io::Result<RelayConfig> {
    ConfigSource::read(home)
        .and_then(|source| source.relay())
        .map_err(io::Error::other)
}

/// The home's relay lock, in its owner-only links folder. A home whose relay
/// is running refuses, naming that relay.
fn lock(home: &HomeLayout) -> io::Result<Held> {
    let links = home.links();
    private::private_directory(&links)?;
    match home_lock::take(&links.join(LOCK))? {
        Taken::Held(held) => Ok(held),
        Taken::Busy(holder) => Err(io::Error::other(already_running(&holder))),
    }
}

/// What the running relay notes on its lock, for a second start to name it.
fn running(pid: u32, address: SocketAddr) -> String {
    format!("pid {pid}, listening on {address}")
}

/// The refusal of a second start; `holder` is the running relay's note,
/// empty when it has not written one yet.
fn already_running(holder: &str) -> String {
    let relay = "a relay is already running in this climon2 home";
    if holder.is_empty() {
        return relay.to_owned();
    }
    format!("{relay} ({holder})")
}

/// Binds the listener, once. A bind failure is a start failure that names
/// the address asked for.
fn listen(address: SocketAddr) -> io::Result<(LinkListener, SocketAddr)> {
    let refused = |error: io::Error| {
        io::Error::new(error.kind(), format!("cannot listen on {address}: {error}"))
    };
    let listener = link_socket::listen(address).map_err(refused)?;
    let bound = listener.local_address().map_err(refused)?;
    Ok((listener, bound))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        net::{Ipv4Addr, TcpListener, TcpStream},
        os::unix::fs::PermissionsExt,
    };

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn any_port() -> RelayArgs {
        RelayArgs {
            listen: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
        }
    }

    fn refusal(start: io::Result<Startup>) -> String {
        start.err().expect("the start is refused").to_string()
    }

    #[test]
    fn a_start_binds_the_listener_and_reports_the_port_it_got() {
        let home = home();
        let startup = Startup::in_home(any_port(), home.path()).unwrap();
        let address = startup.address();
        assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(address.port(), 0);
        TcpStream::connect(address).expect("the listener is bound");
        drop(startup);
        assert!(TcpStream::connect(address).is_err());
    }

    #[test]
    fn a_second_start_in_the_home_names_the_running_relay() {
        let home = home();
        let first = Startup::in_home(any_port(), home.path()).unwrap();
        assert_eq!(
            refusal(Startup::in_home(any_port(), home.path())),
            format!(
                "a relay is already running in this climon2 home (pid {}, listening on {})",
                std::process::id(),
                first.address()
            )
        );
        let elsewhere = self::home();
        Startup::in_home(any_port(), elsewhere.path()).expect("another home runs its own relay");
        drop(first);
        Startup::in_home(any_port(), home.path()).expect("the home is free again");
    }

    #[test]
    fn the_lock_is_an_owner_only_file_in_an_owner_only_links_folder() {
        let home = home();
        let _startup = Startup::in_home(any_port(), home.path()).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(&home.path().join("links")), 0o700);
        assert_eq!(mode(&home.path().join("links/relay.lock")), 0o600);
    }

    #[test]
    fn a_port_in_use_refuses_the_start_and_frees_the_home() {
        let home = home();
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = taken.local_addr().unwrap();
        let listen = Some(address);
        let error = Startup::in_home(RelayArgs { listen }, home.path())
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("cannot listen on {address}: ")),
            "{text}"
        );
        Startup::in_home(any_port(), home.path()).expect("the failed start left no lock");
    }

    #[test]
    fn the_setting_names_the_address_and_the_flag_overrides_it() {
        let home = home();
        let free = |()| {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            listener.local_addr().unwrap()
        };
        let configured = free(());
        fs::write(
            home.path().join("config.jsonc"),
            format!(r#"{{"relay": {{"listen": "{configured}"}}}}"#),
        )
        .unwrap();
        let startup = Startup::in_home(RelayArgs { listen: None }, home.path()).unwrap();
        assert_eq!(startup.address(), configured);
        drop(startup);
        let flagged = free(());
        let listen = Some(flagged);
        let startup = Startup::in_home(RelayArgs { listen }, home.path()).unwrap();
        assert_eq!(startup.address(), flagged);
    }

    #[test]
    fn bad_configuration_refuses_the_start_before_anything_is_locked_or_bound() {
        let home = home();
        fs::write(
            home.path().join("config.jsonc"),
            r#"{"relay": {"listen": "localhost:7222"}}"#,
        )
        .unwrap();
        let text = refusal(Startup::in_home(any_port(), home.path()));
        assert!(
            text.starts_with("config.jsonc is invalid: relay.listen must be"),
            "{text}"
        );
        assert!(!home.path().join("links").exists());
    }

    #[test]
    fn a_relay_that_has_noted_nothing_yet_is_still_named_as_running() {
        assert_eq!(
            already_running(""),
            "a relay is already running in this climon2 home"
        );
        assert_eq!(
            already_running("pid 7, listening on [::1]:9"),
            "a relay is already running in this climon2 home (pid 7, listening on [::1]:9)"
        );
        assert_eq!(
            running(7, SocketAddr::from((Ipv4Addr::LOCALHOST, 7222))),
            "pid 7, listening on 127.0.0.1:7222"
        );
    }

    /// The test harness runs with no terminal on its standard input.
    #[test]
    fn a_start_without_a_terminal_is_refused_before_the_home_is_read() {
        if terminal::require_ttys().is_ok() {
            return;
        }
        let home = home();
        assert_eq!(
            refusal(Startup::read(any_port(), home.path())),
            "c2 relay needs a terminal"
        );
        assert!(!home.path().join("links").exists());
    }
}
