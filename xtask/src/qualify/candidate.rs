//! Release candidates and final tags (#330, docs/for-humans/windows/quality.md
//! "Manual checklist timing"). The Windows manual checklist needs a published
//! candidate installed with `install.ps1`, so a release candidate publishes
//! without it, and a final tag takes the checklist records of the latest
//! candidate of its version, whose source it must equal but for the version
//! stamp and the owner's records under `qualification/`. Pure.

/// Which kind of release a tag publishes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    /// `v<version>-rc.<n>`: a prerelease, published without the manual
    /// checklist.
    Candidate,
    /// Any other tag, and a run with no tag: every case counts.
    Final,
}

impl Stage {
    pub(crate) fn of(tag: &str) -> Self {
        match candidate_number(tag) {
            Some(_) => Self::Candidate,
            None => Self::Final,
        }
    }
}

/// `n` of a tag ending `-rc.<n>`.
fn candidate_number(tag: &str) -> Option<u64> {
    let (_, number) = tag.rsplit_once("-rc.")?;
    let digits = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    digits.then(|| number.parse().ok()).flatten()
}

/// The candidate whose manual checklist records a final tag takes: the
/// published `<tag>-rc.<n>` with the greatest `n`, from `releases`, one tag
/// per line.
pub(crate) fn latest(tag: &str, releases: &str) -> Option<String> {
    let prefix = format!("{tag}-rc.");
    releases
        .lines()
        .map(str::trim)
        .filter(|release| release.starts_with(&prefix))
        .filter_map(|release| Some((candidate_number(release)?, release)))
        .max()
        .map(|(_, release)| release.to_owned())
}

/// Where a final tag's source may differ from its candidate's freely: the
/// owner's dispositions and manual records.
const RECORDS: &str = "qualification/";

/// Whether `path`, at `candidate` (`None` when absent) and at the final tag
/// (`None` when deleted), differs only in the version stamp: the
/// candidate's version replaced by the final one. The owner's records may
/// differ in anything.
pub(crate) fn stamp_only(
    path: &str,
    candidate: Option<&str>,
    released: Option<&str>,
    versions: (&str, &str),
) -> bool {
    if path.starts_with(RECORDS) {
        return true;
    }
    match (candidate, released) {
        (Some(candidate), Some(released)) => candidate.replace(versions.0, versions.1) == released,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a tag ending `-rc.` and digits is a release candidate.
    #[test]
    fn only_rc_tags_are_candidates() {
        assert_eq!(Stage::of("v1.2.0-rc.3"), Stage::Candidate);
        assert_eq!(Stage::of("v1.2.0-rc.12"), Stage::Candidate);
        for tag in ["v1.2.0", "", "v1.2.0-rc.", "v1.2.0-rc.x", "v1.2.0-rc.1a"] {
            assert_eq!(Stage::of(tag), Stage::Final, "{tag}");
        }
    }

    /// The latest candidate is this version's greatest `-rc.<n>`, by
    /// number, not by text; another version's never counts.
    #[test]
    fn the_latest_candidate_is_the_greatest_of_this_version() {
        let releases =
            "v1.2.0-rc.2\n v1.2.0-rc.10 \nv1.2.0-rc.9\nv1.2.1-rc.11\nv1.1.0\nv1.2.0\nv1.2.0-rc.x\n";
        assert_eq!(latest("v1.2.0", releases), Some("v1.2.0-rc.10".to_owned()));
        assert_eq!(latest("v1.3.0", releases), None);
        assert_eq!(latest("v1.2.0", ""), None);
    }

    /// A final tag's file equals its candidate's but for the version stamp;
    /// the owner's records may change freely; an added or deleted file is a
    /// difference.
    #[test]
    fn only_the_version_stamp_and_the_records_may_change() {
        let versions = ("1.2.0-rc.3", "1.2.0");
        let stamped =
            |old: &str, new: &str| stamp_only("Cargo.toml", Some(old), Some(new), versions);
        assert!(stamped(
            "version = \"1.2.0-rc.3\"\n",
            "version = \"1.2.0\"\n"
        ));
        assert!(!stamped(
            "version = \"1.2.0-rc.3\"\n",
            "version = \"1.2.1\"\n"
        ));
        assert!(!stamped("a = 1\n", "a = 2\n"));
        assert!(stamp_only(
            "qualification/manual.json",
            Some("{}"),
            Some("{\"records\": []}"),
            versions
        ));
        assert!(stamp_only(
            "qualification/new.json",
            None,
            Some("x"),
            versions
        ));
        assert!(!stamp_only("src/new.rs", None, Some("x"), versions));
        assert!(!stamp_only("src/old.rs", Some("x"), None, versions));
    }
}
