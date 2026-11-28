//! What a coordinated shutdown proved (spec #73, "Lifecycle and exit";
//! #118), and what the monitor's exit makes of it. Each kind of evidence is
//! kept on its own: the daemon's answer to Leave, connection closure, the
//! final client outcome, worker cleanup, handle release, destroy and the
//! owner thread's join. None stands in for another.

use climon2_core::SessionId;

pub(crate) use climon2_embed::instance::{Leave, Terminal};

/// What is wrong with `leave`, if anything.
fn problem(leave: Leave) -> Option<&'static str> {
    match leave {
        Leave::NotRequired | Leave::Confirmed => None,
        Leave::Rejected => Some("Leave rejected"),
        Leave::Failed => Some("Leave failed"),
        Leave::Indeterminate => Some("Leave unconfirmed"),
    }
}

/// What is known of one instance, each kind of evidence on its own.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Evidence {
    /// It had not ended when shutdown began: its connection may have been
    /// active, so its departure is judged.
    pub(crate) current: bool,
    /// Its owner created it and published its handles.
    pub(crate) published: bool,
    pub(crate) terminal: Option<Terminal>,
    /// Run returned, with its workers cleaned up or why not.
    pub(crate) workers: Option<Result<(), String>>,
    /// Its handles are released, or it had none.
    pub(crate) released: bool,
    /// Its owner destroyed it, or why not.
    pub(crate) destroyed: Option<Result<(), String>>,
    /// Its owner thread is joined.
    pub(crate) joined: bool,
}

