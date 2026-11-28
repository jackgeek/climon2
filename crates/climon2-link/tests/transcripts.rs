//! REV-07 at the Link layer (#498): every protocol transcript of every
//! published link revision, replayed to each end in every test run of
//! every release.
//!
//! The files are in `transcripts/revision-<n>/`. Once a link revision
//! ships its transcripts are never refreshed: a changed byte is a failure,
//! not a reason to record again. A new link revision adds a folder of its
//! own and keeps the old ones. `record_missing_transcripts`, which runs
//! only when asked for, writes a transcript that has no file yet and never
//! touches one that has.
//!
//! The handshake of every link connection here uses climon2's frozen
//! vector's keys and randomness (`tests/handshake.rs`), and the pairing
//! connection its seeds (`tests/pairing_vectors.rs`), so those bytes were
//! computed by implementations that share no code with this crate, and
//! the tests there check that they are the transcripts' bytes.

mod samples;
mod support;
mod transcript;
mod wire;

use climon2_link::client::Arguments;
use climon2_link::{
    Advertisement, EndReason, Ended, HubEnd, HubStream, Link, Opening, PairingRun, Purpose,
    Randomness, RelayClose, RelayEnd, RelayRecord, RelayStream, Revisions, StreamEnded,
};
use std::time::Duration;
use support::Seeded;
use support::pairing::{invited, pairing_hub, pairing_relay, pin, revisions, uninvited};
use transcript::{
    Cue, Entry, Scenario, Side, Transcript, folder, record, replay_hub, replay_relay,
};
use wire::{Fixed, hub_key, paired, relay_end, relay_key};

/// The published link revisions, each with the scenarios of its
/// transcripts.
const REVISIONS: &[(u16, Scenarios)] = &[(1, revision_1)];

/// What gives a link revision's scenarios.
type Scenarios = fn() -> Vec<Scenario>;

/// The committed transcript of `scenario` in link revision `revision`.
fn committed(revision: u16, scenario: &Scenario) -> Transcript {
    transcript::load(revision, scenario.name)
}

fn seeded(seed: u64) -> Box<dyn Randomness> {
    Box::new(Seeded::new(seed))
}

/// Randomness that gives the 32 bytes `from`, `from + 1` and so on: an
/// ephemeral private key of climon2's frozen vector.
fn ephemeral(from: u8) -> Box<dyn Randomness> {
    Box::new(Fixed::new(&(from..from + 32).collect::<Vec<_>>()))
}

/// The frozen vector's ephemeral key of the hub monitor.
fn hub_ephemeral() -> Box<dyn Randomness> {
    ephemeral(0x40)
}

/// The frozen vector's ephemeral key of the relay.
fn relay_ephemeral() -> Box<dyn Randomness> {
    ephemeral(0x60)
}

fn seed_1() -> Box<dyn Randomness> {
    seeded(1)
}

fn seed_2() -> Box<dyn Randomness> {
    seeded(2)
}

fn at(seconds: u64) -> Cue {
    Cue::At(Duration::from_secs(seconds))
}

/// A relay that remembers the hub monitor of the frozen vector and refuses
/// it with `reason`.
fn refusing(reason: RelayClose) -> RelayEnd {
    relay_end(relay_key(), &[hub_key().public()]).refusing(hub_key().public(), reason)
}

/// A scenario of a link connection between the frozen vector's two ends,
/// the relay's being `relay`.
fn linking(
    name: &'static str,
    notes: &'static str,
    relay: fn() -> RelayEnd,
    cues: Vec<Cue>,
) -> Scenario {
    Scenario {
        name,
        notes,
        hub: || paired().0,
        relay,
        hub_randomness: hub_ephemeral,
        relay_randomness: relay_ephemeral,
        cues,
    }
}

fn refusal(name: &'static str, notes: &'static str, relay: fn() -> RelayEnd) -> Scenario {
    linking(name, notes, relay, Vec::new())
}

