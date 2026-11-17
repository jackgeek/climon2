//! The session's retained progress: OSC 9;4 reports normalized into the
//! current `progress-v1` value, after the agreed
//! [source normalization](../../../../docs/protocol/daemon-client/core-v1.md#source-normalization).
//! Progress never requests attention.
use climon2_core::wire::{Percent, Progress, ProgressState};

/// One valid OSC 9;4 report, `ESC ] 9 ; 4 ; state ; percent ST`: the state
/// its number names (0 none, 1 normal, 2 error, 3 indeterminate, 4 paused;
/// none when missing) and the percentage it supplied, if any. A `normal`
/// report always supplies one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Report {
    pub state: ProgressState,
    pub percent: Option<Percent>,
}

/// The states in the order OSC 9;4 numbers them.
const STATES: [ProgressState; 5] = [
    ProgressState::None,
    ProgressState::Normal,
    ProgressState::Error,
    ProgressState::Indeterminate,
    ProgressState::Paused,
];

/// The parameters after `9;4`, if they are a valid report: an optional
/// state from 0 to 4, then an optional percentage from 0 to 100, as
/// decimal digits, and nothing more. Anything else, a `normal` report
/// without its percentage included, leaves the indicator unchanged.
pub fn report(parameters: &[u8]) -> Option<Report> {
    let (state, percent) = fields(parameters)?;
    let report = Report {
        state: progress_state(state)?,
        percent: percentage(percent)?,
    };
    Some(report).filter(|report| report.state != ProgressState::Normal || report.percent.is_some())
}

/// A report's state and percentage fields, each `None` when empty or
/// missing.
type Fields<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

/// The state and percentage fields, if there are no more.
fn fields(parameters: &[u8]) -> Option<Fields<'_>> {
    let mut fields = parameters
        .split(|&byte| byte == b';')
        .map(|field| (!field.is_empty()).then_some(field));
    let fields_given = (fields.next().flatten(), fields.next().flatten());
    fields.next().is_none().then_some(fields_given)
}

/// The state a field numbers; none when missing.
fn progress_state(field: Option<&[u8]>) -> Option<ProgressState> {
    STATES
        .get(usize::from(field.map_or(Some(0), number)?))
        .copied()
}

/// The percentage a field gives, if any: `Some(None)` when missing, `None`
/// when invalid.
fn percentage(field: Option<&[u8]>) -> Option<Option<Percent>> {
    field.map_or(Some(None), |digits| {
        number(digits).and_then(Percent::new).map(Some)
    })
}

/// Decimal digits only, at most 255.
fn number(digits: &[u8]) -> Option<u8> {
    let decimal = digits.iter().all(u8::is_ascii_digit);
    std::str::from_utf8(digits)
        .ok()
        .filter(|_| decimal)?
        .parse()
        .ok()
}

/// The current normalized progress and the last number a report supplied.
pub struct Tracker {
    current: Progress,
    cached: Option<Percent>,
}

impl Default for Tracker {
    /// No progress, and no number cached.
    fn default() -> Self {
        Self {
            current: Progress::new(ProgressState::None, None).expect("none has no percent"),
            cached: None,
        }
    }
}

impl Tracker {
    pub fn current(&self) -> Progress {
        self.current
    }