impl Evidence {
    /// What of its cleanup is unconfirmed: never claimed without proof.
    fn cleanup(&self) -> Vec<String> {
        [
            (!self.released).then(|| "handles not released".to_owned()),
            unconfirmed(
                &self.workers,
                "run not returned",
                "worker cleanup unconfirmed",
            ),
            unconfirmed(&self.destroyed, "not destroyed", "not destroyed"),
            (!self.joined).then(|| "thread not joined".to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// What of its departure is unconfirmed: only a connection that may
    /// have been active when shutdown began is judged.
    fn departure(&self) -> Vec<String> {
        if !(self.current && self.published) {
            return Vec::new();
        }
        let Some(terminal) = &self.terminal else {
            return vec!["no terminal evidence".to_owned()];
        };
        let failed = terminal.failed.as_ref();
        [
            problem(terminal.leave).map(str::to_owned),
            failed.map(|why| format!("client failed: {why}")),
            (!terminal.closed).then(|| "connection closure unconfirmed".to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// What is unconfirmed of one step: `missing` when it never happened,
/// `failed` with why when it failed, nothing when it succeeded.
fn unconfirmed(step: &Option<Result<(), String>>, missing: &str, failed: &str) -> Option<String> {
    match step {
        None => Some(missing.to_owned()),
        Some(Err(why)) => Some(format!("{failed}: {why}")),
        Some(Ok(())) => None,
    }
}

/// What a shutdown proved by its deadline.
#[derive(Debug, Default)]
pub(crate) struct Report {
    /// Each instance owned when shutdown began or since, and each earlier
    /// one whose cleanup is unconfirmed.
    pub(crate) instances: Vec<(SessionId, Evidence)>,
    /// Shared work that had not finished: a release's load, a discovery
    /// pass, the client supervisor itself.
    pub(crate) unfinished: Vec<String>,
}

/// What the exit makes of a shutdown.
#[derive(Debug, Eq, PartialEq)]
pub enum Verdict {
    /// Every current Leave is confirmed or not required, and everything is
    /// cleaned up and joined.
    Clean,
    /// A current departure is unconfirmed, but everything is cleaned up and
    /// joined: the exit fails with this diagnostic.
    Failed(String),
    /// Cleanup is unconfirmed: the whole process ends with this diagnostic
    /// without claiming a join (the fail-stop).
    FailStop(String),
}

impl Report {
    pub(crate) fn verdict(&self) -> Verdict {
        let mut stalled = !self.unfinished.is_empty();
        let mut problems = Vec::new();
        for (id, evidence) in &self.instances {
            let cleanup = evidence.cleanup();
            stalled |= !cleanup.is_empty();
            let all: Vec<_> = evidence.departure().into_iter().chain(cleanup).collect();
            if !all.is_empty() {
                problems.push(format!("session {id}: {}", all.join(", ")));
            }
        }
        problems.extend(
            self.unfinished
                .iter()
                .map(|work| format!("{work} unfinished")),
        );
        if problems.is_empty() {
            return Verdict::Clean;
        }
        let why = format!("monitor shutdown unconfirmed: {}", problems.join("; "));
        [Verdict::Failed, Verdict::FailStop][usize::from(stalled)](why)
    }

    /// Records each instance's evidence, content-free, in the general log.
    pub(crate) fn log(&self) {
        for (id, evidence) in &self.instances {
            let terminal = evidence.terminal.as_ref();
            tracing::info!(
                session = %id,
                leave = ?terminal.map(|terminal| terminal.leave),
                detached = ?terminal.map(|terminal| terminal.detached),
                closed = ?terminal.map(|terminal| terminal.closed),
                failed = ?terminal.map(|terminal| terminal.failed.is_some()),
                workers = ?evidence.workers.as_ref().map(Result::is_ok),
                released = evidence.released,
                destroyed = ?evidence.destroyed.as_ref().map(Result::is_ok),
                joined = evidence.joined,
                "client shutdown evidence"
            );
        }
        for work in &self.unfinished {
            tracing::error!(%work, "shutdown work unfinished");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u8) -> SessionId {
        format!("0d5c0000-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    /// An instance whose connection was current at shutdown, which left
    /// as `leave` says and was cleaned up and joined.
    fn left(leave: Leave, failed: Option<&str>) -> Evidence {
        Evidence {
            current: true,
            published: true,
            terminal: Some(Terminal {
                failed: failed.map(str::to_owned),
                leave,
                detached: false,
                closed: true,
            }),
            workers: Some(Ok(())),
            released: true,
            destroyed: Some(Ok(())),
            joined: true,
        }
    }

    fn report(instances: Vec<Evidence>, unfinished: &[&str]) -> Report {
        Report {
            instances: instances
                .into_iter()
                .enumerate()
                .map(|(index, evidence)| (id(index as u8 + 1), evidence))
                .collect(),
            unfinished: unfinished.iter().map(|work| (*work).to_owned()).collect(),
        }
    }

    /// EXT-01 and EXT-04 (pure supplement): a shutdown is clean when every
    /// current connection's Leave is confirmed or not required and every
    /// instance is cleaned up and joined; an old, cleaned-up loss that left
    /// no terminal evidence at shutdown is not judged on Leave.
    #[test]
    fn confirmed_or_unneeded_leaves_with_complete_cleanup_are_clean() {
        let old = Evidence {
            current: false,
            terminal: None,
            ..left(Leave::Confirmed, None)
        };
        let unopened = Evidence {
            published: false,
            terminal: None,
            ..left(Leave::Confirmed, None)
        };
        let clean = report(
            vec![
                left(Leave::Confirmed, None),
                left(Leave::NotRequired, None),
                old,
                unopened,
            ],
            &[],
        );
        assert_eq!(clean.verdict(), Verdict::Clean);
        assert_eq!(report(Vec::new(), &[]).verdict(), Verdict::Clean);
    }

    /// EXT-02 and EXT-04 (pure supplement): a current connection whose
    /// Leave was rejected, failed or lost, whose client failed, whose
    /// closure is unconfirmed or that ended with no terminal evidence
    /// fails the exit, naming the session and what is unconfirmed, while
    /// its cleanup is confirmed: no fail-stop.
    #[test]
    fn an_unconfirmed_current_leave_fails_the_exit_naming_it() {
        let cases = [
            (left(Leave::Rejected, None), "Leave rejected"),
            (left(Leave::Failed, None), "Leave failed"),
            (
                left(Leave::Indeterminate, Some("timeout: no answer")),
                "Leave unconfirmed, client failed: timeout: no answer",
            ),
            (
                left(Leave::NotRequired, Some("io_error: reset")),
                "client failed: io_error: reset",
            ),
            (
                Evidence {
                    terminal: None,
                    ..left(Leave::Confirmed, None)
                },
                "no terminal evidence",
            ),
        ];
        for (evidence, why) in cases {
            assert_eq!(
                report(vec![evidence], &[]).verdict(),
                Verdict::Failed(format!(
                    "monitor shutdown unconfirmed: session {}: {why}",
                    id(1)
                )),
            );
        }
        let mut open = left(Leave::Confirmed, None);
        open.terminal.as_mut().unwrap().closed = false;
        assert_eq!(
            report(vec![open], &[]).verdict(),
            Verdict::Failed(format!(
                "monitor shutdown unconfirmed: session {}: connection closure unconfirmed",
                id(1)
            )),
        );
    }

    /// EXT-02 and LIB-06 (pure supplement): any unconfirmed cleanup, of a
    /// current instance or an old one, and any unfinished shared work, is a
    /// fail-stop that names each piece: never a claimed join or release.
    #[test]
    fn unconfirmed_cleanup_or_unfinished_work_is_a_fail_stop_naming_each_piece() {
        let stalled = Evidence {
            current: true,
            published: true,
            terminal: None,
            workers: None,
            released: false,
            destroyed: None,
            joined: false,
        };
        let failed_destroy = Evidence {
            current: false,
            workers: Some(Err("worker cleanup exceeded cleanup_ms".into())),
            destroyed: Some(Err("busy: still in use".into())),
            ..left(Leave::Confirmed, None)
        };
        let verdict = report(
            vec![stalled, failed_destroy, left(Leave::Confirmed, None)],
            &["release 0.1.0 load", "discovery pass"],
        )
        .verdict();
        assert_eq!(
            verdict,
            Verdict::FailStop(format!(
                "monitor shutdown unconfirmed: session {}: no terminal evidence, \
                 handles not released, run not returned, not destroyed, thread not joined; \
                 session {}: worker cleanup unconfirmed: worker cleanup exceeded cleanup_ms, \
                 not destroyed: busy: still in use; \
                 release 0.1.0 load unfinished; discovery pass unfinished",
                id(1),
                id(2)
            ))
        );
        assert_eq!(
            report(Vec::new(), &["client supervisor"]).verdict(),
            Verdict::FailStop(
                "monitor shutdown unconfirmed: client supervisor unfinished".to_owned()
            )
        );
    }
}
