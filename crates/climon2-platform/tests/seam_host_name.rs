//! The host name seam (#501): the name each side of a monitor link suggests
//! for itself, its hostname by default (P22, CM25).
#![cfg(unix)]

use std::process::Command;

#[test]
fn the_host_name_is_the_systems() {
    let expected = Command::new("uname").arg("-n").output().unwrap();
    let expected = String::from_utf8(expected.stdout).unwrap();
    assert_eq!(
        climon2_platform::host_name::get().unwrap(),
        expected.trim_end()
    );
}