    /// Applies `report`: none resets the cached number and indeterminate
    /// ignores a supplied one; any number supplied becomes the cached one,
    /// and paused without one reports it. Error without one is unknown.
    /// Returns the new value only when it changed.
    pub fn report(&mut self, Report { state, percent }: Report) -> Option<Progress> {
        let percent = match state {
            ProgressState::None => {
                self.cached = None;
                None
            }
            ProgressState::Indeterminate => None,
            ProgressState::Paused => percent.or(self.cached),
            ProgressState::Normal | ProgressState::Error => percent,
        };
        self.cached = percent.or(self.cached);
        let next =
            Progress::new(state, percent).expect("normalization keeps percent and state agreeing");
        (next != self.current).then(|| {
            self.current = next;
            next
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Report, Tracker, report};
    use ProgressState::{Error, Indeterminate, None as Cleared, Normal, Paused};
    use climon2_core::wire::{Percent, Progress, ProgressState};

    fn percent(value: u8) -> Percent {
        Percent::new(value).unwrap()
    }

    fn reported(state: ProgressState, value: Option<u8>) -> Report {
        Report {
            state,
            percent: value.map(percent),
        }
    }

    fn progress(state: ProgressState, value: Option<u8>) -> Progress {
        Progress::new(state, value.map(percent)).unwrap()
    }

    /// STA-06 (pure, OSC 9;4 syntax): states 0 to 4 with or without a
    /// percentage from 0 to 100, each in decimal digits that may have
    /// leading zeros; no state at all is none; `normal` needs
    /// its percentage, while none and indeterminate accept one.
    #[test]
    fn valid_progress_reports() {
        for (parameters, state, value) in [
            ("", Cleared, None),
            (";", Cleared, None),
            ("0", Cleared, None),
            ("0;5", Cleared, Some(5)),
            ("1;0", Normal, Some(0)),
            ("1;100", Normal, Some(100)),
            ("1;050", Normal, Some(50)),
            ("01;5", Normal, Some(5)),
            ("2", Error, None),
            ("2;", Error, None),
            ("2;7", Error, Some(7)),
            ("3", Indeterminate, None),
            ("3;40", Indeterminate, Some(40)),
            ("4", Paused, None),
            ("4;9", Paused, Some(9)),
        ] {
            let expected = reported(state, value);
            assert_eq!(
                report(parameters.as_bytes()),
                Some(expected),
                "{parameters:?}"
            );
        }
    }

    /// STA-06 (pure, invalid reports): an unknown state, `normal` without
    /// a percentage, a percentage past 100, signs, spaces and other bytes,
    /// and extra parameters are ignored.
    #[test]
    fn invalid_progress_reports_are_ignored() {
        for parameters in [
            "5",
            "255",
            "256",
            "9;1",
            "1",
            "1;",
            "1;101",
            "1;256",
            "1;+5",
            "1; 5",
            "+1;5",
            "-1",
            "1;5;0",
            "1;5;",
            "x",
            "2;x",
            "1;\u{0665}",
        ] {
            assert_eq!(report(parameters.as_bytes()), None, "{parameters:?}");
        }
    }

    /// STA-06 (pure, normalization): the indicator starts at none. A
    /// number updates the cache; indeterminate ignores a supplied number
    /// and keeps the cache; error without a number is unknown, not zero;
    /// paused without one reports the cached number, or unknown once none
    /// reset the cache. An unchanged value reports nothing, and 100 is only
    /// a value.
    #[test]
    fn progress_normalizes_against_the_cached_number() {
        let mut tracker = Tracker::default();
        assert_eq!(tracker.current(), progress(Cleared, None));
        let steps = [
            ((Normal, Some(30)), Some(progress(Normal, Some(30)))),
            ((Normal, Some(30)), None),
            (
                (Indeterminate, Some(80)),
                Some(progress(Indeterminate, None)),
            ),
            ((Paused, None), Some(progress(Paused, Some(30)))),
            ((Error, None), Some(progress(Error, None))),
            ((Paused, None), Some(progress(Paused, Some(30)))),
            ((Error, Some(60)), Some(progress(Error, Some(60)))),
            ((Paused, None), Some(progress(Paused, Some(60)))),
            ((Paused, Some(70)), Some(progress(Paused, Some(70)))),
            ((Normal, Some(100)), Some(progress(Normal, Some(100)))),
            ((Paused, None), Some(progress(Paused, Some(100)))),
            ((Cleared, Some(5)), Some(progress(Cleared, None))),
            ((Cleared, None), None),
            ((Paused, None), Some(progress(Paused, None))),
            ((Indeterminate, None), Some(progress(Indeterminate, None))),
            ((Error, None), Some(progress(Error, None))),
        ];
        for (index, ((state, value), expected)) in steps.into_iter().enumerate() {
            assert_eq!(
                tracker.report(reported(state, value)),
                expected,
                "step {index}"
            );
            assert_eq!(
                tracker.current(),
                expected.unwrap_or(tracker.current()),
                "step {index}"
            );
        }
    }
}
