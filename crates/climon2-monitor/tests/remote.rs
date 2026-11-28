//! #502 (ADV-01, ADV-04, ADV-05, SEL-01): the hub monitor's side of
//! advertisement, through the monitor library's own interface. Relays made
//! here from the link code listen on loopback and advertise what each test
//! scripts; the monitor dials the relays its home's `links/relays.json`
//! remembers and lists their sessions beside its local ones.
#![cfg(unix)]

use climon2_core::{
    SessionId,
    config::MonitorConfig,
    layout::{ExecutableHome, HomeLayout},
    metadata::SessionMetadata,
    pairing::{self, Pairings, PrivateKey, PublicKey as StoredKey, Relay},
    protocol::TerminalSize,
};
use climon2_link::{
    Advertisement, Carrier, Ended, Hello, HubClose, Introduction, Invitations, Link, Next,
    PublicKey, Randomness, RelayEnd, RelayLink, RelayRecord, Revisions, Session, StaticKey, Time,
    drive,
};
use climon2_monitor::{
    Action, Canvas, Choice, Consumer, Event, Inputs, LOCAL, Monitor, SessionAt, System as Clock,
    load, toast::Toast,
};
use climon2_platform::{
    clock,
    key_storage::KeyStore,
    link_socket::{self, LinkListener, LinkStream},
    randomness,
};
use std::{
    fs, io,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(20);
const LARGEST_RECORD: u32 = 1_048_576;
/// The redial wait the monitors here are configured with.
const CONFIG: &[u8] = br#"{"monitor": {"limits": {"retry": {"initial_ms": 200}}}}"#;

struct System;

impl Randomness for System {
    fn fill(&mut self, bytes: &mut [u8]) {
        randomness::fill(bytes).unwrap();
    }
}

struct Wire(LinkStream);

impl Carrier for Wire {
    fn now(&mut self) -> Time {
        Time::since_start(clock::monotonic())
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        let tick = Duration::from_millis(20);
        self.0
            .read(buffer, wait.map_or(tick, |wait| wait.min(tick)))
    }
}

fn key() -> StaticKey {
    let mut private = [0; 32];
    randomness::fill(&mut private).unwrap();
    StaticKey::from_private(private)
}

fn id(last: u8) -> SessionId {
    format!("0d5c0000-0000-4000-8000-0000000000{last:02x}")
        .parse()
        .unwrap()
}

/// A session a relay advertises: `id(last)` running `command`.
fn session(last: u8, command: &str) -> Advertisement {
    Advertisement::Session(Session {
        id: id(last).bytes(),
        encoding: 1,
        command: command.as_bytes().to_vec(),
        arguments: Vec::new(),
        working_directory: b"/home/sam".to_vec(),
        depth: 1,
        created_at: *b"2026-11-27T09:30:00.000Z",
        release: "9.9.9".to_owned(),
    })
}

/// What a test tells one of its relay's link connections.
enum Script {
    /// Advertise this.
    Say(Advertisement),
    /// End the connection by closing its byte stream.
    Drop,
}

/// A relay made from the link code that serves link connections from one
/// hub monitor key, one after another, each advertising what the test
/// scripts once it is established.
struct TestRelay {
    identity: StaticKey,
    listener: Arc<LinkListener>,
    address: SocketAddr,
}

/// One link connection a test relay accepted, which the test scripts.
struct Served {
    script: Sender<Script>,
    ended: JoinHandle<RelayLink>,
}

impl TestRelay {
    fn new() -> Self {
        let listener = link_socket::listen("127.0.0.1:0".parse().unwrap()).unwrap();
        Self {
            identity: key(),
            address: listener.local_address().unwrap(),
            listener: Arc::new(listener),
        }
    }

    /// On a fresh port, with the same identity: the relay restarted
    /// elsewhere.
    fn moved(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            ..Self::new()
        }
    }

    /// Accepts the next link connection from `hub` and holds it.
    fn serve(&self, hub: PublicKey) -> Served {
        let listener = Arc::clone(&self.listener);
        let identity = self.identity.clone();
        let (script, scripted) = mpsc::channel();
        let ended = thread::spawn(move || {
            let stream = listener.accept(WAIT).unwrap();
            let introduction = Introduction::new(*identity.public().as_bytes(), "relay").unwrap();
            let hello = Hello::new("9.9.9", "relay", LARGEST_RECORD).unwrap();
            let mut end = RelayEnd::new(Revisions::published(), Invitations::new(introduction))
                .linking(identity, vec![hub], hello);
            let between = |end: &mut RelayEnd| Ok(follow(end, &scripted));
            drive(&mut end, &mut Wire(stream), &mut System, between).unwrap();
            end.link().clone()
        });
        Served { script, ended }
    }

    /// Whether a hub monitor connects within `wait`.
    fn dialled_within(&self, wait: Duration) -> bool {
        self.listener.accept(wait).is_ok()
    }
}

