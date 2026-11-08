//! DOC-01 (Contract case for the part-1 daemon wire pages): every published
//! example decodes to the values its page documents and re-encodes to the
//! same bytes. The examples are read from the pages themselves, so the
//! published text and the codec cannot drift apart.
use climon2_core::wire::{
    ControlRecord, Counter, Families, Frame, Kill, Leave, Mode, Open, Request, Requested,
};
use std::{fs, path::Path};

/// The single JSON example on one daemon wire page.
fn example(page: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/protocol/daemon-client")
        .join(page);
    let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let blocks: Vec<_> = text
        .split("```json\n")
        .skip(1)
        .map(|block| block.split_once("\n```").expect("closed JSON example").0)
        .collect();
    assert_eq!(blocks.len(), 1, "{page} publishes one JSON example");
    blocks[0].to_owned()
}

fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}

fn families(tokens: &[&str]) -> Families {
    Families::new(tokens.iter().map(|token| token.to_string()).collect()).unwrap()
}

fn request(code: u8, page: &str) -> (Request, Frame) {
    let frame = Frame::new(code, example(page).into_bytes());
    (Request::from_frame(&frame).unwrap(), frame)
}

/// DOC-01: Open `0x30`, Leave `0x37` and Kill `0x38`.
#[test]
fn published_request_examples_round_trip_byte_identically() {
    let (open, frame) = request(0x30, "request-open.md");
    let expected = Open {
        request_id: counter(1),
        revision: 1,
        capabilities: Requested::new(
            families(&["status-v1"]),
            families(&["title-v1", "notification-v1", "progress-v1", "focus-v1"]),
        )
        .unwrap(),
        mode: Mode::Monitor,
    };
    assert_eq!(open, Request::Open(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (leave, frame) = request(0x37, "request-leave.md");
    let expected = Leave {
        request_id: counter(8),
    };
    assert_eq!(leave, Request::Leave(expected.clone()));
    assert_eq!(expected.to_frame(), frame);

    let (kill, frame) = request(0x38, "request-kill.md");
    let expected = Kill {
        request_id: counter(1),
        revision: 1,
        required_capabilities: families(&[]),
    };
    assert_eq!(kill, Request::Kill(expected.clone()));
    assert_eq!(expected.to_frame(), frame);
}
