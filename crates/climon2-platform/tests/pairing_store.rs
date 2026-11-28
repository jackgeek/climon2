//! Spec #488 Storage (STO-01 and STO-02, Contract): the pairing store on
//! real files, through the key storage seam. The records and their rules
//! are proven in `climon2-core`; this proves what only a real `links/`
//! folder shows: the files are plain owner-only files, a file the seam
//! distrusts is damage to the store, and a killed writer's leftover is
//! never read as a record.
#![cfg(unix)]

use climon2_core::pairing::{
    self, HUB_MONITORS_FILE, HubMonitor, Keypair, Pairings, PrivateKey, PublicKey,
    RELAY_IDENTITY_FILE, RELAYS_FILE, Relay, RelayPairings,
};
use climon2_platform::key_storage::KeyStore;
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

/// A links folder inside a fresh climon2 home, with the process creating
/// files as permissively as a umask allows.
fn links() -> (tempfile::TempDir, PathBuf, KeyStore) {
    // SAFETY: umask only swaps the process's file mode mask.
    unsafe { libc::umask(0) };
    let home = tempfile::tempdir().expect("a climon2 home");
    let folder = home.path().join("links");
    let store = KeyStore::open(&folder).expect("the links folder opens");
    (home, folder, store)
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("the path exists")
        .permissions()
        .mode()
        & 0o7777
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder lists")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("the file reads")).expect("the file is JSON")
}

fn identity() -> io::Result<Keypair> {
    Ok(Keypair {
        public_key: PublicKey::new([1; 32]),
        private_key: PrivateKey::new([2; 32]),
    })
}

fn no_new_identity() -> io::Result<Keypair> {
    panic!("a new relay identity was made")
}

fn hub_monitor() -> HubMonitor {
    HubMonitor {
        public_key: PublicKey::new([3; 32]),
        name: "laptop".into(),
        paired_at: 1_795_000_000,
        last_linked_at: Some(1_795_000_600),
    }
}

fn relay() -> Relay {
    Relay {
        name: "build box".into(),
        address: "10.0.0.7:7222".into(),
        relay_public_key: PublicKey::new([1; 32]),
        public_key: PublicKey::new([3; 32]),
        private_key: PrivateKey::new([4; 32]),
        paired_at: 1_795_000_000,
        last_linked_at: None,
        disconnected: true,
    }
}

/// Both sides' files in one folder, as a home that runs a relay and a hub
/// monitor holds them.
fn paired(store: &KeyStore) {
    pairing::relay_identity_at_start(store, identity).unwrap();
    let mut remembered = RelayPairings {
        hold: true,
        ..RelayPairings::default()
    };
    remembered.hub_monitors.insert(hub_monitor()).unwrap();
    pairing::save_hub_monitors(store, &remembered).unwrap();
    let mut relays = Pairings::default();
    relays.insert(relay()).unwrap();
    pairing::save_relays(store, &relays).unwrap();
}

fn damage(result: io::Result<impl std::fmt::Debug>) -> String {
    let error = result.expect_err("a damaged file was read");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    error.to_string()
}

/// What each of the three files reads as, in the order identity, hub
/// monitors, relays.
fn every_read(store: &KeyStore) -> [io::Result<String>; 3] {
    [
        pairing::relay_identity_at_start(store, no_new_identity).map(|read| format!("{read:?}")),
        pairing::load_hub_monitors(store).map(|read| format!("{read:?}")),
        pairing::load_relays(store).map(|read| format!("{read:?}")),
    ]
}

const FILES: [&str; 3] = [RELAY_IDENTITY_FILE, HUB_MONITORS_FILE, RELAYS_FILE];

/// STO-01 (Contract): after pairing, `links/` holds `relay-identity.json`,
/// `hub-monitors.json` and `relays.json` with the specified fields, as plain
/// owner-only files in an owner-only folder, and nothing else.
#[test]
fn sto01_links_holds_the_three_files_as_plain_owner_only_files() {
    let (_home, folder, store) = links();
    paired(&store);
    assert_eq!(
        names(&folder),
        ["hub-monitors.json", "relay-identity.json", "relays.json"]
    );
    assert_eq!(mode(&folder), 0o700);
    for name in FILES {
        let path = folder.join(name);
        assert_eq!(mode(&path), 0o600, "{name}");
        assert!(fs::symlink_metadata(&path).unwrap().is_file(), "{name}");
    }
    let one = "01".repeat(32);
    let three = "03".repeat(32);
    assert_eq!(
        json(&folder.join(RELAY_IDENTITY_FILE)),
        serde_json::json!({
            "version": 1, "public_key": one, "private_key": "02".repeat(32),
        })
    );
    assert_eq!(
        json(&folder.join(HUB_MONITORS_FILE)),
        serde_json::json!({
            "version": 1,
            "hold": true,
            "hub_monitors": [{
                "public_key": three, "name": "laptop",
                "paired_at": 1_795_000_000_u64, "last_linked_at": 1_795_000_600_u64,
            }],
        })
    );
    assert_eq!(
        json(&folder.join(RELAYS_FILE)),
        serde_json::json!({
            "version": 1,
            "relays": [{
                "name": "build box", "address": "10.0.0.7:7222",
                "relay_public_key": one, "public_key": three,
                "private_key": "04".repeat(32),
                "paired_at": 1_795_000_000_u64, "last_linked_at": null,
                "disconnected": true,
            }],
        })
    );
}