/// Between two steps of a test relay's link: once it is established, what
/// the test scripted is sent, or the connection dropped.
fn follow(end: &mut RelayEnd, scripted: &Receiver<Script>) -> Next {
    if !matches!(end.link(), Link::Established(_)) {
        return Next::Wait;
    }
    match scripted.try_recv() {
        Ok(Script::Say(advertisement)) => {
            end.send(&RelayRecord::Advertisement(advertisement))
                .unwrap();
            Next::Step
        }
        Ok(Script::Drop) => Next::Stop,
        Err(_) => Next::Wait,
    }
}

impl Served {
    fn say(&self, advertisement: Advertisement) {
        self.script.send(Script::Say(advertisement)).unwrap();
    }

    /// Drops the connection and waits until its thread has.
    fn drop_link(self) {
        self.script.send(Script::Drop).unwrap();
        self.ended.join().unwrap();
    }

    /// How the hub monitor ended the link.
    fn ended(self) -> RelayLink {
        self.ended.join().unwrap()
    }
}

/// A hub monitor's record of `relay`, named `name`, at its address, with
/// a key of its own for the pairing.
fn record(relay: &TestRelay, name: &str) -> (Relay, StaticKey) {
    let mut private = [0; 32];
    randomness::fill(&mut private).unwrap();
    let own = StaticKey::from_private(private);
    let relay = Relay {
        name: name.to_owned(),
        address: relay.address.to_string(),
        relay_public_key: StoredKey::new(*relay.identity.public().as_bytes()),
        public_key: StoredKey::new(*own.public().as_bytes()),
        private_key: PrivateKey::new(private),
        paired_at: 1,
        last_linked_at: Some(1),
        disconnected: false,
    };
    (relay, own)
}

/// Writes `relays` as the relays `home` remembers.
fn remember(home: &Path, relays: &[Relay]) {
    let store = KeyStore::open(&HomeLayout::at(home).unwrap().links()).unwrap();
    let mut pairings = Pairings::default();
    for relay in relays {
        pairings.insert(relay.clone()).unwrap();
    }
    pairing::save_relays(&store, &pairings).unwrap();
}

/// A consumer with room for 80 by 24 that covers nothing.
struct Fake;

impl Consumer for Fake {
    fn covered(&self) -> bool {
        false
    }

    fn focused(&self) -> bool {
        true
    }

    fn size(&self) -> Option<TerminalSize> {
        Some(TerminalSize { cols: 80, rows: 24 })
    }

    fn fits(&self, _: &Toast) -> bool {
        true
    }

    fn obscures(&self, _: &Toast, _: Option<(u16, u16)>) -> bool {
        false
    }
}

fn home() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    (directory, root)
}

fn started(root: &Path) -> Monitor {
    let home = HomeLayout::at(root).unwrap();
    let executables = root.join("executables");
    fs::create_dir_all(&executables).unwrap();
    let executable = ExecutableHome::new(&executables).unwrap();
    let log = climon2_platform::logging::LogFile::monitor(root).unwrap();
    Monitor::start(Inputs {
        home,
        load: load::loader(executable),
        config: MonitorConfig::parse(CONFIG).unwrap(),
        own: None,
        filter: "info".to_owned(),
        dispatch: log.dispatch(Some("info")).unwrap(),
    })
    .unwrap()
}

