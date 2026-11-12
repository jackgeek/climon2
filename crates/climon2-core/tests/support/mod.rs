//! Helpers shared by core's integration tests; each test uses a subset.
#![allow(dead_code)]

use climon2_core::wire::{Counter, Families, Geometry};
use std::{fs, num::NonZeroU32, path::Path};

/// Every fenced block of `language` in a Markdown document, in document
/// order.
fn fenced(document: &str, language: &str) -> Vec<String> {
    document
        .replace("\r\n", "\n")
        .split(&format!("```{language}\n"))
        .skip(1)
        .map(|block| block.split_once("\n```").expect("closed example").0)
        .map(str::to_owned)
        .collect()
}

/// Every JSON example in a Markdown document, in document order.
pub fn json_examples(document: &str) -> Vec<String> {
    fenced(document, "json")
}

fn read_page(page: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/protocol/daemon-client")
        .join(page);
    fs::read_to_string(&path).unwrap()
}

/// Every JSON example on one daemon wire page, read from the page itself.
pub fn page_examples(page: &str) -> Vec<String> {
    json_examples(&read_page(page))
}

/// Every hex dump on one daemon wire page, as the bytes it shows: each is a
/// whole frame, length prefix included.
pub fn page_dumps(page: &str) -> Vec<Vec<u8>> {
    fenced(&read_page(page), "text")
        .iter()
        .map(|dump| {
            dump.split_whitespace()
                .map(|byte| {
                    assert_eq!(byte.len(), 2, "{byte} is one hex byte");
                    u8::from_str_radix(byte, 16).unwrap()
                })
                .collect()
        })
        .collect()
}

pub fn counter(value: u64) -> Counter {
    Counter::new(value).unwrap()
}

pub fn families(tokens: &[&str]) -> Families {
    Families::new(tokens.iter().map(|token| token.to_string()).collect()).unwrap()
}

pub fn geometry(cols: u32, rows: u32) -> Geometry {
    Geometry {
        cols: NonZeroU32::new(cols).unwrap(),
        rows: NonZeroU32::new(rows).unwrap(),
    }
}
