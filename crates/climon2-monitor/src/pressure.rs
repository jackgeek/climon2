//! Bounded history under memory pressure (#119; operational limits,
//! "Memory, history and admission"): which session caches give up history,
//! and in what order, when the monitor's tracked memory or its histories
//! together exceed their budgets. History is given up a chunk at a time,
//! oldest first, never the chunk holding a cache's newest page (the model
//! keeps it), and every shortened cache shows its truncation.
use std::time::Instant;

/// One session's cache as the pressure policy sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Cache<K> {
    pub(crate) key: K,
    /// Disconnected or final: only such caches yield to other sessions.
    pub(crate) retained: bool,
    /// The selected session's: inspected now, so it yields last.
    pub(crate) selected: bool,
    /// When it was last inspected (shown selected), if ever.
    pub(crate) inspected: Option<Instant>,
}

/// The caches that yield history under aggregate pressure, in the order
/// they yield: the unselected disconnected or final ones, least recently
/// inspected first (never inspected before any), then the selected one if
/// it is disconnected or final. A connected session's history is bounded
/// only by its own budgets.
pub(crate) fn order<K: Clone + Ord>(caches: &[Cache<K>]) -> Vec<K> {
    let mut eligible: Vec<_> = caches.iter().filter(|cache| cache.retained).collect();
    eligible.sort_by_key(|cache| (cache.selected, cache.inspected, cache.key.clone()));
    eligible
        .into_iter()
        .map(|cache| cache.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cache(key: char, retained: bool, selected: bool, inspected: Option<u64>) -> Cache<char> {
        let start = Instant::now();
        Cache {
            key,
            retained,
            selected,
            inspected: inspected.map(|seconds| start + Duration::from_secs(seconds)),
        }
    }

    /// LIM-03 (pure supplement, pressure order): connected caches never
    /// yield to others; unselected disconnected or final ones yield first,
    /// never inspected before least recently inspected, ties by key; the
    /// selected one, though inspected longest ago, yields last.
    #[test]
    fn unselected_retained_caches_yield_first_least_recently_inspected_first() {
        let caches = [
            cache('a', true, false, Some(5)),
            cache('b', false, false, None),
            cache('c', true, true, Some(1)),
            cache('d', true, false, Some(2)),
            cache('e', true, false, None),
            cache('f', true, false, None),
            cache('g', false, true, Some(0)),
        ];
        assert_eq!(order(&caches), ['e', 'f', 'd', 'a', 'c']);
        assert_eq!(order::<char>(&[]), Vec::<char>::new());
    }
}
