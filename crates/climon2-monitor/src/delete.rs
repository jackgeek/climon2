//! Where a confirmed Delete leaves the selection (selector document,
//! "Delete: terminate and remove"; #117): pure. Its confirmation is its
//! consumer's.
use crate::machine::SessionAt;

/// After a confirmed Delete removed `id` from `before`, leaving `after`:
/// the row now at its place, else the one before it, else none (selector
/// document, "Delete: terminate and remove").
pub fn fallback(id: &SessionAt, before: &[&SessionAt], after: &[&SessionAt]) -> Option<SessionAt> {
    let index = before.iter().position(|row| *row == id)?;
    let fallen = after.get(index).or(after.last());
    fallen.map(|row| (*row).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DEL-02 (pure supplement, selector document): a removed row's place
    /// goes to the row now there, else, when it was last, to the one before
    /// it, else to none; a row that was never there has no fallback.
    #[test]
    fn a_removed_row_falls_back_to_its_place_then_the_row_before() {
        let [a, b, c]: [SessionAt; 3] = [1, 2, 3].map(|suffix| {
            let id = format!("00000000-0000-4000-8000-0000000000{suffix:02x}");
            SessionAt::local(id.parse().unwrap())
        });
        assert_eq!(fallback(&b, &[&a, &b, &c], &[&a, &c]), Some(c.clone()));
        assert_eq!(fallback(&c, &[&a, &b, &c], &[&a, &b]), Some(b.clone()));
        assert_eq!(fallback(&a, &[&a], &[]), None);
        assert_eq!(fallback(&a, &[&b, &c], &[&b, &c]), None);
    }
}
