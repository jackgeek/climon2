//! Which shipped files a gate mutates (#452): those a push changed, all of them, or one
//! slice of the rolling tenth that main mutates on each squash-merge.

use std::path::Path;

use sha2::{Digest, Sha256};

/// The rolling pass's slices: every shipped file is in exactly one, so ten passes in a row
/// mutate every file once.
pub(crate) const SLICES: u8 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Selection {
    /// What the push changed, since `CLIMON2_MUTATION_BASE`.
    Changed,
    /// Every shipped file, whole (`--mutate-all`).
    All,
    /// The whole shipped files of one slice (`--mutate-slice <k>`).
    Slice(Slice),
}

impl Selection {
    /// Whether whole shipped files are chosen without looking at changes.
    pub(crate) fn whole_files(self) -> bool {
        self != Self::Changed
    }

    /// Whether `path`, relative to the workspace root, is one this selection mutates whole.
    pub(crate) fn keeps(self, path: &Path) -> bool {
        match self {
            Self::Changed => false,
            Self::All => true,
            Self::Slice(slice) => Slice::of(path) == slice,
        }
    }

    /// Whether CRAP scores `path`: every source the gate scores, except that a slice scores
    /// only its own files, so its findings are its own.
    pub(crate) fn scores(self, path: &Path) -> bool {
        !matches!(self, Self::Slice(_)) || self.keeps(path)
    }
}

/// One of the `SLICES` slices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Slice(u8);

impl Slice {
    /// The slice `text` names, from 0 to `SLICES - 1`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        if text.len() != 1 {
            return None;
        }
        text.parse::<u8>()
            .ok()
            .filter(|&index| index < SLICES)
            .map(Self)
    }

    /// The slice a file is in, from a hash of its path with `/` separators. A file keeps its
    /// slice whatever other files are added or removed, so no file misses its turn.
    pub(crate) fn of(path: &Path) -> Self {
        let text = path.to_string_lossy().replace('\\', "/");
        let digest = Sha256::digest(text.as_bytes());
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&digest[..8]);
        Self((u64::from_be_bytes(bytes) % u64::from(SLICES)) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slice_is_a_single_digit_below_ten() {
        assert_eq!(Slice::parse("0"), Some(Slice(0)));
        assert_eq!(Slice::parse("9"), Some(Slice(9)));
        for text in ["10", "", "-1", "+1", "a", "01", " 1"] {
            assert_eq!(Slice::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_files_slice_comes_from_its_path_alone() {
        // Fixed by SHA-256, so the rotation is the same on every platform and run.
        let path = Path::new("crates/climon2-core/src/lib.rs");
        assert_eq!(Slice::of(path), Slice::of(path));
        assert_eq!(
            Slice::of(Path::new("crates\\climon2-core\\src\\lib.rs")),
            Slice::of(path)
        );
        assert_eq!(Slice::of(Path::new("a.rs")), Slice(SLICE_OF_A_RS));
        assert_eq!(Slice::of(Path::new("b.rs")), Slice(SLICE_OF_B_RS));
    }

    // sha256("a.rs") and sha256("b.rs"): their first eight bytes, big-endian, mod 10.
    const SLICE_OF_A_RS: u8 = 6;
    const SLICE_OF_B_RS: u8 = 5;

    #[test]
    fn ten_slices_cover_every_file_exactly_once() {
        let paths: Vec<_> = (0..500)
            .map(|index| format!("crates/c{index}/src/lib.rs"))
            .collect();
        let mut counts = [0usize; SLICES as usize];
        for path in &paths {
            let path = Path::new(path);
            let kept: Vec<_> = (0..SLICES)
                .filter(|&index| Selection::Slice(Slice(index)).keeps(path))
                .collect();
            assert_eq!(kept.len(), 1, "{}", path.display());
            counts[usize::from(kept[0])] += 1;
            assert!(Selection::All.keeps(path));
            assert!(!Selection::Changed.keeps(path));
        }
        // Roughly even: no slice holds more than twice its share.
        assert!(
            counts.iter().all(|&count| count > 0 && count < 100),
            "{counts:?}"
        );
    }

    #[test]
    fn a_slice_scores_only_its_own_files() {
        let path = Path::new("a.rs");
        assert!(Selection::Changed.scores(path));
        assert!(Selection::All.scores(path));
        assert!(Selection::Slice(Slice(SLICE_OF_A_RS)).scores(path));
        assert!(!Selection::Slice(Slice(SLICE_OF_B_RS)).scores(path));
    }

    #[test]
    fn only_a_changed_selection_looks_at_changes() {
        assert!(!Selection::Changed.whole_files());
        assert!(Selection::All.whole_files());
        assert!(Selection::Slice(Slice(3)).whole_files());
    }
}
