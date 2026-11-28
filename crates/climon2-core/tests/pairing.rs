//! Spec #488 Storage (STO-01 and STO-02, Contract): the pairing store each
//! side of a monitor link keeps in `links/`. Every expected file is written
//! here from the crate README's description of the three files, never taken
//! from the code under test. The key files are held in memory: the real
//! folder is the platform's key storage seam, proven by its own suite.
use climon2_core::pairing::{
    self, HUB_MONITORS_FILE, HubMonitor, KeyFiles, Keypair, MAX_PAIRINGS, Pairings, PrivateKey,
    PublicKey, RELAY_IDENTITY_FILE, RELAYS_FILE, Relay, RelayPairings, StoreFull,
};
use std::{cell::RefCell, collections::BTreeMap, io};

/// Key files in memory. `broken` makes every read and write fail, as an
/// unreadable folder would.
#[derive(Default)]
struct Files {
    held: RefCell<BTreeMap<String, Vec<u8>>>,
    broken: bool,
}

impl Files {
    fn holding(name: &str, contents: &str) -> Self {
        let files = Self::default();
        files.put(name, contents);
        files
    }

    fn broken() -> Self {
        Self {
            broken: true,
            ..Self::default()
        }
    }

    fn put(&self, name: &str, contents: &str) {
        self.held
            .borrow_mut()
            .insert(name.into(), contents.as_bytes().to_vec());
    }

    fn text(&self, name: &str) -> Option<String> {
        self.held
            .borrow()
            .get(name)
            .map(|bytes| String::from_utf8(bytes.clone()).unwrap())
    }

    fn names(&self) -> Vec<String> {
        self.held.borrow().keys().cloned().collect()
    }

    fn failure(&self) -> io::Result<()> {
        if self.broken {
            return Err(io::Error::other("the folder cannot be used"));
        }
        Ok(())
    }
}

impl KeyFiles for Files {
    fn load(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.failure()?;
        Ok(self.held.borrow().get(name).cloned())
    }

    fn store(&self, name: &str, contents: &[u8]) -> io::Result<()> {
        self.failure()?;
        self.held
            .borrow_mut()
            .insert(name.into(), contents.to_vec());
        Ok(())
    }
}

const KEY_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const KEY_B: &str = "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff00112233445566778899aabbccddeeff";
const KEY_C: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f";

fn bytes_a() -> [u8; 32] {
    std::array::from_fn(|index| index as u8)
}

fn bytes_b() -> [u8; 32] {
    [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe,
        0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
        0xee, 0xff,
    ]
}

fn bytes_c() -> [u8; 32] {
    let mut bytes = [0xff; 32];
    bytes[31] = 0x7f;
    bytes
}

/// A public key no other number gives.
fn numbered_key(number: usize) -> PublicKey {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&(number as u64).to_be_bytes());
    PublicKey::new(bytes)
}

fn hub_monitor(number: usize) -> HubMonitor {
    HubMonitor {
        public_key: numbered_key(number),
        name: format!("laptop {number}"),
        paired_at: 1_795_000_000,
        last_linked_at: None,
    }
}

fn relay(number: usize) -> Relay {
    Relay {
        name: format!("build box {number}"),
        address: "10.0.0.7:7222".into(),
        relay_public_key: numbered_key(number),
        public_key: PublicKey::new(bytes_b()),
        private_key: PrivateKey::new(bytes_c()),
        paired_at: 1_795_000_000,
        last_linked_at: Some(1_795_000_600),
        disconnected: false,
    }
}

fn full<T: pairing::Pairing>(record: impl Fn(usize) -> T) -> Pairings<T> {
    let mut pairings = Pairings::default();
    for number in 0..MAX_PAIRINGS {
        pairings.insert(record(number)).unwrap();
    }
    pairings
}

fn damage(result: io::Result<impl std::fmt::Debug>) -> String {
    let error = result.expect_err("a damaged file was read");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    error.to_string()
}

fn relays_file(records: &str) -> String {
    format!("{{\"version\":1,\"relays\":[{records}]}}\n")
}

