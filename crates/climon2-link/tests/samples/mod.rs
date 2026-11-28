//! Records of #496 as tests send them: a linked end with a link peer
//! stand-in at the other end, and every advertisement, session-stream and
//! client interface record next to its bytes. The bytes are written by hand
//! from the protocol's record pages and the client interface header, field
//! by field, never by the code under test.

// Each test binary compiles this module for itself and uses its own part.
#![allow(dead_code)]

use crate::wire::{self, SECOND, StandIn, hex, hub_end, hub_key, relay_end, relay_key, step};
use climon2_link::client::{
    Answer, Arguments, Authority, AuthorityResult, AvailableProgress, AvailableStatus, Barrier,
    CommandResult, Data, Display, Error, Event, Final, Flag, FlagResult, Geometry, GeometryChange,
    Granted, Input, Metadata, Notification, Open, Opened, OptionalCommand, OptionalText, Progress,
    Request, Resize, Restoration, Revoked, Status, Termination, Title,
};
use climon2_link::{
    Advertisement, HubEnd, HubRecord, HubStream, Input as Step, Link, OpenStream, RelayEnd,
    RelayRecord, RelayStream, Session, StreamOpened,
};
use std::time::Duration;

/// The Hello the relay stand-in of these tests sends: `2.5.1`, `forge`,
/// 1,048,576.
pub const RELAY_HELLO: &[u8] = b"\x01\0\0\0\x052.5.1\0\0\0\x05forge\0\0\0\0\0\x10\0\0";

/// The time of every step after the link is established.
pub const LATER: Duration = Duration::from_secs(2);

/// The plaintext of a unit that holds a whole record on `stream`.
pub fn on(stream: u32, record: &[u8]) -> Vec<u8> {
    [&stream.to_be_bytes()[..], &[0], record].concat()
}

/// A relay that remembers the stand-in, dialled by it and linked.
pub fn linked_relay() -> (RelayEnd, StandIn) {
    let mut relay = relay_end(relay_key(), &[hub_key().public()]);
    let mut stand_in = StandIn::dial(&mut relay, [0x11; 32], relay_key().public());
    let hello = stand_in.record(&wire::HELLO);
    let output = step(&mut relay, SECOND, Step::Bytes(&hello));
    assert_eq!(stand_in.open(&output.bytes).len(), 1, "the relay's Hello");
    assert!(matches!(relay.link(), Link::Established(_)));
    (relay, stand_in)
}

/// A hub monitor answered by the stand-in and linked. The stand-in's Hello
/// accepts records up to 1,048,576 bytes.
pub fn linked_hub() -> (HubEnd, StandIn) {
    let mut hub = hub_end(hub_key(), relay_key().public());
    let mut stand_in = StandIn::answer(&mut hub, [0x22; 32]);
    let hello = stand_in.record(RELAY_HELLO);
    step(&mut hub, SECOND, Step::Bytes(&hello));
    assert!(matches!(hub.link(), Link::Established(_)));
    (hub, stand_in)
}

/// The session ID of the Session page's example.
pub const SESSION_ID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];

/// `2026-11-27T09:30:00.000Z`, a UTC timestamp in the client interface's
/// form.
pub const CREATED_AT: [u8; 24] = *b"2026-11-27T09:30:00.000Z";

/// The Session page's example: `vim notes.txt` in `/home/sam`.
pub fn session() -> Session {
    Session {
        id: SESSION_ID,
        encoding: 1,
        command: b"vim".to_vec(),
        arguments: vec![b"notes.txt".to_vec()],
        working_directory: b"/home/sam".to_vec(),
        depth: 1,
        created_at: CREATED_AT,
        release: "2.4.0".to_owned(),
    }
}