/// Link revision 1's transcripts.
fn revision_1() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "opening-chosen",
            notes: "The opening exchange of a hub monitor that comes to link, both ends\n\
                    supporting only link revision 1: revision 1 is chosen.",
            hub: || HubEnd::new(Purpose::Link, revisions(&[1])),
            relay: || RelayEnd::new(revisions(&[1]), uninvited()),
            hub_randomness: seed_1,
            relay_randomness: seed_2,
            cues: Vec::new(),
        },
        Scenario {
            name: "opening-no-shared-revision",
            notes: "The opening exchange of a hub monitor that comes to link supporting\n\
                    only link revision 1, and a relay that supports only revisions 2 and 3:\n\
                    the relay refuses with no shared revision and both ends close.",
            hub: || HubEnd::new(Purpose::Link, revisions(&[1])),
            relay: || RelayEnd::new(revisions(&[2, 3]), uninvited()),
            hub_randomness: seed_1,
            relay_randomness: seed_2,
            cues: Vec::new(),
        },
        Scenario {
            name: "pairing-correct-pin",
            notes: "A pairing connection with the PIN 8517-9087, minted from seed 3: the\n\
                    hub monitor's randomness is seed 1 and the relay's seed 2. The hub\n\
                    monitor introduces itself as `laptop`, the relay as `büild-box`.",
            hub: || pairing_hub(&invited().1),
            relay: || pairing_relay(&invited().0),
            hub_randomness: seed_1,
            relay_randomness: seed_2,
            cues: Vec::new(),
        },
        Scenario {
            name: "pairing-wrong-pin",
            notes: "The pairing connection of pairing-correct-pin, but the hub monitor\n\
                    types 1234-5678: the relay checks the hub monitor's confirmation, finds\n\
                    it wrong and closes without its own.",
            hub: || pairing_hub(&pin("1234-5678")),
            relay: || pairing_relay(&invited().0),
            hub_randomness: seed_1,
            relay_randomness: seed_2,
            cues: Vec::new(),
        },
        linking(
            "handshake",
            "A link connection with climon2's frozen vector's keys and ephemeral\n\
             keys: the opening exchange, the pinned-key handshake and both Hellos,\n\
             the hub monitor's `2.4.0`, `atlas`, 1,048,576 and the relay's `2.5.1`,\n\
             `forge`, 4,096.",
            || paired().1,
            Vec::new(),
        ),
        refusal(
            "refusal-not-paired",
            "The link connection of handshake to a relay that does not remember the\n\
             hub monitor: it answers the hub monitor's Hello with Close alone, giving\n\
             \"not paired\" (4).",
            || relay_end(relay_key(), &[]),
        ),
        refusal(
            "refusal-on-hold",
            "The link connection of handshake to a relay on hold: Close alone, \"on\n\
             hold\" (5).",
            || refusing(RelayClose::OnHold),
        ),
        refusal(
            "refusal-busy",
            "The link connection of handshake to a relay busy with another hub\n\
             monitor: Close alone, \"busy\" (6).",
            || refusing(RelayClose::Busy),
        ),
        refusal(
            "refusal-replaced",
            "The link connection of handshake to a relay that refuses it as replaced\n\
             by a newly paired hub monitor: Close alone, \"replaced\" (7).",
            || refusing(RelayClose::Replaced),
        ),
        linking(
            "link",
            "A short link over the link connection of handshake. At 1 s the relay\n\
             advertises one session and completes its snapshot; the hub monitor opens\n\
             stream 1 with 1,048,576 bytes of credit and the relay answers it. At 2 s\n\
             the hub monitor sends an input request, the relay a 20,000-byte display\n\
             event in two fragments, and the hub monitor 65,536 bytes more credit.\n\
             At 7 s each end, having sent nothing for 5 seconds, sends a keepalive.\n\
             At 8 s the hub monitor closes stream 1, the relay ends it and then closes\n\
             the link, giving \"relay stopping\" (2).",
            || paired().1,
            short_link(),
        ),
    ]
}

/// The script of the short link.
fn short_link() -> Vec<Cue> {
    vec![
        at(1),
        Cue::Relay("advertise a session", |relay| {
            let session = Advertisement::Session(samples::session());
            send_relay(relay, RelayRecord::Advertisement(session));
        }),
        Cue::Relay("complete the snapshot", |relay| {
            send_relay(relay, samples::advertise(Advertisement::SnapshotComplete));
        }),
        Cue::Hub("open stream 1", |hub| {
            send_hub(hub, HubStream::Open(open_stream()));
        }),
        Cue::Relay("answer stream 1", |relay| {
            send_relay(relay, samples::from_relay(1, samples::stream_opened()));
        }),
        at(2),
        Cue::Hub("request input on stream 1", |hub| {
            let (input, _) = samples::requests()
                .into_iter()
                .find(|(request, _)| matches!(request.arguments, Arguments::Input(_)))
                .expect("an input request");
            send_hub(hub, HubStream::Request(input));
        }),
        Cue::Relay("send a display event on stream 1", |relay| {
            let event = RelayStream::Event(samples::display(20_000, b'x'));
            send_relay(relay, samples::from_relay(1, event));
        }),
        Cue::Hub("credit stream 1", |hub| {
            send_hub(hub, HubStream::Credit(65_536));
        }),
        at(7),
        at(8),
        Cue::Hub("close stream 1", |hub| send_hub(hub, HubStream::Close)),
        Cue::Relay("end stream 1", |relay| {
            send_relay(relay, samples::from_relay(1, stream_ended()));
        }),
        Cue::Relay("stop", |relay| {
            relay
                .close(RelayClose::Stopping)
                .expect("an established link");
        }),
    ]
}

