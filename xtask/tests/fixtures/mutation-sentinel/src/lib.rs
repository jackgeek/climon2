//! The sentinel of `cargo xtask mutation-probe`: its one test checks `checked`
//! and runs `unchecked` without checking it, so a working mutator kills the
//! mutant of the first and the mutant of the second survives.

pub fn checked(value: u32) -> u32 {
    value + value
}

pub fn unchecked(value: u32) -> u32 {
    value + value
}
