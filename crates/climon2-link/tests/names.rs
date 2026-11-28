//! #501 (P22, R16): a suggested name is a label, and one from the other end
//! is hostile text. Each side shows, stores and suggests a name only in the
//! form `display_name` gives: no control characters, at most 64 bytes.

use climon2_link::{Hello, Introduction, display_name};

#[test]
fn control_characters_are_removed() {
    assert_eq!(display_name("build\x1b[2Jbox\r\n\x07"), "build[2Jbox");
    assert_eq!(display_name("buildbox"), "buildbox");
    assert_eq!(display_name("\u{9b}31mred"), "31mred");
}

#[test]
fn a_name_is_cut_to_64_bytes_at_a_character_boundary() {
    assert_eq!(display_name(&"a".repeat(64)), "a".repeat(64));
    assert_eq!(display_name(&"a".repeat(65)), "a".repeat(64));
    assert_eq!(
        display_name(&format!("{}é", "a".repeat(63))),
        "a".repeat(63)
    );
    assert_eq!(
        display_name(&format!("{}\x07b", "a".repeat(63))),
        format!("{}b", "a".repeat(63))
    );
}

#[test]
fn every_name_it_gives_can_be_suggested() {
    let long = "名".repeat(40);
    let name = display_name(&long);
    assert!(Introduction::new([0; 32], &name).is_some());
    assert!(Hello::new("1.0.0", &name, 4_096).is_some());
}