/// Steps `monitor` until its rows, as `status|machine|label`, are `rows`,
/// in any order, at most 10 s. Rows waiting for a client order by when
/// each entered its condition, which these tests do not pin.
fn until_rows(monitor: &mut Monitor, rows: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut rows = rows.to_vec();
    rows.sort_unstable();
    loop {
        monitor.step(&Clock, &Fake);
        let mut shown: Vec<_> = monitor.rows().into_iter().map(shown).collect();
        shown.sort();
        if shown == rows {
            return;
        }
        assert!(Instant::now() < deadline, "the rows stayed {shown:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn shown(row: climon2_monitor::View) -> String {
    let machine = row.machine.unwrap_or_else(|| "-".to_owned());
    format!("{}|{machine}|{}", row.status, row.label)
}

/// Local session metadata that discovery cannot judge, so its row is
/// Unavailable and no client starts.
fn uncertain(home: &Path, id: &SessionId, command: &str) {
    let sessions = home.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let socket = home.join("absent.sock");
    let metadata: SessionMetadata = serde_json::from_value(serde_json::json!({
        "format_version": 1, "session_id": id.as_str(), "socket_path": socket,
        "daemon_pid": 0, "daemon_start_time": 0, "command": command, "args": [],
        "cwd": home, "depth": 1, "created_at": "2026-11-28T10:00:00.000Z",
        "climon2_version": "0.1.0"
    }))
    .unwrap();
    fs::write(
        sessions.join(format!("{id}.pty")),
        metadata.to_json().unwrap(),
    )
    .unwrap();
}

/// ADV-05 and SEL-01 through the library: a local session and sessions on
/// two relays share one session ID and are three rows, each with its
/// machine, `local` or the relay's name. A remote row is selected without
/// acquiring, offers no Delete and shows no content until its session
/// stream exists (#503).
#[test]
fn adv_05_remote_sessions_are_listed_beside_local_ones_by_machine() {
    let (_directory, root) = home();
    uncertain(&root, &id(1), "top");
    let (first, second) = (TestRelay::new(), TestRelay::new());
    let (first_record, first_own) = record(&first, "buildbox");
    let (second_record, second_own) = record(&second, "laptop");
    remember(&root, &[first_record, second_record]);
    let on_first = first.serve(first_own.public());
    let on_second = second.serve(second_own.public());
    let mut monitor = started(&root);
    on_first.say(session(1, "vim"));
    on_first.say(Advertisement::SnapshotComplete);
    on_second.say(session(1, "htop"));
    until_rows(
        &mut monitor,
        &[
            "Unavailable|local|top",
            "Unavailable|laptop|htop",
            "Unavailable|buildbox|vim",
        ],
    );
    let mut ids: Vec<_> = monitor.rows().into_iter().map(|row| row.id).collect();
    ids.sort();
    let remote = SessionAt::remote(first.identity.public(), id(1));
    let mut expected = [
        SessionAt::local(id(1)),
        SessionAt::remote(second.identity.public(), id(1)),
        remote.clone(),
    ];
    expected.sort();
    assert_eq!(ids, expected);
    let select = Action::Choose(Choice::Select(remote.clone()));
    assert_eq!(
        monitor.act(select, Instant::now(), &Fake),
        [Event::Selected]
    );
    assert_eq!(monitor.selected(), Some(&remote));
    let rows = monitor.rows();
    let selected = monitor.session(&rows).unwrap();
    assert!(matches!(
        selected.canvas,
        Canvas::NoContent("Unavailable", None)
    ));
    assert_eq!(monitor.offer(&remote), None);
    assert!(monitor.offer(&SessionAt::local(id(1))).is_some());
    assert_eq!(LOCAL, "local");

    monitor.shut_down();
    let stopped = Link::Ended(Ended::Closed(HubClose::Stopped));
    assert_eq!(
        on_first.ended(),
        stopped,
        "a stopping monitor closes its links"
    );
    assert_eq!(on_second.ended(), stopped);
}

/// ADV-01 and ADV-04 through the library: a withdrawal drops a row that
/// was never connected; a reconnect, here to the address the store gives
/// now, discards what the relay advertised before and takes its fresh
/// snapshot, and a remote session keeps its identity across it (ADV-05).
/// A relay disconnected in the store meanwhile is not dialled again.
#[test]
fn adv_04_a_reconnect_discards_what_the_relay_held_and_follows_its_address() {
    let (_directory, root) = home();
    let relay = TestRelay::new();
    let (mut stored, own) = record(&relay, "buildbox");
    remember(&root, std::slice::from_ref(&stored));
    let served = relay.serve(own.public());
    let mut monitor = started(&root);
    served.say(session(1, "vim"));
    served.say(session(2, "top"));
    served.say(session(3, "htop"));
    served.say(Advertisement::SnapshotComplete);
    until_rows(
        &mut monitor,
        &[
            "Unavailable|buildbox|vim",
            "Unavailable|buildbox|top",
            "Unavailable|buildbox|htop",
        ],
    );
    served.say(Advertisement::Withdrawn(id(3).bytes()));
    until_rows(
        &mut monitor,
        &["Unavailable|buildbox|vim", "Unavailable|buildbox|top"],
    );
    let before = SessionAt::remote(relay.identity.public(), id(2));
    assert!(monitor.rows().iter().any(|row| row.id == before));

    let moved = relay.moved();
    stored.address = moved.address.to_string();
    remember(&root, std::slice::from_ref(&stored));
    let again = moved.serve(own.public());
    served.drop_link();
    again.say(session(2, "top"));
    again.say(Advertisement::SnapshotComplete);
    until_rows(&mut monitor, &["Unavailable|buildbox|top"]);
    assert_eq!(monitor.rows()[0].id, before, "the same remote session");

    stored.disconnected = true;
    remember(&root, std::slice::from_ref(&stored));
    again.drop_link();
    assert!(!moved.dialled_within(Duration::from_secs(1)));
    monitor.shut_down();
}

/// A monitor whose home has no `links/` folder remembers no relay: it has
/// no machine column and makes no folder.
#[test]
fn no_store_means_no_machine_column_and_no_links_folder() {
    let (_directory, root) = home();
    uncertain(&root, &id(1), "top");
    let mut monitor = started(&root);
    until_rows(&mut monitor, &["Unavailable|-|top"]);
    assert!(!root.join("links").exists());
    monitor.shut_down();
}

/// A relay the user disconnected is never dialled, though the monitor
/// still remembers it and so shows a machine column.
#[test]
fn a_disconnected_relay_is_remembered_but_not_dialled() {
    let (_directory, root) = home();
    uncertain(&root, &id(1), "top");
    let relay = TestRelay::new();
    let (mut stored, _) = record(&relay, "buildbox");
    stored.disconnected = true;
    remember(&root, &[stored]);
    let mut monitor = started(&root);
    until_rows(&mut monitor, &["Unavailable|local|top"]);
    assert!(!relay.dialled_within(Duration::from_secs(1)));
    monitor.shut_down();
}
