//! Bounded history under memory pressure (#119; operational limits,
//! "Memory, history and admission"): which session caches give up history,
//! and in what order, when the monitor's tracked memory or its histories
//! together exceed their budgets. History is given up a chunk at a time,
//! oldest first, never the chunk holding a cache's newest page (the model
//! keeps it), and every shortened cache shows its truncation.
use crate::terminal::Terminal;
use std::{collections::BTreeMap, time::Instant};

/// The budgets history is kept within, in bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budgets {
    /// `memory.history_bytes_per_session`.
    pub(crate) per_session: usize,
    /// `memory.history_bytes_total`.
    pub(crate) histories: usize,
    /// `memory.total_bytes`, which `fixed` and every model share.
    pub(crate) total: usize,
}

/// Whether `used` bytes exceed `budget`.
fn exceeds(used: usize, budget: usize) -> bool {
    budget.checked_sub(used).is_none()
}

/// Keeps `terminals`' histories within `budgets`: each one's within the
/// per-session budget, shortening its own; then, while all histories
/// together exceed theirs or `fixed` bytes and every model exceed the
/// total, `caches` yield in the pressure [`order`], each until the pressure
/// is gone or only its newest page is left.
pub(crate) fn relieve<K: Clone + Ord>(
    terminals: &mut BTreeMap<K, Terminal>,
    caches: &[Cache<K>],
    fixed: usize,
    budgets: Budgets,
) {
    for terminal in terminals.values_mut() {
        within(terminal, budgets.per_session);
    }
    for key in order(caches) {
        yielded(terminals, &key, fixed, budgets);
    }
}

/// Shortens `terminal`'s history until it is within its own `budget` or
/// only its newest page is left.
fn within(terminal: &mut Terminal, budget: usize) {
    while exceeds(terminal.history_bytes(), budget) && terminal.shorten() {}
}

/// `key`'s cache yields history while the aggregate budgets are
/// [`pressed`], until the pressure is gone or only its newest page is
/// left.
fn yielded<K: Ord>(terminals: &mut BTreeMap<K, Terminal>, key: &K, fixed: usize, budgets: Budgets) {
    while pressed(terminals, fixed, budgets)
        && terminals.get_mut(key).is_some_and(Terminal::shorten)
    {}
}

/// Whether the histories together, or `fixed` bytes and every model,
/// exceed their budgets.
fn pressed<K>(terminals: &BTreeMap<K, Terminal>, fixed: usize, budgets: Budgets) -> bool {
    let histories = terminals.values().map(Terminal::history_bytes).sum();
    let models: usize = terminals.values().map(Terminal::footprint).sum();
    exceeds(histories, budgets.histories) || exceeds(fixed + models, budgets.total)
}

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

    /// A terminal whose model, at 10x3 with up to 3,000 history rows,
    /// holds 2,500 lines: three history chunks.
    fn deep() -> Terminal {
        use crate::terminal::{Authority, Presentation};
        use climon2_vt::{Geometry, Limits, Parser};
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(Limits {
            bytes: 64 << 20,
            history_rows: 3000,
        });
        let held = Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = Geometry::for_host(10, 3).unwrap();
        let lines: String = (1..=2500).map(|line| format!("{line}\r\n")).collect();
        for presentation in [
            Presentation::Granted(held, geometry),
            Presentation::Begin(held, geometry),
            Presentation::Display(lines.into()),
            Presentation::Complete,
        ] {
            terminal.present(&mut parser, presentation);
        }
        terminal
    }

    /// The history bytes of a [`deep`] terminal, shortened `chunks` times.
    fn shortened(chunks: usize) -> usize {
        let mut terminal = deep();
        for _ in 0..chunks {
            assert!(terminal.shorten());
        }
        terminal.history_bytes()
    }

    /// LIM-03 (pure supplement): a budget is exceeded one byte past it,
    /// not at it.
    #[test]
    fn a_budget_is_exceeded_only_past_it() {
        assert!(!exceeds(10, 10));
        assert!(exceeds(11, 10));
        assert!(!exceeds(0, 0));
    }

    /// LIM-03 (pure supplement, bounded history): at the per-session budget
    /// a history keeps its chunks, one byte under it gives one up; with
    /// histories together one byte over their budget, the unselected cache
    /// never inspected yields first, one chunk, and the connected one
    /// nothing; and with the total one byte short of `fixed` and every
    /// model, the next cache in order yields. No cache gives up its newest
    /// page however far over the budgets go.
    #[test]
    fn caches_yield_in_order_until_the_budgets_hold() {
        let (full, less, least) = (shortened(0), shortened(1), shortened(2));
        assert!(full > less && less > least, "{full} {less} {least}");
        let roomy = Budgets {
            per_session: usize::MAX,
            histories: usize::MAX,
            total: usize::MAX,
        };
        let caches = [
            cache('a', true, false, Some(1)),
            cache('b', true, false, None),
            cache('c', false, false, None),
        ];
        let three = || BTreeMap::from([('a', deep()), ('b', deep()), ('c', deep())]);
        let held = |terminals: &BTreeMap<char, Terminal>| {
            terminals
                .values()
                .map(Terminal::history_bytes)
                .collect::<Vec<_>>()
        };

        let mut terminals = three();
        let per_session = Budgets {
            per_session: full,
            ..roomy
        };
        relieve(&mut terminals, &caches, 0, per_session);
        assert_eq!(held(&terminals), [full, full, full]);
        let per_session = Budgets {
            per_session: full - 1,
            ..roomy
        };
        relieve(&mut terminals, &caches, 0, per_session);
        assert_eq!(held(&terminals), [less, less, less]);

        let mut terminals = three();
        let histories = Budgets {
            histories: 3 * full - 1,
            ..roomy
        };
        relieve(&mut terminals, &caches, 0, histories);
        assert_eq!(held(&terminals), [full, less, full]);
        relieve(
            &mut terminals,
            &caches,
            0,
            Budgets {
                histories: 0,
                ..roomy
            },
        );
        assert_eq!(held(&terminals), [least, least, full]);

        let mut terminals = three();
        let models: usize = terminals.values().map(Terminal::footprint).sum();
        let total = Budgets {
            total: 5 + models - 1,
            ..roomy
        };
        relieve(&mut terminals, &caches, 5, total);
        assert_eq!(held(&terminals), [full, less, full]);
    }
}
