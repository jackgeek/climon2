//! Everything that can refuse the relay, checked in order before the
//! terminal changes: a terminal to own, a session that is not nested, the
//! one configuration snapshot, the home's lock, the pairing store and the
//! listener. A relay that started
//! holds the lock and the bound listener until it ends.
use crate::{
    discover::Discovery,
    links::{Links, Remembered, Sessions},
};
use climon2_core::{
    SessionId,
    config::{ConfigSource, RelayConfig},
    layout::HomeLayout,
    pairing::{self, Keypair, PrivateKey, PublicKey as StoredKey},
    relay::RelayArgs,
    session::enclosing_depth,
};
use climon2_link::{StaticKey, display_name};
use climon2_platform::{
    home_lock::{self, Held, Taken},
    host_name,
    key_storage::KeyStore,
    link_socket::{self, LinkListener},
    private, randomness, terminal,
};
use std::{ffi::OsString, io, net::SocketAddr, path::Path};

/// The lock's file in a home's links folder.
const LOCK: &str = "relay.lock";

/// A validated start: nothing has touched the terminal yet.
pub(crate) struct Startup {
    /// Bound once, for the relay's whole life.
    listener: LinkListener,
    address: SocketAddr,
    /// One relay per climon2 home.
    lock: Held,
    /// The relay's one identity key (P16, ST5).
    identity: StaticKey,
    /// The hub monitors it remembers.
    remembered: Remembered,
    /// The home whose sessions it serves.
    home: HomeLayout,
    /// The session it runs in, never served (U3).
    own: Option<SessionId>,
}

/// A relay that has started: it accepts connections, and holds its home's
/// lock, until this is dropped.
pub(crate) struct Serving {
    pub(crate) links: Links,
    pub(crate) address: SocketAddr,
    _lock: Held,
}

impl Startup {
    /// `home` is the canonical climon2 home.
    pub(crate) fn read(arguments: RelayArgs, home: &Path) -> io::Result<Self> {
        terminal::require_ttys().map_err(|_| io::Error::other("c2 relay needs a terminal"))?;
        let environment: Vec<_> = std::env::vars_os().collect();
        Self::in_home(arguments, home, &environment)
    }

    /// The start once a terminal is known to be there, in `environment`.
    fn in_home(
        arguments: RelayArgs,
        home: &Path,
        environment: &[(OsString, OsString)],
    ) -> io::Result<Self> {
        let own = enclosing(environment)?;
        let home = HomeLayout::at(home).map_err(io::Error::other)?;
        let config = configuration(&home)?;
        let address = arguments.listen(&config);
        Self::locked(lock(&home)?, home, address, own)
    }

    /// The start once the home is this relay's: it reads the pairing store
    /// and binds `address`.
    fn locked(
        lock: Held,
        home: HomeLayout,
        address: SocketAddr,
        own: Option<SessionId>,
    ) -> io::Result<Self> {
        let (identity, remembered) = keys(&home)?;
        let (listener, address) = listen(address)?;
        Self {
            listener,
            address,
            lock,
            identity,
            remembered,
            home,
            own,
        }
        .noted()
    }

    /// Notes on the lock, once the listener is bound, what a second start
    /// names.
    fn noted(mut self) -> io::Result<Self> {
        self.lock.note(&running(std::process::id(), self.address))?;
        Ok(self)
    }

    /// Starts accepting connections, as the relay serves until it ends.
    pub(crate) fn serve(self) -> Serving {
        let name = display_name(&host_name::get().unwrap_or_default());
        let home = self.home;
        let sessions = Sessions {
            own: self.own,
            discover: Box::new(move || Discovery::start(home.clone())),
        };
        Serving {
            links: Links::start(
                self.listener,
                self.identity,
                &name,
                self.remembered,
                sessions,
            ),
            address: self.address,
            _lock: self.lock,
        }
    }

    /// The address the listener is bound to: with port 0 asked for, the port
    /// the system picked.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
}

/// The session the relay runs in, from its `environment` (U3): none
/// outside a session. A relay refuses to start in a nested session, so a
/// hub monitor can never reach the relay's own screen through a parent
/// session's.
fn enclosing(environment: &[(OsString, OsString)]) -> io::Result<Option<SessionId>> {
    match enclosing_depth(environment) {
        None => Ok(None),
        Some(1) => Ok(own_session(environment)),
        Some(depth) => Err(io::Error::other(format!(
            "c2 relay cannot start in a nested session (this one is at depth {depth})"
        ))),
    }
}

/// The ID the environment names as its session's, if it is one.
fn own_session(environment: &[(OsString, OsString)]) -> Option<SessionId> {
    let (_, id) = environment
        .iter()
        .rev()
        .find(|(name, _)| name == "CLIMON2_SESSION_ID")?;
    id.to_str()?.parse().ok()
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
    home_lock::take(&links.join(LOCK)).and_then(held)
}

/// The relay's identity, made on its first start (ST5), and the hub
/// monitors it remembers. A damaged store refuses the start.
fn keys(home: &HomeLayout) -> io::Result<(StaticKey, Remembered)> {
    let files = KeyStore::open(&home.links())?;
    let identity = pairing::relay_identity_at_start(&files, new_identity)?;
    let remembered = pairing::load_hub_monitors(&files)?;
    let identity = StaticKey::from_private(*identity.private_key.expose());
    Ok((identity, Remembered::new(files, remembered)))
}

/// A new identity keypair, from the operating system's generator.
fn new_identity() -> io::Result<Keypair> {
    let mut private = [0; 32];
    randomness::fill(&mut private)?;
    let public = StaticKey::from_private(private).public();
    Ok(Keypair {
        public_key: StoredKey::new(*public.as_bytes()),
        private_key: PrivateKey::new(private),
    })
}

