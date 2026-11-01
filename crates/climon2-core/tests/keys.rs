//! Plan 1.3/1.4: the exact client key vocabulary and the settings snapshot rules.
use climon2_core::keys::*;

const NAMES: [(&str, u8); 32] = [
    ("Ctrl+Space", 0x00),
    ("Ctrl+A", 0x01),
    ("Ctrl+B", 0x02),
    ("Ctrl+C", 0x03),
    ("Ctrl+D", 0x04),
    ("Ctrl+E", 0x05),
    ("Ctrl+F", 0x06),
    ("Ctrl+G", 0x07),
    ("Ctrl+H", 0x08),
    ("Ctrl+I", 0x09),
    ("Ctrl+J", 0x0A),
    ("Ctrl+K", 0x0B),
    ("Ctrl+L", 0x0C),
    ("Ctrl+M", 0x0D),
    ("Ctrl+N", 0x0E),
    ("Ctrl+O", 0x0F),
    ("Ctrl+P", 0x10),
    ("Ctrl+Q", 0x11),
    ("Ctrl+R", 0x12),
    ("Ctrl+S", 0x13),
    ("Ctrl+T", 0x14),
    ("Ctrl+U", 0x15),
    ("Ctrl+V", 0x16),
    ("Ctrl+W", 0x17),
    ("Ctrl+X", 0x18),
    ("Ctrl+Y", 0x19),
    ("Ctrl+Z", 0x1A),
    ("Ctrl+[", 0x1B),
    ("Ctrl+\\", 0x1C),
    ("Ctrl+]", 0x1D),
    ("Ctrl+^", 0x1E),
    ("Ctrl+_", 0x1F),
];

fn key(byte: u8) -> ControlKey {
    ControlKey::from_byte(byte).unwrap()
}

#[test]
fn every_vocabulary_name_maps_to_its_control_byte_and_back_by_byte() {
    for (name, byte) in NAMES {
        let parsed = ControlKey::from_name(name).unwrap();
        assert_eq!(parsed.byte(), byte, "{name}");
        assert_eq!(ControlKey::from_byte(byte), Some(parsed), "{name}");
        assert_eq!(parsed.to_string(), name);
    }
}

#[test]
fn only_the_exact_spelling_of_single_control_keys_is_accepted() {
    for bad in [
        "",
        "Ctrl+",
        "Ctrl+a",
        "ctrl+A",
        "CTRL+A",
        "Ctrl-A",
        "Ctrl+@",
        "Ctrl+`",
        "Ctrl+{",
        "Ctrl+?",
        "Ctrl+1",
        "Ctrl+space",
        "Ctrl+SPACE",
        "Ctrl+ ",
        "Ctrl+AB",
        "Ctrl+A+B",
        " Ctrl+A",
        "Ctrl+A ",
        "Alt+A",
        "Ctrl+Alt+A",
        "A",
        "Space",
        "F1",
        "Ctrl+F1",
        "Ctrl+é",
    ] {
        assert_eq!(ControlKey::from_name(bad), None, "{bad:?}");
    }
    for byte in [0x20, 0x41, 0x7F, 0x80, 0xFF] {
        assert_eq!(ControlKey::from_byte(byte), None, "{byte}");
    }
}

#[test]
fn defaults_are_ctrl_right_bracket_and_ctrl_t() {
    let settings = ClientSettings::default();
    assert_eq!(settings.escape_prefix(), Some(key(0x1D)));
    assert_eq!(settings.take_control_key(), Some(key(0x14)));
}

#[test]
fn settings_keep_each_binding_or_its_absence_exactly() {
    for (escape, take) in [
        (None, None),
        (Some(key(0x1B)), Some(key(0x14))),
        (Some(key(0x00)), None),
        (None, Some(key(0x1F))),
        (Some(key(0x02)), Some(key(0x01))),
    ] {
        let settings = ClientSettings::new(escape, take).unwrap();
        assert_eq!(
            (settings.escape_prefix(), settings.take_control_key()),
            (escape, take)
        );
    }
}

#[test]
fn take_control_may_not_be_escape_and_enabled_bindings_may_not_share_a_byte() {
    for escape in [None, Some(key(0x1D)), Some(key(0x1B))] {
        let error = ClientSettings::new(escape, Some(key(0x1B))).unwrap_err();
        assert_eq!(error, SettingsError::TakeControlIsEscape);
        assert_eq!(error.to_string(), "take_control_key may not be Ctrl+[");
    }
    for byte in [0x00, 0x14, 0x1D] {
        let error = ClientSettings::new(Some(key(byte)), Some(key(byte))).unwrap_err();
        assert_eq!(error, SettingsError::Conflict);
        assert_eq!(
            error.to_string(),
            "escape_prefix and take_control_key are the same key"
        );
    }
}
