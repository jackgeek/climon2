//! Title emission (plan section 1.8.1): C0, DEL and C1 controls are
//! percent-encoded byte by byte; everything else is kept.

pub(crate) fn osc(title: &str) -> Vec<u8> {
    let mut bytes = b"\x1b]2;".to_vec();
    title.chars().for_each(|c| encode(c, &mut bytes));
    bytes.extend_from_slice(b"\x1b\\");
    bytes
}

fn encode(c: char, bytes: &mut Vec<u8>) {
    let utf8 = c.to_string().into_bytes();
    if c.is_control() {
        utf8.iter()
            .for_each(|byte| bytes.extend_from_slice(format!("%{byte:02X}").as_bytes()));
    } else {
        bytes.extend(utf8);
    }
}
