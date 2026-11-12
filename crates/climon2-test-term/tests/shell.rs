#![cfg(target_os = "linux")]

use climon2_test_term::controlled_bash;

#[test]
fn linux_interactive_profile_uses_system_bash_five() {
    assert!(
        controlled_bash().is_some(),
        "Linux bash 5 profile is required"
    );
}