fn relay_record(relay_key: &str) -> String {
    format!(
        "{{\"name\":\"build box\",\"address\":\"10.0.0.7:7222\",\"relay_public_key\":\"{relay_key}\",\
         \"public_key\":\"{KEY_B}\",\"private_key\":\"{KEY_C}\",\"paired_at\":1795000000,\
         \"last_linked_at\":1795000600,\"disconnected\":true}}"
    )
}

fn hub_monitors_file(records: &str) -> String {
    format!("{{\"version\":1,\"hold\":false,\"hub_monitors\":[{records}]}}\n")
}

fn hub_monitor_record(key: &str) -> String {
    format!(
        "{{\"public_key\":\"{key}\",\"name\":\"laptop\",\"paired_at\":1795000000,\
         \"last_linked_at\":null}}"
    )
}

fn identity_file() -> String {
    format!("{{\"version\":1,\"public_key\":\"{KEY_A}\",\"private_key\":\"{KEY_C}\"}}\n")
}

fn no_new_identity() -> io::Result<Keypair> {
    panic!("a new relay identity was made")
}

/// STO-01 (Contract): the hub monitor's `relays.json` holds one record per
/// relay with the specified fields, as one line of version 1 JSON.
#[test]
fn sto01_the_hub_monitor_keeps_one_record_per_relay_in_relays_json() {
    let files = Files::default();
    let mut relays = Pairings::default();
    relays
        .insert(Relay {
            name: "build box".into(),
            address: "10.0.0.7:7222".into(),
            relay_public_key: PublicKey::new(bytes_a()),
            public_key: PublicKey::new(bytes_b()),
            private_key: PrivateKey::new(bytes_c()),
            paired_at: 1_795_000_000,
            last_linked_at: Some(1_795_000_600),
            disconnected: true,
        })
        .unwrap();
    pairing::save_relays(&files, &relays).unwrap();
    assert_eq!(files.names(), [RELAYS_FILE]);
    assert_eq!(RELAYS_FILE, "relays.json");
    assert_eq!(
        files.text(RELAYS_FILE).unwrap(),
        relays_file(&relay_record(KEY_A))
    );
}

/// STO-01 (Contract): what was saved is what the next process reads.
#[test]
fn sto01_saved_relays_are_read_back() {
    let files = Files::holding(
        RELAYS_FILE,
        &relays_file(&format!(
            "{},{}",
            relay_record(KEY_A),
            relay_record(KEY_B)
                .replace("1795000600", "null")
                .replace("true", "false")
        )),
    );
    let relays = pairing::load_relays(&files).unwrap();
    let [first, second] = relays.records() else {
        panic!("{relays:?}")
    };
    assert_eq!(first.name, "build box");
    assert_eq!(first.address, "10.0.0.7:7222");
    assert_eq!(first.relay_public_key.as_bytes(), &bytes_a());
    assert_eq!(first.public_key.as_bytes(), &bytes_b());
    assert_eq!(first.private_key.expose(), &bytes_c());
    assert_eq!(first.paired_at, 1_795_000_000);
    assert_eq!(first.last_linked_at, Some(1_795_000_600));
    assert!(first.disconnected);
    assert_eq!(second.relay_public_key.as_bytes(), &bytes_b());
    assert_eq!(second.last_linked_at, None);
    assert!(!second.disconnected);

    let rewritten = Files::default();
    pairing::save_relays(&rewritten, &relays).unwrap();
    assert_eq!(rewritten.text(RELAYS_FILE), files.text(RELAYS_FILE));
}

/// STO-01 (Contract): the relay's `hub-monitors.json` holds each remembered
/// hub monitor and the relay's hold.
#[test]
fn sto01_the_relay_keeps_its_hub_monitors_and_its_hold_in_hub_monitors_json() {
    let files = Files::default();
    let mut remembered = RelayPairings::default();
    assert!(!remembered.hold);
    remembered
        .hub_monitors
        .insert(HubMonitor {
            public_key: PublicKey::new(bytes_a()),
            name: "laptop".into(),
            paired_at: 1_795_000_000,
            last_linked_at: None,
        })
        .unwrap();
    pairing::save_hub_monitors(&files, &remembered).unwrap();
    assert_eq!(files.names(), [HUB_MONITORS_FILE]);
    assert_eq!(HUB_MONITORS_FILE, "hub-monitors.json");
    assert_eq!(
        files.text(HUB_MONITORS_FILE).unwrap(),
        hub_monitors_file(&hub_monitor_record(KEY_A))
    );

    remembered.hold = true;
    pairing::save_hub_monitors(&files, &remembered).unwrap();
    assert_eq!(
        files.text(HUB_MONITORS_FILE).unwrap(),
        hub_monitors_file(&hub_monitor_record(KEY_A)).replace("false", "true")
    );
}