fn held(taken: Taken) -> io::Result<Held> {
    match taken {
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
        let startup = Startup::in_home(any_port(), home.path(), &[]).unwrap();
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
        let first = Startup::in_home(any_port(), home.path(), &[]).unwrap();
        assert_eq!(
            refusal(Startup::in_home(any_port(), home.path(), &[])),
            format!(
                "a relay is already running in this climon2 home (pid {}, listening on {})",
                std::process::id(),
                first.address()
            )
        );
        let elsewhere = self::home();
        Startup::in_home(any_port(), elsewhere.path(), &[])
            .expect("another home runs its own relay");
        drop(first);
        Startup::in_home(any_port(), home.path(), &[]).expect("the home is free again");
    }

    #[test]
    fn the_lock_is_an_owner_only_file_in_an_owner_only_links_folder() {
        let home = home();
        let _startup = Startup::in_home(any_port(), home.path(), &[]).unwrap();
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
        let error = Startup::in_home(RelayArgs { listen }, home.path(), &[])
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("cannot listen on {address}: ")),
            "{text}"
        );
        Startup::in_home(any_port(), home.path(), &[]).expect("the failed start left no lock");
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
        let startup = Startup::in_home(RelayArgs { listen: None }, home.path(), &[]).unwrap();
        assert_eq!(startup.address(), configured);
        drop(startup);
        let flagged = free(());
        let listen = Some(flagged);
        let startup = Startup::in_home(RelayArgs { listen }, home.path(), &[]).unwrap();
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
        let text = refusal(Startup::in_home(any_port(), home.path(), &[]));
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

    fn identity(home: &Path) -> Keypair {
        let files = KeyStore::open(&home.join("links")).unwrap();
        pairing::relay_identity_at_start(&files, || panic!("an identity exists")).unwrap()
    }

    /// ST5, P16: the first start makes the relay's one identity, and every
    /// later start keeps it.
    #[test]
    fn the_first_start_makes_the_identity_and_later_starts_keep_it() {
        let home = home();
        drop(Startup::in_home(any_port(), home.path(), &[]).unwrap());
        let first = identity(home.path());
        let public = StaticKey::from_private(*first.private_key.expose()).public();
        assert_eq!(
            public.as_bytes(),
            first.public_key.as_bytes(),
            "a whole keypair"
        );
        let startup = Startup::in_home(any_port(), home.path(), &[]).unwrap();
        assert_eq!(
            startup.identity.public().as_bytes(),
            first.public_key.as_bytes()
        );
        drop(startup);
        assert_eq!(identity(home.path()).public_key, first.public_key);
        let elsewhere = self::home();
        drop(Startup::in_home(any_port(), elsewhere.path(), &[]).unwrap());
        assert_ne!(identity(elsewhere.path()).public_key, first.public_key);
    }

    /// ST5, STO-02: a damaged store refuses the start and is never
    /// replaced, and the refused start leaves the home free.
    #[test]
    fn a_damaged_store_refuses_the_start() {
        for file in ["relay-identity.json", "hub-monitors.json"] {
            let home = home();
            drop(Startup::in_home(any_port(), home.path(), &[]).unwrap());
            let path = home.path().join("links").join(file);
            fs::write(&path, "{").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let text = refusal(Startup::in_home(any_port(), home.path(), &[]));
            assert!(
                text.starts_with(&format!("damaged pairing file {file}")),
                "{text}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), "{");
            fs::remove_file(&path).unwrap();
            Startup::in_home(any_port(), home.path(), &[]).expect("the refused start left no lock");
        }
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

    fn environment(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    const ID: &str = "0d5c0000-0000-4000-8000-000000000001";

    /// ADV-03 (U3): outside a session the relay serves every session; in
    /// one at depth 1 it knows its own; in a nested one it refuses to
    /// start, before the home is touched.
    #[test]
    fn adv_03_a_relay_knows_its_own_session_and_refuses_a_nested_one() {
        let home = home();
        let outside = Startup::in_home(any_port(), home.path(), &[]).unwrap();
        assert_eq!(outside.own, None);
        drop(outside);
        let depth_1 = environment(&[("CLIMON2_SESSION_ID", ID)]);
        let inside = Startup::in_home(any_port(), home.path(), &depth_1).unwrap();
        assert_eq!(inside.own, Some(ID.parse().unwrap()));
        drop(inside);
        let stated = environment(&[("CLIMON2_SESSION_ID", ID), ("CLIMON2_SESSION_DEPTH", "1")]);
        let inside = Startup::in_home(any_port(), home.path(), &stated).unwrap();
        assert_eq!(inside.own, Some(ID.parse().unwrap()));
        drop(inside);

        let elsewhere = tempfile::tempdir().unwrap();
        let nested = environment(&[("CLIMON2_SESSION_ID", ID), ("CLIMON2_SESSION_DEPTH", "2")]);
        assert_eq!(
            refusal(Startup::in_home(any_port(), elsewhere.path(), &nested)),
            "c2 relay cannot start in a nested session (this one is at depth 2)"
        );
        assert!(!elsewhere.path().join("links").exists());
    }

    /// A session ID that is not one names no session to leave out.
    #[test]
    fn an_unreadable_session_id_names_no_own_session() {
        let home = home();
        let unreadable = environment(&[("CLIMON2_SESSION_ID", "not-an-id")]);
        let startup = Startup::in_home(any_port(), home.path(), &unreadable).unwrap();
        assert_eq!(startup.own, None);
    }
}