/// The Session page's example record.
pub fn session_record() -> Vec<u8> {
    hex("10 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd
         ef 00 00 00 01 00 00 00 03 76 69 6d 00 00 00 01
         00 00 00 09 6e 6f 74 65 73 2e 74 78 74 00 00 00
         09 2f 68 6f 6d 65 2f 73 61 6d 00 00 00 01 32 30
         32 36 2d 31 31 2d 32 37 54 30 39 3a 33 30 3a 30
         30 2e 30 30 30 5a 00 00 00 05 32 2e 34 2e 30")
}

/// A session with this ID and nothing else.
pub fn session_with(id: [u8; 16]) -> Session {
    Session {
        id,
        encoding: 1,
        command: Vec::new(),
        arguments: Vec::new(),
        working_directory: Vec::new(),
        depth: 0,
        created_at: [0; 24],
        release: "2".to_owned(),
    }
}

pub fn advertise(advertisement: Advertisement) -> RelayRecord {
    RelayRecord::Advertisement(advertisement)
}

/// The Open stream page's example: session `0123…cdef`, generation 3 and
/// 1,048,576 bytes of credit.
pub fn open_stream() -> HubStream {
    HubStream::Open(OpenStream {
        session: SESSION_ID,
        generation: 3,
        credit: 1_048_576,
    })
}

/// The Open stream page's example record.
pub fn open_stream_record() -> Vec<u8> {
    hex("20 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd
         ef 00 00 00 00 00 00 00 03 00 00 00 01 00 10 00
         00")
}

pub fn from_hub(stream: u32, record: HubStream) -> HubRecord {
    HubRecord { stream, record }
}

pub fn from_relay(stream: u32, record: RelayStream) -> RelayRecord {
    RelayRecord::Stream { stream, record }
}

/// The Stream opened page's example: a library that offers `status-v1`.
pub fn stream_opened() -> RelayStream {
    RelayStream::Opened(StreamOpened {
        capabilities: vec!["status-v1".to_owned()],
    })
}

pub fn stream_opened_record() -> Vec<u8> {
    hex("21 00 00 00 01 00 00 00 09 73 74 61 74 75 73 2d 76 31")
}

/// A relay linked with the stand-in, with stream 1 opened by the stand-in
/// and answered with Stream opened.
pub fn relay_streaming() -> (RelayEnd, StandIn) {
    let (mut relay, mut stand_in) = linked_relay();
    let open = stand_in.unit(1, 0, &open_stream_record());
    step(&mut relay, LATER, Step::Bytes(&open));
    assert_eq!(relay.records(), [from_hub(1, open_stream())]);
    relay
        .send(&from_relay(1, stream_opened()))
        .expect("Stream opened");
    let output = step(&mut relay, LATER, Step::Tick);
    assert_eq!(
        stand_in.open(&output.bytes),
        [on(1, &stream_opened_record())]
    );
    (relay, stand_in)
}

/// A hub monitor linked with the stand-in, with stream 1 opened and
/// answered by the stand-in with Stream opened.
pub fn hub_streaming() -> (HubEnd, StandIn) {
    let (mut hub, mut stand_in) = linked_hub();
    hub.send(&from_hub(1, open_stream())).expect("Open stream");
    let output = step(&mut hub, LATER, Step::Tick);
    assert_eq!(stand_in.open(&output.bytes), [on(1, &open_stream_record())]);
    let opened = stand_in.unit(1, 0, &stream_opened_record());
    step(&mut hub, LATER, Step::Bytes(&opened));
    assert_eq!(hub.records(), [from_relay(1, stream_opened())]);
    (hub, stand_in)
}

/// The authority of most samples: generation 3, control period 2,
/// restoration 1.
fn authority() -> Authority {
    Authority {
        generation: 3,
        period: 2,
        restoration_id: 1,
    }
}
const AUTHORITY: &str = "00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 02 00 00 00 00 00 00 00 01";

/// A geometry of 80 columns and 24 rows.
fn geometry() -> Geometry {
    Geometry {
        columns: 80,
        rows: 24,
    }
}
const GEOMETRY: &str = "00 00 00 50 00 00 00 18";

fn request(arguments: Arguments) -> Request {
    Request { id: 9, arguments }
}
const REQUEST_ID: &str = "00 00 00 00 00 00 00 09";

/// Every request, each next to its record.
pub fn requests() -> Vec<(Request, Vec<u8>)> {
    let flag = |value| Flag {
        authority: authority(),
        value,
    };
    let tokens = |tokens: &[&str]| tokens.iter().map(|token| token.to_string()).collect();
    [
        (
            request(Arguments::Open(Open {
                mode: 2,
                geometry: geometry(),
                required: tokens(&["a"]),
                optional: tokens(&["bc", "d"]),
            })),
            format!(
                "41 {REQUEST_ID} 00 00 00 02 {GEOMETRY} 00 00 00 01 00 00 00 01 61
                 00 00 00 02 00 00 00 02 62 63 00 00 00 01 64"
            ),
        ),
        (
            request(Arguments::Input(Input {
                authority: authority(),
                bytes: b"ls\r".to_vec(),
            })),
            // The Request page's example.
            "42 00 00 00 00 00 00 00 09 00 00 00 00 00 00 00
             03 00 00 00 00 00 00 00 02 00 00 00 00 00 00 00
             01 00 00 00 03 6c 73 0d"
                .to_owned(),
        ),
        (
            request(Arguments::Resize(Resize {
                authority: authority(),
                geometry: Geometry {
                    columns: 100,
                    rows: 30,
                },
            })),
            format!("43 {REQUEST_ID} {AUTHORITY} 00 00 00 64 00 00 00 1e"),
        ),
        (
            request(Arguments::TakeControl(3)),
            format!("44 {REQUEST_ID} 00 00 00 00 00 00 00 03"),
        ),
        (
            request(Arguments::Ready(authority())),
            format!("45 {REQUEST_ID} {AUTHORITY}"),
        ),
        (
            request(Arguments::Viewing(flag(1))),
            format!("46 {REQUEST_ID} {AUTHORITY} 01"),
        ),
        (
            request(Arguments::Acknowledge(authority())),
            format!("47 {REQUEST_ID} {AUTHORITY}"),
        ),
        (
            request(Arguments::Leave(4)),
            format!("48 {REQUEST_ID} 00 00 00 00 00 00 00 04"),
        ),
        (
            request(Arguments::Kill(tokens(&["a"]))),
            format!("49 {REQUEST_ID} 00 00 00 01 00 00 00 01 61"),
        ),
        (
            request(Arguments::Focus(flag(0))),
            format!("4a {REQUEST_ID} {AUTHORITY} 00"),
        ),
        (
            request(Arguments::Break(authority())),
            format!("4b {REQUEST_ID} {AUTHORITY}"),
        ),
    ]
    .into_iter()
    .map(|(request, bytes)| (request, hex(&bytes)))
    .collect()
}

/// An absent error.
pub fn no_error() -> Error {
    Error {
        origin: 0,
        code: String::new(),
        message: String::new(),
    }
}
const NO_ERROR: &str = "00 00 00 00 00 00 00 00 00 00 00 00";

/// An error from the daemon with the code `c` and the message `m`.
pub fn error() -> Error {
    Error {
        origin: 2,
        code: "c".to_owned(),
        message: "m".to_owned(),
    }
}
pub const ERROR: &str = "00 00 00 02 00 00 00 01 63 00 00 00 01 6d";

fn text(present: u32, value: &str) -> OptionalText {
    OptionalText {
        present,
        value: value.to_owned(),
    }
}
const ABSENT: &str = "00 00 00 00 00 00 00 00";

fn exited() -> CommandResult {
    CommandResult {
        kind: 1,
        value: 0,
        signal_name: text(0, ""),
    }
}
const EXITED: &str = "00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00";

const UNSET_TIME: &str = "00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";
const CREATED: &str = "32 30 32 36 2d 31 31 2d 32 37 54 30 39 3a 33 30 3a 30 30 2e 30 30 30 5a";

/// A Response event, generation 3, request 9, outcome 1 and no error,
/// with this answer; and the record's bytes up to the answer's tag.
fn response(answer: Answer) -> Event {
    event(Data::Response(climon2_link::client::Response {
        request_id: 9,
        outcome: 1,
        error: no_error(),
        answer,
    }))
}
const RESPONSE: &str = "81 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 09 00 00 00 01";

fn event(data: Data) -> Event {
    Event {
        generation: 3,
        data,
    }
}
const GENERATION: &str = "00 00 00 00 00 00 00 03";

/// Every response answer, each in a Response event next to its record.
pub fn responses() -> Vec<(Event, Vec<u8>)> {
    let authority = |changed| AuthorityResult {
        changed,
        period: 2,
        restoration_id: 1,
    };
    let barrier = "00 00 00 00 00 00 00 02 00 00 00 00 00 00 00 01";
    [
        (Answer::None, format!("00 00 00 00 {NO_ERROR}")),
        (
            Answer::Opened(Opened {
                connection_id: [0x10; 16],
                selected: vec!["a".to_owned()],
            }),
            format!(
                "00 00 00 01 {NO_ERROR} 10 10 10 10 10 10 10 10 10 10 10 10 10 10 10 10
                 00 00 00 01 00 00 00 01 61"
            ),
        ),
        (
            Answer::InputAccepted(3),
            format!("00 00 00 02 {NO_ERROR} 00 00 00 03"),
        ),
        (Answer::ResizeAccepted, format!("00 00 00 03 {NO_ERROR}")),
        (
            Answer::Acquired(authority(1)),
            format!("00 00 00 04 {NO_ERROR} 01 {barrier}"),
        ),
        (
            Answer::Ready(authority(0)),
            format!("00 00 00 05 {NO_ERROR} 00 {barrier}"),
        ),
        (
            Answer::Viewing(FlagResult {
                changed: 1,
                value: 1,
                accepted_bytes: 0,
            }),
            format!("00 00 00 06 {NO_ERROR} 01 01 00 00 00 00"),
        ),
        (
            Answer::Acknowledgement(1),
            format!("00 00 00 07 {NO_ERROR} 01"),
        ),
        (Answer::Left(0), format!("00 00 00 08 {NO_ERROR} 00")),
        (
            Answer::Termination(Termination {
                command: OptionalCommand {
                    present: 1,
                    command: exited(),
                },
                cleanup: 1,
            }),
            format!("00 00 00 09 {NO_ERROR} 00 00 00 01 {EXITED} 00 00 00 01"),
        ),
        (
            Answer::Focus(FlagResult {
                changed: 0,
                value: 1,
                accepted_bytes: 3,
            }),
            format!("00 00 00 0a {NO_ERROR} 00 01 00 00 00 03"),
        ),
        (
            Answer::KnownFinal(CommandResult {
                kind: 2,
                value: 9,
                signal_name: text(1, "KILL"),
            }),
            format!(
                "00 00 00 0b {NO_ERROR} 00 00 00 02 00 00 00 09 00 00 00 01 00 00 00 04
                 4b 49 4c 4c"
            ),
        ),
        (Answer::BreakSent, format!("00 00 00 0c {NO_ERROR}")),
    ]
    .into_iter()
    .map(|(answer, rest)| (response(answer), hex(&format!("{RESPONSE} {rest}"))))
    .collect()
}

/// Every event but Response, each next to its record, and an error in a
/// Response.
pub fn events() -> Vec<(Event, Vec<u8>)> {
    let barrier = "00 00 00 00 00 00 00 02 00 00 00 00 00 00 00 01";
    let status = |state, episode_present, episode_id| Status {
        state,
        episode_present,
        episode_id,
        changed_at: CREATED_AT,
    };
    let refused = event(Data::Response(climon2_link::client::Response {
        request_id: 9,
        outcome: 3,
        error: error(),
        answer: Answer::BreakSent,
    }));
    let refused_bytes =
        format!("81 {GENERATION} 00 00 00 00 00 00 00 09 00 00 00 03 00 00 00 0c {ERROR}");
    [
        (refused, refused_bytes),
        (
            event(Data::InitialMetadata(Metadata {
                title: Title {
                    state: 2,
                    value: b"t".to_vec(),
                },
                status: AvailableStatus {
                    available: 1,
                    status: status(3, 1, 7),
                },
                progress: AvailableProgress {
                    available: 1,
                    progress: Progress {
                        state: 2,
                        percent_present: 1,
                        percent: 40,
                    },
                },
                command: OptionalCommand {
                    present: 0,
                    command: CommandResult {
                        kind: 0,
                        value: 0,
                        signal_name: text(0, ""),
                    },
                },
            })),
            format!(
                "82 {GENERATION} 00 00 00 02 00 00 00 01 74
                 00 00 00 01 00 00 00 03 00 00 00 01 00 00 00 00 00 00 00 07 {CREATED}
                 00 00 00 01 00 00 00 02 00 00 00 01 00 00 00 28
                 00 00 00 00 00 00 00 00 00 00 00 00 {ABSENT}"
            ),
        ),
        (
            event(Data::Title(Title {
                state: 2,
                value: vec![0xff, 0xfe],
            })),
            format!("83 {GENERATION} 00 00 00 02 00 00 00 02 ff fe"),
        ),
        (
            event(Data::Status(status(1, 0, 0))),
            format!("84 {GENERATION} 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 {CREATED}"),
        ),
        (
            event(Data::Notification(Notification {
                source: 2,
                episode_present: 1,
                episode_id: 5,
                source_id: text(1, "s"),
                title: text(1, "T"),
                body: text(0, ""),
            })),
            format!(
                "85 {GENERATION} 00 00 00 02 00 00 00 01 00 00 00 00 00 00 00 05
                 00 00 00 01 00 00 00 01 73 00 00 00 01 00 00 00 01 54 {ABSENT}"
            ),
        ),
        (
            event(Data::Progress(Progress {
                state: 3,
                percent_present: 0,
                percent: 0,
            })),
            format!("86 {GENERATION} 00 00 00 03 00 00 00 00 00 00 00 00"),
        ),
        (
            event(Data::Granted(Granted {
                period: 2,
                restoration_id: 1,
                geometry: geometry(),
                reason: 1,
            })),
            format!("87 {GENERATION} {barrier} {GEOMETRY} 00 00 00 01"),
        ),
        (
            event(Data::Revoked(Revoked {
                period: 2,
                restoration_id: 1,
                reason: 2,
            })),
            format!("88 {GENERATION} {barrier} 00 00 00 02"),
        ),
        (
            event(Data::RestorationStart(Restoration {
                period: 2,
                restoration_id: 1,
                geometry: geometry(),
            })),
            format!("89 {GENERATION} {barrier} {GEOMETRY}"),
        ),
        (
            event(Data::Display(Display {
                authority: Barrier {
                    period: 2,
                    restoration_id: 1,
                },
                bytes: b"hi".to_vec(),
            })),
            // The Event page's example.
            "8a 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00
             02 00 00 00 00 00 00 00 01 00 00 00 02 68 69"
                .to_owned(),
        ),
        (
            event(Data::RestorationComplete(Barrier {
                period: 2,
                restoration_id: 1,
            })),
            format!("8b {GENERATION} {barrier}"),
        ),
        (
            event(Data::Final(Final {
                command: exited(),
                status: AvailableStatus {
                    available: 0,
                    status: Status {
                        state: 0,
                        episode_present: 0,
                        episode_id: 0,
                        changed_at: [0; 24],
                    },
                },
            })),
            format!(
                "8c {GENERATION} {EXITED} 00 00 00 00
                 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 {UNSET_TIME}"
            ),
        ),
        (
            event(Data::Geometry(GeometryChange {
                period: 2,
                geometry: Geometry {
                    columns: 100,
                    rows: 30,
                },
            })),
            format!("8d {GENERATION} 00 00 00 00 00 00 00 02 00 00 00 64 00 00 00 1e"),
        ),
        (
            event(Data::Diagnostic(error())),
            format!("8e {GENERATION} {ERROR}"),
        ),
    ]
    .into_iter()
    .map(|(event, bytes)| (event, hex(&bytes)))
    .collect()
}

/// A Display event on generation 3 holding `size` bytes of display.
pub fn display(size: usize, byte: u8) -> Event {
    event(Data::Display(Display {
        authority: Barrier {
            period: 2,
            restoration_id: 1,
        },
        bytes: vec![byte; size],
    }))
}