/// STO-01 (Contract): the hold and every hub monitor are read back.
#[test]
fn sto01_saved_hub_monitors_are_read_back() {
    let text = hub_monitors_file(&format!(
        "{},{}",
        hub_monitor_record(KEY_A),
        hub_monitor_record(KEY_B).replace("null", "1795000600")
    ));
    let files = Files::holding(HUB_MONITORS_FILE, &text);
    let remembered = pairing::load_hub_monitors(&files).unwrap();
    assert!(!remembered.hold);
    assert_eq!(
        remembered.hub_monitors.records(),
        [
            HubMonitor {
                public_key: PublicKey::new(bytes_a()),
                name: "laptop".into(),
                paired_at: 1_795_000_000,
                last_linked_at: None,
            },
            HubMonitor {
                public_key: PublicKey::new(bytes_b()),
                name: "laptop".into(),
                paired_at: 1_795_000_000,
                last_linked_at: Some(1_795_000_600),
            },
        ]
    );
    let held = Files::holding(HUB_MONITORS_FILE, &text.replace("false", "true"));
    assert!(pairing::load_hub_monitors(&held).unwrap().hold);
}

/// STO-01 (Contract): a side that has never paired has no file, and reading
/// that writes nothing.
#[test]
fn sto01_a_side_that_never_paired_has_no_pairings() {
    let files = Files::default();
    assert!(pairing::load_relays(&files).unwrap().records().is_empty());
    let remembered = pairing::load_hub_monitors(&files).unwrap();
    assert!(remembered.hub_monitors.records().is_empty());
    assert!(!remembered.hold);
    assert_eq!(files.names(), [""; 0]);
}

/// STO-02 (Contract): a missing identity is created on a relay's start, kept
/// in `relay-identity.json`, and never made again while the file is there.
#[test]
fn sto02_a_missing_relay_identity_is_created_on_the_relays_start() {
    let files = Files::default();
    let made = pairing::relay_identity_at_start(&files, || {
        Ok(Keypair {
            public_key: PublicKey::new(bytes_a()),
            private_key: PrivateKey::new(bytes_c()),
        })
    })
    .unwrap();
    assert_eq!(made.public_key.as_bytes(), &bytes_a());
    assert_eq!(made.private_key.expose(), &bytes_c());
    assert_eq!(RELAY_IDENTITY_FILE, "relay-identity.json");
    assert_eq!(files.names(), [RELAY_IDENTITY_FILE]);
    assert_eq!(files.text(RELAY_IDENTITY_FILE).unwrap(), identity_file());

    let kept = pairing::relay_identity_at_start(&files, no_new_identity).unwrap();
    assert_eq!(kept.public_key.as_bytes(), &bytes_a());
    assert_eq!(kept.private_key.expose(), &bytes_c());
    assert_eq!(files.text(RELAY_IDENTITY_FILE).unwrap(), identity_file());
}

/// STO-02 (Contract): nothing but a relay's start makes an identity. Reading
/// and writing the pairings on either side leaves it missing.
#[test]
fn sto02_only_a_relays_start_creates_the_identity() {
    let files = Files::default();
    pairing::load_relays(&files).unwrap();
    pairing::load_hub_monitors(&files).unwrap();
    pairing::save_relays(&files, &Pairings::default()).unwrap();
    pairing::save_hub_monitors(&files, &RelayPairings::default()).unwrap();
    assert_eq!(files.names(), [HUB_MONITORS_FILE, RELAYS_FILE]);
}

/// STO-02 (Contract): an identity that cannot be made or kept is an error,
/// and no half-made identity is left behind.
#[test]
fn sto02_an_identity_that_cannot_be_made_is_an_error() {
    let files = Files::default();
    let error = pairing::relay_identity_at_start(&files, || Err(io::Error::other("no randomness")))
        .unwrap_err();
    assert_eq!(error.to_string(), "no randomness");
    assert_eq!(files.names(), [""; 0]);
}

