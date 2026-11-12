//! Tables that pair each variant with its exact, case-sensitive name.

/// Every table lists every variant, so a lookup by variant always succeeds.
pub(crate) fn name<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(known, _)| *known == value)
        .map(|(_, name)| *name)
        .expect("the table lists every variant")
}

pub(crate) fn named<T: Copy>(table: &[(T, &'static str)], text: &str) -> Option<T> {
    table
        .iter()
        .find(|(_, name)| *name == text)
        .map(|(value, _)| *value)
}
