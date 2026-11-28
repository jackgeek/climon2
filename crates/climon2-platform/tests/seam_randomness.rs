//! The randomness seam's contract suite (ONE-03): every platform's generator
//! passes these by itself, with no link code and no second machine.

use climon2_platform::randomness;
use std::collections::HashSet;

fn random(length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    randomness::fill(&mut bytes).expect("the operating system's generator answers");
    bytes
}

#[test]
fn every_fill_is_different() {
    let keys: HashSet<Vec<u8>> = (0..64).map(|_| random(32)).collect();

    assert_eq!(keys.len(), 64);
}

#[test]
fn a_fill_writes_the_whole_buffer() {
    // A 32-byte buffer of zeros is left with a zero in any one place about
    // one time in 256, so 64 fills leave one place zero every time with a
    // chance near 2^-512 per place.
    for length in [1, 32, 64, 255, 256, 257, 4096, 70_000] {
        let mut untouched = vec![true; length];
        for _ in 0..64 {
            for (place, byte) in random(length).into_iter().enumerate() {
                untouched[place] &= byte == 0;
            }
        }
        assert!(
            untouched.iter().all(|untouched| !untouched),
            "a {length}-byte fill left a byte unwritten"
        );
    }
}

#[test]
fn an_empty_buffer_is_filled_without_error() {
    randomness::fill(&mut []).expect("nothing to fill");
}

#[test]
fn every_bit_takes_both_values() {
    let (mut ones, mut zeros) = (0u8, 0u8);
    for byte in random(4096) {
        ones |= byte;
        zeros |= !byte;
    }

    assert_eq!((ones, zeros), (0xff, 0xff));
}

#[test]
fn bytes_are_spread_evenly() {
    // 1,048,576 bytes put 4,096 in each of 256 values on average, with a
    // standard deviation near 64. A tenth either way is over six deviations.
    let mut counts = [0u32; 256];
    for byte in random(1 << 20) {
        counts[usize::from(byte)] += 1;
    }

    for (value, count) in counts.into_iter().enumerate() {
        assert!(
            (3686..=4506).contains(&count),
            "{value} came up {count} times"
        );
    }
}
