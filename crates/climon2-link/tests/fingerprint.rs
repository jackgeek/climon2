//! #501 (PAIR-01, P23): the key fingerprint each side shows of the other,
//! for its user to compare with the other machine's screen. The expected
//! values were computed with Python's `hashlib`, which shares no code with
//! the crate: the first 16 bytes of SHA-256 over the 32 key bytes, as eight
//! groups of four lowercase hexadecimal digits.

use climon2_link::PublicKey;

#[test]
fn a_fingerprint_is_the_start_of_the_keys_sha_256_in_groups_of_four() {
    let counting: [u8; 32] = std::array::from_fn(|index| index as u8);
    assert_eq!(
        PublicKey::from_bytes(counting).fingerprint(),
        "630d:cd29:66c4:3366:9112:5448:bbb2:5b4f"
    );
    assert_eq!(
        PublicKey::from_bytes([0xff; 32]).fingerprint(),
        "af96:1376:0f72:635f:bdb4:4a5a:0a63:c39f"
    );
}
