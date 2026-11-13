//! GUIDs cross the C ABI as 16 bytes in canonical textual order
//! (`00112233-4455-6677-8899-aabbccddeeff` is `00 11 22 ... ff`), without
//! host-endian field swapping; metadata, hello and the wire name them in
//! that canonical lowercase hyphenated text.

/// The canonical text of a GUID's bytes.
pub(super) fn text(bytes: [u8; 16]) -> String {
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The bytes of a GUID the wire codec has already validated as canonical.
pub(super) fn bytes(text: &str) -> [u8; 16] {
    let hex: String = text.chars().filter(|character| *character != '-').collect();
    u128::from_str_radix(&hex, 16)
        .unwrap_or_default()
        .to_be_bytes()
}