fn open_stream() -> climon2_link::OpenStream {
    let HubStream::Open(open) = samples::open_stream() else {
        unreachable!()
    };
    open
}

fn stream_ended() -> RelayStream {
    RelayStream::Ended(StreamEnded {
        reason: EndReason::ClientEnded,
        workers_cleanup: 1,
        generation: 3,
        client_outcome: 1,
        retry: 1,
        leave: 2,
        attachment_detached: 0,
        connection_closed: 1,
        error: samples::no_error(),
    })
}

fn send_hub(hub: &mut HubEnd, record: HubStream) {
    hub.send(&samples::from_hub(1, record))
        .expect("a record in turn");
}

fn send_relay(relay: &mut RelayEnd, record: RelayRecord) {
    relay.send(&record).expect("a record in turn");
}

/// Every published revision's scenarios, each with its revision.
fn every_scenario() -> Vec<(u16, Scenario)> {
    REVISIONS
        .iter()
        .flat_map(|(revision, scenarios)| scenarios().into_iter().map(|s| (*revision, s)))
        .collect()
}

/// REV-07: each published link revision has transcripts, and no folder
/// holds a revision that is not published.
#[test]
fn rev_07_every_published_revision_has_its_transcripts() {
    let listed: Vec<u16> = REVISIONS.iter().map(|(revision, _)| *revision).collect();
    assert_eq!(listed, Revisions::published().as_slice());
    let mut folders: Vec<String> = std::fs::read_dir(folder(1).parent().unwrap())
        .expect("the transcripts folder")
        .map(|entry| entry.expect("an entry").file_name().into_string().unwrap())
        .collect();
    folders.sort();
    let expected: Vec<String> = listed.iter().map(|r| format!("revision-{r}")).collect();
    assert_eq!(folders, expected);
}

/// REV-07: every transcript replayed to the hub monitor's end. Handed the
/// relay's recorded bytes, it sends exactly its own.
#[test]
fn rev_07_the_hub_monitor_reproduces_every_transcript() {
    for (revision, scenario) in every_scenario() {
        replay_hub(&scenario, &committed(revision, &scenario));
    }
}

/// REV-07: every transcript replayed to the relay's end. Handed the hub
/// monitor's recorded bytes, it sends exactly its own.
#[test]
fn rev_07_the_relay_reproduces_every_transcript() {
    for (revision, scenario) in every_scenario() {
        replay_relay(&scenario, &committed(revision, &scenario));
    }
}

/// Each folder holds exactly the files of its scenarios.
#[test]
fn rev_07_each_folder_holds_its_scenarios_files() {
    for (revision, scenarios) in REVISIONS {
        let mut names: Vec<String> = scenarios()
            .iter()
            .map(|scenario| format!("{}.txt", scenario.name))
            .collect();
        names.sort();
        let mut files: Vec<String> = std::fs::read_dir(folder(*revision))
            .expect("the revision's folder")
            .map(|entry| entry.expect("an entry").file_name().into_string().unwrap())
            .collect();
        files.sort();
        assert_eq!(files, names, "revision {revision}");
    }
}

/// The scenario of link revision 1 called `name`, as both ends finish it
/// when replayed: the hub monitor's end and the relay's.
fn replayed(name: &str) -> (HubEnd, RelayEnd, Transcript) {
    let scenario = revision_1()
        .into_iter()
        .find(|scenario| scenario.name == name)
        .expect("a scenario of that name");
    let transcript = committed(1, &scenario);
    let hub = replay_hub(&scenario, &transcript);
    let relay = replay_relay(&scenario, &transcript);
    (hub, relay, transcript)
}

