//! Helpers shared by core's integration tests; each test uses a subset.
#![allow(dead_code)]

use climon2_core::wire::{AuthorityIds, Counter, Families, FrameLimit, Geometry};
use std::{
    fs,
    io::{self, Read, Write},
    num::NonZeroU32,
    path::{Path, PathBuf},
};

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

/// The directory of the daemon wire pages.
fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/protocol/daemon-client")
}

/// The file name of every daemon wire page.
pub fn pages() -> Vec<String> {
    fs::read_dir(directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect()
}

fn read_page(page: &str) -> String {
    fs::read_to_string(directory().join(page)).unwrap()
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

pub fn ids(period: u64, restoration_id: u64) -> AuthorityIds {
    AuthorityIds {
        period: counter(period),
        restoration_id: counter(restoration_id),
    }
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

pub fn limit(bytes: u32) -> FrameLimit {
    FrameLimit::new(bytes).unwrap()
}

/// Moves at most one byte per read or write, as a slow transport may.
pub struct Fragmented<T>(pub T);

impl<T: Read> Read for Fragmented<T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(1);
        self.0.read(&mut buffer[..length])
    }
}

impl<T: Write> Write for Fragmented<T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(&buffer[..buffer.len().min(1)])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