/// STO-02 (Contract): damage to any of the three files is an explicit error
/// naming the file, and is never read as an empty store.
#[test]
fn sto02_a_damaged_file_is_an_error_and_never_an_empty_store() {
    let two_the_same = format!("{0},{0}", relay_record(KEY_A));
    let relays = [
        "not json".to_string(),
        "[]".into(),
        "null".into(),
        "{}".into(),
        "{\"version\":1}".into(),
        "{\"relays\":[]}".into(),
        "{\"version\":1,\"relays\":[],}".into(),
        "{\"version\":1,\"relays\":{}}".into(),
        relays_file("").replace("\"relays\"", "\"relay\""),
        relays_file("").replace("]", "],\"extra\":1"),
        relays_file(&relay_record(KEY_A)).replace("\"disconnected\":true", "\"extra\":true"),
        relays_file(&relay_record(KEY_A)).replace(",\"disconnected\":true", ""),
        relays_file(&relay_record(KEY_A)).replace("1795000000", "-1"),
        relays_file(&relay_record(KEY_A)).replace("1795000000", "\"today\""),
        relays_file(&relay_record(&KEY_A[..62])),
        relays_file(&relay_record(&format!("{KEY_A}00"))),
        relays_file(&relay_record(&KEY_A.replace('a', "A"))),
        relays_file(&relay_record(&KEY_A.replace('a', "g"))),
        relays_file(&relay_record(&KEY_A.replace("0a", "+a"))),
        relays_file(&relay_record(KEY_A)).replace(KEY_C, &KEY_C[..63]),
        relays_file(&two_the_same),
    ];
    for text in &relays {
        let files = Files::holding(RELAYS_FILE, text);
        let message = damage(pairing::load_relays(&files));
        assert!(
            message.starts_with("damaged pairing file relays.json: "),
            "{text}: {message}"
        );
    }

    let two_the_same = format!("{0},{0}", hub_monitor_record(KEY_A));
    let hub_monitors = [
        "\u{0}".to_string(),
        "[]".into(),
        "{\"version\":1,\"hub_monitors\":[]}".into(),
        "{\"version\":1,\"hold\":false}".into(),
        hub_monitors_file("").replace("false", "0"),
        hub_monitors_file("").replace("]", "],\"extra\":1"),
        hub_monitors_file(&hub_monitor_record(KEY_A)).replace("\"name\":\"laptop\",", ""),
        hub_monitors_file(&hub_monitor_record(&KEY_A[..60])),
        hub_monitors_file(&two_the_same),
    ];
    for text in &hub_monitors {
        let files = Files::holding(HUB_MONITORS_FILE, text);
        let message = damage(pairing::load_hub_monitors(&files));
        assert!(
            message.starts_with("damaged pairing file hub-monitors.json: "),
            "{text}: {message}"
        );
    }

    let identities = [
        "{".to_string(),
        "[]".into(),
        "{\"version\":1}".into(),
        identity_file().replace(KEY_A, ""),
        identity_file().replace(KEY_C, &KEY_C[1..]),
        identity_file().replace("}", ",\"extra\":1}"),
        identity_file().replace(",\"private_key\"", ",\"private\""),
    ];
    for text in &identities {
        let files = Files::holding(RELAY_IDENTITY_FILE, text);
        let message = damage(pairing::relay_identity_at_start(&files, no_new_identity));
        assert!(
            message.starts_with("damaged pairing file relay-identity.json: "),
            "{text}: {message}"
        );
        // A damaged identity is never replaced by a new one.
        assert_eq!(files.text(RELAY_IDENTITY_FILE).as_deref(), Some(&**text));
    }
}