/// STO-01 (Contract): another process opening the same folder reads what
/// was stored.
#[test]
fn sto01_the_next_process_reads_what_was_stored() {
    let (_home, folder, store) = links();
    paired(&store);
    let reopened = KeyStore::open(&folder).unwrap();
    let kept = pairing::relay_identity_at_start(&reopened, no_new_identity).unwrap();
    assert_eq!(kept.public_key, PublicKey::new([1; 32]));
    assert_eq!(kept.private_key.expose(), &[2; 32]);
    let remembered = pairing::load_hub_monitors(&reopened).unwrap();
    assert!(remembered.hold);
    assert_eq!(remembered.hub_monitors.records(), [hub_monitor()]);
    let relays = pairing::load_relays(&reopened).unwrap();
    let [kept] = relays.records() else {
        panic!("{relays:?}")
    };
    assert_eq!(kept.name, "build box");
    assert_eq!(kept.private_key.expose(), &[4; 32]);
}

/// STO-01 (Fault): a writer killed mid-write leaves its temporary file
/// behind. It is never read as a record, whatever it holds, and the store
/// keeps the old file.
#[test]
fn sto01_a_killed_writers_temporary_file_is_never_a_record() {
    let (_home, folder, store) = links();
    paired(&store);
    let before = every_read(&store).map(Result::unwrap);
    let mut others = Pairings::default();
    others
        .insert(Relay {
            relay_public_key: PublicKey::new([9; 32]),
            ..relay()
        })
        .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let elsewhere = KeyStore::open(&scratch.path().join("links")).unwrap();
    pairing::save_relays(&elsewhere, &others).unwrap();
    let whole_file = fs::read(scratch.path().join("links").join(RELAYS_FILE)).unwrap();
    let leftovers: [(&str, &[u8]); 4] = [
        (".relays.json.00000000deadbeef.tmp", &whole_file),
        (".relays.json.0123456789abcdef.tmp", &whole_file[..40]),
        (".hub-monitors.json.00000000deadbeef.tmp", b"{"),
        (".relay-identity.json.00000000deadbeef.tmp", b""),
    ];
    for (name, contents) in leftovers {
        fs::write(folder.join(name), contents).unwrap();
    }
    assert_eq!(every_read(&store).map(Result::unwrap), before);
    assert_eq!(pairing::load_relays(&store).unwrap().records().len(), 1);

    // With no store file at all, leftovers still make no record and no
    // identity.
    for name in FILES {
        fs::remove_file(folder.join(name)).unwrap();
    }
    assert!(pairing::load_relays(&store).unwrap().records().is_empty());
    let remembered = pairing::load_hub_monitors(&store).unwrap();
    assert!(remembered.hub_monitors.records().is_empty());
    let made = pairing::relay_identity_at_start(&store, identity).unwrap();
    assert_eq!(made.public_key, PublicKey::new([1; 32]));
}

/// STO-02 (Contract): each store file, damaged in each way a file can be,
/// is an explicit error naming it and is never read as empty.
#[test]
fn sto02_each_damaged_store_file_is_an_error() {
    type Damage = fn(&Path);
    let damages: [(&str, Damage); 6] = [
        ("malformed", |path| {
            fs::write(path, "{\"version\":1").unwrap()
        }),
        ("truncated to nothing", |path| fs::write(path, "").unwrap()),
        ("readable by others", |path| {
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }),
        ("a link", |path| {
            let target = path.with_extension("real");
            fs::rename(path, &target).unwrap();
            symlink(&target, path).unwrap();
        }),
        ("a folder", |path| {
            fs::remove_file(path).unwrap();
            fs::create_dir(path).unwrap();
        }),
        ("another version", |path| {
            let text = fs::read_to_string(path).unwrap();
            fs::write(path, text.replace("\"version\":1", "\"version\":2")).unwrap();
        }),
    ];
    for (what, damage_file) in damages {
        for (index, name) in FILES.into_iter().enumerate() {
            let (_home, folder, store) = links();
            paired(&store);
            damage_file(&folder.join(name));
            let mut reads = every_read(&store);
            let message = damage(std::mem::replace(&mut reads[index], Ok(String::new())));
            assert!(message.contains(name), "{name} {what}: {message}");
            // The other files are still read.
            assert_eq!(reads.iter().filter(|read| read.is_ok()).count(), 3);
        }
    }
}

/// STO-02 (Contract): a missing identity is created only by a relay's
/// start. With a new identity the relay is a different relay, and it still
/// holds the hub monitors it remembered.
#[test]
fn sto02_a_removed_identity_is_made_again_only_at_a_relays_start() {
    let (_home, folder, store) = links();
    paired(&store);
    fs::remove_file(folder.join(RELAY_IDENTITY_FILE)).unwrap();

    pairing::load_hub_monitors(&store).unwrap();
    pairing::load_relays(&store).unwrap();
    assert_eq!(names(&folder), ["hub-monitors.json", "relays.json"]);

    let made = pairing::relay_identity_at_start(&store, || {
        Ok(Keypair {
            public_key: PublicKey::new([7; 32]),
            private_key: PrivateKey::new([8; 32]),
        })
    })
    .unwrap();
    assert_eq!(made.public_key, PublicKey::new([7; 32]));
    assert_eq!(mode(&folder.join(RELAY_IDENTITY_FILE)), 0o600);
    let kept = pairing::relay_identity_at_start(&store, no_new_identity).unwrap();
    assert_eq!(kept.public_key, PublicKey::new([7; 32]));
    assert_eq!(kept.private_key.expose(), &[8; 32]);
    let remembered = pairing::load_hub_monitors(&store).unwrap();
    assert_eq!(remembered.hub_monitors.records(), [hub_monitor()]);
}