/// The transcripts are the exchanges the acceptance criteria list: each
/// end finishes its replay where the exchange's name says.
#[test]
fn rev_07_each_transcript_is_the_exchange_it_names() {
    let (hub, relay, _) = replayed("opening-chosen");
    assert!(matches!(hub.opening(), Opening::Opened(opened) if opened.revision == 1));
    assert!(matches!(relay.opening(), Opening::Opened(opened) if opened.revision == 1));

    let (hub, relay, transcript) = replayed("opening-no-shared-revision");
    let incompatible = |theirs: &[u16]| Opening::Incompatible {
        theirs: revisions(theirs),
    };
    assert_eq!(hub.opening(), &incompatible(&[2, 3]));
    assert_eq!(relay.opening(), &incompatible(&[1]));
    assert!(transcript.0.contains(&Entry::Closes(Side::Relay)));

    let (hub, relay, _) = replayed("pairing-correct-pin");
    assert!(matches!(hub.pairing(), PairingRun::Paired(_)));
    assert!(matches!(relay.pairing(), PairingRun::Paired(_)));

    let (hub, relay, _) = replayed("pairing-wrong-pin");
    let failed = PairingRun::Failed { counted: true };
    assert_eq!((hub.pairing(), relay.pairing()), (&failed, &failed));

    let (hub, relay, _) = replayed("handshake");
    assert!(matches!(hub.link(), Link::Established(_)));
    assert!(matches!(relay.link(), Link::Established(_)));

    let refusals = [
        ("refusal-not-paired", RelayClose::NotPaired),
        ("refusal-on-hold", RelayClose::OnHold),
        ("refusal-busy", RelayClose::Busy),
        ("refusal-replaced", RelayClose::Replaced),
    ];
    for (name, reason) in refusals {
        let (hub, relay, _) = replayed(name);
        assert_eq!(hub.link(), &Link::Ended(Ended::Closed(reason)), "{name}");
        assert_eq!(relay.link(), &Link::Ended(Ended::Left(reason)), "{name}");
    }

    let (hub, relay, _) = replayed("link");
    assert_eq!(
        hub.link(),
        &Link::Ended(Ended::Closed(RelayClose::Stopping))
    );
    assert_eq!(
        relay.link(),
        &Link::Ended(Ended::Left(RelayClose::Stopping))
    );
}

/// Whether replaying `transcript` to the end on `side` of the scenario
/// called `name` fails.
fn fails(name: &str, side: Side, transcript: &Transcript) -> bool {
    let scenario = revision_1()
        .into_iter()
        .find(|scenario| scenario.name == name)
        .expect("a scenario of that name");
    std::panic::catch_unwind(|| match side {
        Side::Hub => drop(replay_hub(&scenario, transcript)),
        Side::Relay => drop(replay_relay(&scenario, transcript)),
    })
    .is_err()
}

/// The replay is exact: one byte changed in what either end sent fails
/// the replay of both ends, the sender because it sends another byte and
/// the receiver because it answers what it was handed otherwise. So does a
/// close that is not in the transcript, and one that is missing.
#[test]
fn rev_07_a_changed_byte_or_close_fails_both_replays() {
    let handshake = transcript::load(1, "handshake");
    for side in [Side::Hub, Side::Relay] {
        let mut changed = handshake.clone();
        let last = changed
            .0
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                Entry::Sends(by, bytes) if *by == side => Some(bytes),
                _ => None,
            })
            .expect("bytes the side sent");
        last[2] ^= 1;
        assert!(fails("handshake", Side::Hub, &changed), "{side:?}");
        assert!(fails("handshake", Side::Relay, &changed), "{side:?}");

        let mut closing = handshake.clone();
        closing.0.push(Entry::Closes(side));
        assert!(fails("handshake", side, &closing), "{side:?}");
    }
    let mut unclosed = transcript::load(1, "refusal-busy");
    unclosed
        .0
        .retain(|entry| !matches!(entry, Entry::Closes(_)));
    assert!(fails("refusal-busy", Side::Hub, &unclosed));
    assert!(fails("refusal-busy", Side::Relay, &unclosed));
}

/// Writes the transcript of every scenario that has no file yet, and
/// checks that recording again gives every file that exists. It runs only
/// when asked for:
/// `cargo test -p climon2-link --test transcripts -- --ignored`.
#[test]
#[ignore = "records transcripts that have no file yet"]
fn record_missing_transcripts() {
    for (revision, scenario) in every_scenario() {
        let (transcript, _, _) = record(&scenario);
        let path = folder(revision).join(format!("{}.txt", scenario.name));
        if path.exists() {
            assert_eq!(
                committed(revision, &scenario),
                transcript,
                "{}: a shipped transcript is never recorded again",
                path.display()
            );
            continue;
        }
        std::fs::create_dir_all(folder(revision)).expect("the revision's folder");
        let notes = format!(
            "Link revision {revision}: {}\n\n{}",
            scenario.name, scenario.notes
        );
        std::fs::write(&path, transcript.render(&notes)).expect("the transcript");
    }
}