/// STO-02 (Contract): a file a later release wrote is refused, never read as
/// far as this release understands it, and the error says which version.
#[test]
fn sto02_a_file_of_another_version_is_refused() {
    for version in ["0", "2", "4294967295"] {
        let reason = format!("version {version} is not one this release reads");
        let files = Files::holding(
            RELAYS_FILE,
            &relays_file("").replace("\"version\":1", &format!("\"version\":{version}")),
        );
        assert_eq!(
            damage(pairing::load_relays(&files)),
            format!("damaged pairing file relays.json: {reason}")
        );
        let files = Files::holding(
            HUB_MONITORS_FILE,
            &hub_monitors_file("").replace("\"version\":1", &format!("\"version\":{version}")),
        );
        assert_eq!(
            damage(pairing::load_hub_monitors(&files)),
            format!("damaged pairing file hub-monitors.json: {reason}")
        );
        let files = Files::holding(
            RELAY_IDENTITY_FILE,
            &identity_file().replace("\"version\":1", &format!("\"version\":{version}")),
        );
        assert_eq!(
            damage(pairing::relay_identity_at_start(&files, no_new_identity)),
            format!("damaged pairing file relay-identity.json: {reason}")
        );
    }
}

/// STO-02 (Contract): the error says where a file is malformed and what rule
/// its records break, so the user can see why.
#[test]
fn sto02_the_error_says_why_the_file_is_damaged() {
    let files = Files::holding(RELAYS_FILE, "{\"version\":1,\n \"relays\":7}");
    assert_eq!(
        damage(pairing::load_relays(&files)),
        "damaged pairing file relays.json: malformed at line 2 column 11"
    );
    let two_the_same = format!("{0},{0}", relay_record(KEY_A));
    let files = Files::holding(RELAYS_FILE, &relays_file(&two_the_same));
    assert_eq!(
        damage(pairing::load_relays(&files)),
        "damaged pairing file relays.json: two pairings share a public key"
    );
}

/// The blanket check on secrets: no error and no debug text carries a key's
/// bytes, even when the damage is a key in the wrong place.
#[test]
fn sto02_errors_and_debug_text_never_carry_a_private_key() {
    let misplaced =
        relays_file(&relay_record(KEY_A)).replace("1795000000", &format!("\"{KEY_C}\""));
    let message = damage(pairing::load_relays(&Files::holding(
        RELAYS_FILE,
        &misplaced,
    )));
    assert!(!message.contains(KEY_C), "{message}");
    let short = relays_file(&relay_record(KEY_A)).replace(KEY_C, &KEY_C[..62]);
    let message = damage(pairing::load_relays(&Files::holding(RELAYS_FILE, &short)));
    assert!(!message.contains(&KEY_C[..62]), "{message}");

    let relay = relay(0);
    let shown = format!("{relay:?} {:?}", relay.private_key);
    assert!(shown.contains("PrivateKey(withheld)"), "{shown}");
    // The private key is 31 bytes of 255 and one of 127.
    assert!(!shown.contains("255, 255"), "{shown}");
    assert!(!shown.contains("127"), "{shown}");
    assert!(!shown.contains("ffff"), "{shown}");
}

/// STO-02 (Contract): key files that cannot be read or written are an error
/// on every path, never an empty store or a new identity.
#[test]
fn sto02_key_files_that_cannot_be_used_are_an_error() {
    let files = Files::broken();
    let unusable = "the folder cannot be used";
    assert_eq!(
        pairing::load_relays(&files).unwrap_err().to_string(),
        unusable
    );
    assert_eq!(
        pairing::load_hub_monitors(&files).unwrap_err().to_string(),
        unusable
    );
    assert_eq!(
        pairing::relay_identity_at_start(&files, no_new_identity)
            .unwrap_err()
            .to_string(),
        unusable
    );
    assert_eq!(
        pairing::save_relays(&files, &Pairings::default())
            .unwrap_err()
            .to_string(),
        unusable
    );
    assert_eq!(
        pairing::save_hub_monitors(&files, &RelayPairings::default())
            .unwrap_err()
            .to_string(),
        unusable
    );
}

/// Spec #488 user story 38: each side keeps at most 64 pairings and refuses
/// the next explicitly, leaving the store as it was.
#[test]
fn at_most_64_pairings_are_kept_per_side() {
    assert_eq!(MAX_PAIRINGS, 64);
    let mut relays = full(relay);
    assert_eq!(relays.records().len(), 64);
    assert_eq!(relays.insert(relay(64)).unwrap_err(), StoreFull);
    assert_eq!(relays.records().len(), 64);
    assert!(
        relays
            .records()
            .iter()
            .all(|kept| kept.relay_public_key != numbered_key(64))
    );
    assert_eq!(
        StoreFull.to_string(),
        "64 pairings are already kept, which is the most a side keeps"
    );

    let mut hub_monitors = full(hub_monitor);
    assert_eq!(hub_monitors.insert(hub_monitor(64)).unwrap_err(), StoreFull);
    assert_eq!(hub_monitors.records().len(), 64);
}

