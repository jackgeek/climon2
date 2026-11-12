//! Helpers shared by core's integration tests; each test uses a subset.
#![allow(dead_code)]

use climon2_core::wire::{Counter, Families};
use std::{fs, path::Path};

/// Every JSON example in a Markdown document, in document order.
pub fn json_examples(document: &str) -> Vec<String> {
    document
        .replace("\r\n", "\n")
        .split("```json\n")
        .skip(1)
        .map(|block| block.split_once("\n```").expect("closed JSON example").0)
        .map(str::to_owned)
        .collect()
}

/// Every JSON example on one daemon wire page, read from the page itself.
pub fn page_examples(page: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/protocol/daemon-client")
        .join(page);
    json_examples(&fs::read_to_string(&path).unwrap())
}

pub fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}

pub fn families(tokens: &[&str]) -> Families {
    Families::new(tokens.iter().map(|token| token.to_string()).collect()).unwrap()
}