/// A full store on disk is read; one record more is damage.
#[test]
fn a_file_holds_at_most_64_pairings() {
    let files = Files::default();
    pairing::save_relays(&files, &full(relay)).unwrap();
    pairing::save_hub_monitors(
        &files,
        &RelayPairings {
            hold: false,
            hub_monitors: full(hub_monitor),
        },
    )
    .unwrap();
    assert_eq!(pairing::load_relays(&files).unwrap().records().len(), 64);
    let remembered = pairing::load_hub_monitors(&files).unwrap();
    assert_eq!(remembered.hub_monitors.records().len(), 64);

    let one_more = files
        .text(RELAYS_FILE)
        .unwrap()
        .replace("]}", &format!(",{}]}}", relay_record(KEY_B)));
    files.put(RELAYS_FILE, &one_more);
    assert_eq!(
        damage(pairing::load_relays(&files)),
        "damaged pairing file relays.json: more than 64 pairings"
    );
    let one_more = files
        .text(HUB_MONITORS_FILE)
        .unwrap()
        .replace("]}", &format!(",{}]}}", hub_monitor_record(KEY_B)));
    files.put(HUB_MONITORS_FILE, &one_more);
    assert_eq!(
        damage(pairing::load_hub_monitors(&files)),
        "damaged pairing file hub-monitors.json: more than 64 pairings"
    );
}

/// Pairing again with the same peer replaces its record where it stands, and
/// needs no room: a pairing is known by the other side's public key.
#[test]
fn a_record_for_the_same_peer_replaces_the_old_one() {
    let mut hub_monitors = full(hub_monitor);
    let renamed = HubMonitor {
        name: "desk".into(),
        last_linked_at: Some(9),
        ..hub_monitor(5)
    };
    hub_monitors.insert(renamed.clone()).unwrap();
    assert_eq!(hub_monitors.records().len(), 64);
    assert_eq!(hub_monitors.records()[5], renamed);
    assert_eq!(hub_monitors.records()[4], hub_monitor(4));
    assert_eq!(hub_monitors.records()[6], hub_monitor(6));

    let mut relays = Pairings::default();
    relays.insert(relay(1)).unwrap();
    relays.insert(relay(2)).unwrap();
    relays
        .insert(Relay {
            address: "example.test:9".into(),
            ..relay(1)
        })
        .unwrap();
    let addresses: Vec<_> = relays
        .records()
        .iter()
        .map(|relay| relay.address.as_str())
        .collect();
    assert_eq!(addresses, ["example.test:9", "10.0.0.7:7222"]);
}

/// Forgetting takes one record out by the other side's public key, hands it
/// back, and frees its place.
#[test]
fn a_forgotten_pairing_is_removed_and_frees_its_place() {
    let mut hub_monitors = full(hub_monitor);
    assert_eq!(hub_monitors.remove(&numbered_key(64)), None);
    assert_eq!(hub_monitors.records().len(), 64);
    assert_eq!(hub_monitors.remove(&numbered_key(7)), Some(hub_monitor(7)));
    assert_eq!(hub_monitors.records().len(), 63);
    assert_eq!(hub_monitors.remove(&numbered_key(7)), None);
    assert_eq!(hub_monitors.records()[7], hub_monitor(8));
    hub_monitors.insert(hub_monitor(64)).unwrap();
    assert_eq!(hub_monitors.records()[63], hub_monitor(64));

    let mut relays = Pairings::default();
    relays.insert(relay(1)).unwrap();
    relays.insert(relay(2)).unwrap();
    let forgotten = relays.remove(&numbered_key(1)).unwrap();
    assert_eq!(forgotten.name, "build box 1");
    let [kept] = relays.records() else {
        panic!("{relays:?}")
    };
    assert_eq!(kept.name, "build box 2");
}
