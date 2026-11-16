//! Drives the exact candidate archive through the matrix: for each case, N
//! sessions of the workload, measured once under one `climon2 monitor` and
//! once under N `climon2 attach` clients in N terminals (the owner's
//! baseline, #121). Every wait is bounded; a probe that outlives its bound
//! is recorded as a timeout, never retried into a sample.

pub mod candidate;
pub mod sampler;

use crate::{
    args,
    evaluate::{Latencies, Probe, Requirement},
    matrix::{Case, Population, Releases, Scenario, matrix},
    report::{Environment, Mode, Outcome, Qualification, Record, Report, Run, outcome},
    sample,
};
use candidate::{Candidate, Session, User};
use climon2_platform::library::Loader;
use climon2_test_term::{HostSignal, Options, Terminal};
use sampler::{Baseline, Sampler, Watched};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

/// The default escape prefix, Ctrl+].
const PREFIX: &str = "\x1d";
/// Escape, then a focus report that ends its classification at once.
const ESCAPE: &str = "\x1b\x1b[I";
/// The host leaving the alternate screen: the monitor releasing it.
const ALTERNATE_OFF: &[u8] = b"\x1b[?1049l";
/// The selector's top-left corner.
const SELECTOR: &[u8] = "╔".as_bytes();
/// The hint every attention toast's text holds with the default prefix.
const TOAST_HINT: &str = "jump to attention";
/// A bound for starting, readiness and exit.
const SETUP: Duration = Duration::from_secs(60);
/// A probe that takes longer than this is a timeout.
const PROBE: Duration = Duration::from_secs(5);
/// Readiness ends once no further client connects for this long.
const SETTLE: Duration = Duration::from_secs(5);
/// A STA-02 sampler probe's bound: ten quiet seconds and the sampling.
const SAMPLER: Duration = Duration::from_secs(30);
/// How long a reconnect storm stalls every daemon.
const STORM: Duration = Duration::from_secs(3);
/// A sufficient configuration, disclosed, for exact populations the
/// defaults cannot admit (an owner question on #121).
fn sufficient() -> Value {
    json!({ "memory": { "total_bytes": 2_147_483_648u64 } })
}

/// Whether `child` exited successfully by `deadline`; killed and reaped if
/// not.
pub(crate) fn reaped(mut child: std::process::Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Runs `command` to completion within `bound`, or kills and reaps it.
pub(crate) fn bounded(mut command: Command, bound: Duration) -> io::Result<Output> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + bound;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(io::Error::other(format!("{command:?} outlived {bound:?}")));
        }
        thread::sleep(Duration::from_millis(5));
    }
    child.wait_with_output()
}

fn options() -> Options {
    Options {
        replies: true,
        history: 0,
        recent: 1 << 20,
    }
}

/// Uppercase probe tokens, unique within a run: no workload output holds one.
#[derive(Default)]
struct Tokens(u32);

impl Tokens {
    fn next(&mut self) -> String {
        self.0 += 1;
        let letter = |shift: u32| char::from(b'A' + ((self.0 / 26u32.pow(shift)) % 26) as u8);
        format!("Q{}{}{}", letter(2), letter(1), letter(0))
    }
}

/// One run's latencies by probe.
#[derive(Default)]
struct Probes(BTreeMap<Probe, Latencies>);

impl Probes {
    fn record(&mut self, probe: Probe, latency: Option<Duration>) {
        let latencies = self.0.entry(probe).or_default();
        match latency {
            Some(latency) => latencies.samples.push(latency),
            None => latencies.timeouts += 1,
        }
    }
}

/// Typed `token` until the terminal paints its echo, then Enter.
fn input(terminal: &mut Terminal, token: &str, bound: Duration) -> Option<Duration> {
    let mark = terminal.received_total();
    let start = Instant::now();
    terminal.send(token).ok()?;
    let echoed = terminal.wait_for_bytes_since(mark, token.as_bytes(), bound);
    let elapsed = start.elapsed();
    let _ = terminal.send("\r");
    echoed.ok().map(|()| elapsed)
}

/// Types until the session answers, since keys typed during a restoration
/// are discarded and nothing on the screen marks its end.
fn ready_for_input(terminal: &mut Terminal, tokens: &mut Tokens) -> io::Result<()> {
    let deadline = Instant::now() + SETUP;
    while Instant::now() < deadline {
        if input(terminal, &tokens.next(), Duration::from_secs(1)).is_some() {
            return Ok(());
        }
    }
    Err(io::Error::other(format!(
        "typed input never echoed:\n{}",
        terminal.screen()
    )))
}

fn bells(signals: &[HostSignal]) -> usize {
    signals
        .iter()
        .filter(|signal| **signal == HostSignal::Bell)
        .count()
}

/// Writes `line` to a workload's FIFO.
fn ask(session: &Session, line: &str) -> io::Result<()> {
    let fifo = session
        .fifo
        .as_ref()
        .ok_or_else(|| io::Error::other("no FIFO"))?;
    let mut writer = fs::OpenOptions::new().write(true).open(fifo)?;
    writer.write_all(format!("{line}\n").as_bytes())
}

/// Stops or resumes daemons.
fn signal(daemons: &[i32], signal: libc::c_int) {
    for &daemon in daemons {
        // SAFETY: the pids are this run's session daemons.
        unsafe { libc::kill(daemon, signal) };
    }
}

/// Polls until `expected` of `daemons` have a client, or none more for
/// [`SETTLE`]: how many connected, and when since `start` (the clients'
/// spawning) the last one did.
fn connected(
    baseline: Baseline,
    daemons: &[i32],
    expected: u32,
    start: Instant,
) -> (u32, Duration) {
    let (mut count, mut last) = (0, Duration::ZERO);
    loop {
        let now = baseline.count(daemons);
        if now > count {
            (count, last) = (now, start.elapsed());
        }
        let elapsed = start.elapsed();
        if count >= expected || elapsed >= SETUP || elapsed >= last + SETTLE {
            return (count, last);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Polls until no client is connected to any of `daemons`.
fn disconnected(baseline: Baseline, daemons: &[i32]) -> io::Result<()> {
    let deadline = Instant::now() + SETUP;
    while baseline.count(daemons) > 0 {
        if Instant::now() >= deadline {
            return Err(io::Error::other("clients stayed connected after exiting"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Sleeps until `at`, if it is still ahead.
fn until(at: Instant) {
    thread::sleep(at.saturating_duration_since(Instant::now()));
}

/// A case's sessions and their user; ended when dropped.
struct Fleet {
    user: User,
    sessions: Vec<Session>,
    /// Newer sessions stalled from the start.
    stalled: Vec<i32>,
    baseline: Baseline,
    notes: Vec<String>,
}

impl Fleet {
    fn live(&self) -> Vec<i32> {
        self.sessions
            .iter()
            .map(|session| session.daemon)
            .filter(|daemon| !self.stalled.contains(daemon))
            .collect()
    }

    fn daemons(&self) -> Vec<i32> {
        self.sessions.iter().map(|session| session.daemon).collect()
    }

    fn titled(&self, title: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.title == title)
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        self.user.end(&self.sessions);
    }
}

pub struct Bench {
    candidate: Candidate,
    measure: PathBuf,
    window: Duration,
}

impl Bench {
    pub fn new(candidate: Candidate, measure: PathBuf, window: Duration) -> Self {
        Self {
            candidate,
            measure,
            window,
        }
    }

    /// The case's sessions: s001 (selected, at the selected rate), the rest
    /// at the background rate, s002 with an attention FIFO for background
    /// output; a heuristic h001 for selected sustained output (STA-02); and
    /// for stalled newer starts, a tenth more newer sessions, stopped.
    fn fleet(&self, case: &Case, limits: Option<&Value>) -> io::Result<Fleet> {
        let user = User::new(&self.candidate, limits)?;
        let mut fleet = Fleet {
            user,
            sessions: Vec::new(),
            stalled: Vec::new(),
            baseline: Baseline(0),
            notes: Vec::new(),
        };
        let rates = case.scenario.rates();
        let offered = case.population.offered();
        for index in 1..=offered {
            let rate = if index == 1 {
                rates.selected
            } else {
                rates.background
            };
            let attention = index == 2 && case.scenario == Scenario::BackgroundOutput;
            let session = fleet.user.start(
                &self.measure,
                &format!("s{index:03}"),
                rate,
                attention,
                false,
            )?;
            fleet.sessions.push(session);
        }
        if case.scenario == Scenario::SustainedOutput {
            let session = fleet.user.start(&self.measure, "h001", 0, true, true)?;
            fleet.sessions.push(session);
            fleet
                .notes
                .push("one more session, h001, is heuristic for the STA-02 sampler probe".into());
        }
        if case.scenario == Scenario::StalledNewerStarts {
            let stalled = (offered / 10).max(1);
            for index in 1..=stalled {
                let session =
                    fleet
                        .user
                        .start(&self.measure, &format!("n{index:03}"), 0, false, false)?;
                fleet.stalled.push(session.daemon);
                fleet.sessions.push(session);
            }
            fleet.notes.push(format!(
                "{stalled} newer session(s) n001.. are stopped (SIGSTOP) from the start"
            ));
        }
        fleet.baseline = Baseline::of(&fleet.daemons());
        signal(&fleet.stalled, libc::SIGSTOP);
        Ok(fleet)
    }

    /// One case: the monitor at the default limits, again with a disclosed
    /// sufficient configuration if the defaults admit fewer than an exact
    /// population, and the standalone clients.
    pub fn case(&self, case: Case) -> (Record, Option<String>) {
        if case.releases == Releases::Mixed {
            let reason = "no ten-release mixed-version fixture exists yet (owner question on #121)";
            return (
                Record {
                    case,
                    runs: Vec::new(),
                    outcome: Outcome::NotRun(reason.to_owned()),
                },
                None,
            );
        }
        let mut runs = Vec::new();
        let result = (|| {
            let defaults = self.run(&case, Mode::Monitor, None)?;
            let short = defaults
                .admitted
                .is_some_and(|admitted| admitted < defaults.offered);
            runs.push(defaults);
            if short && matches!(case.population, Population::Sessions(_)) {
                runs.push(self.run(&case, Mode::Monitor, Some(sufficient()))?);
            }
            runs.push(self.run(&case, Mode::Standalone, None)?);
            io::Result::Ok(())
        })();
        let (outcome, error) = match result {
            Ok(()) => (outcome(&case, &runs), None),
            Err(error) => {
                let error = format!("harness error: {error}");
                (Outcome::NotRun(error.clone()), Some(error))
            }
        };
        (
            Record {
                case,
                runs,
                outcome,
            },
            error,
        )
    }

    fn run(&self, case: &Case, mode: Mode, limits: Option<Value>) -> io::Result<Run> {
        let start = Instant::now();
        let mut fleet = self.fleet(case, limits.as_ref())?;
        eprintln!(
            "climon2-measure: {case} {}: {} sessions started in {:.1} s",
            mode.name(),
            fleet.sessions.len(),
            start.elapsed().as_secs_f64()
        );
        let mut probes = Probes::default();
        let mut measured = match (mode, case.scenario) {
            (Mode::Monitor, Scenario::Loading) => {
                self.monitor_loading(case, &mut fleet, &mut probes)
            }
            (Mode::Standalone, Scenario::Loading) => {
                self.standalone_loading(case, &mut fleet, &mut probes)
            }
            (Mode::Monitor, _) => self.monitor(case, &mut fleet, &mut probes),
            (Mode::Standalone, _) => self.standalone(case, &mut fleet, &mut probes),
        }?;
        measured.notes.append(&mut fleet.notes);
        eprintln!(
            "climon2-measure: {case} {}: measured after {:.1} s",
            mode.name(),
            start.elapsed().as_secs_f64()
        );
        Ok(Run {
            mode,
            configuration: limits,
            offered: case.population.offered(),
            admitted: measured.admitted,
            clients: sample::usage(&measured.sampled.clients),
            daemons: sample::usage(&measured.sampled.daemons),
            components: measured.components,
            latencies: probes.0,
            display_bytes: measured.display_bytes,
            backlog_max: Some(measured.sampled.backlog_max),
            notes: measured.notes,
        })
    }

    fn spawn(&self, case: &Case, fleet: &Fleet, args: &[&str]) -> io::Result<Terminal> {
        let geometry = case.geometry;
        Terminal::spawn_with(
            fleet.user.terminal_command(args),
            geometry.rows,
            geometry.cols,
            options(),
        )
    }

    /// The monitor started, ready, s001 selected and taking input.
    fn monitor(&self, case: &Case, fleet: &mut Fleet, probes: &mut Probes) -> io::Result<Measured> {
        let spawned = Instant::now();
        let mut terminal = self.spawn(case, fleet, &["monitor"])?;
        let live = fleet.live();
        let (admitted, ready) = connected(fleet.baseline, &live, live.len() as u32, spawned);
        probes.record(Probe::Readiness, Some(ready));
        let mut tokens = Tokens::default();
        let silent = case.scenario.rates().selected == 0;
        select(&mut terminal, "s001", silent)?;
        if silent {
            // Under sustained output the echo scrolls away between frames.
            ready_for_input(&mut terminal, &mut tokens)?;
        }
        let sampler = Sampler::start(
            Watched {
                clients: vec![terminal.pid()],
                daemons: fleet.daemons(),
                expected: live.len() as u32,
            },
            fleet.baseline,
        );
        let mark = terminal.received_total();
        let mut notes = Vec::new();
        let start = Instant::now();
        let mut second = 0u32;
        while start.elapsed() < self.window {
            until(start + Duration::from_secs(second.into()));
            self.monitor_step(
                case,
                fleet,
                &sampler,
                &mut terminal,
                probes,
                &mut tokens,
                second,
            )?;
            second += 1;
        }
        if case.scenario == Scenario::SustainedOutput {
            notes.push(
                "input is not probed under sustained scrolling output: its echo scrolls away \
                 between frames; the selector and global actions are"
                    .to_owned(),
            );
        }
        let display_bytes = terminal.received_total() - mark;
        let components = sample::components(terminal.pid());
        probes.record(Probe::ExitStart, exit(&mut terminal));
        Ok(Measured {
            admitted: Some(admitted),
            sampled: sampler.finish(),
            components,
            display_bytes,
            notes,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn monitor_step(
        &self,
        case: &Case,
        fleet: &mut Fleet,
        sampler: &Sampler,
        terminal: &mut Terminal,
        probes: &mut Probes,
        tokens: &mut Tokens,
        second: u32,
    ) -> io::Result<()> {
        match case.scenario {
            Scenario::PassiveIdle | Scenario::Loading => {}
            Scenario::BackgroundOutput => {
                probes.record(Probe::Input, input(terminal, &tokens.next(), PROBE));
                if second % 5 == 1 {
                    selector(terminal, probes);
                }
                if second % 5 == 3
                    && let Some(session) = fleet.titled("s002")
                {
                    attention(terminal, session, probes)?;
                }
            }
            Scenario::SustainedOutput => {
                if second == 0
                    && let Some(session) = fleet.titled("h001")
                {
                    probes.record(Probe::Sampler, status_sampler(terminal, session)?);
                }
                selector(terminal, probes);
            }
            Scenario::Switching => {
                let target = &fleet.sessions[second as usize % fleet.sessions.len()];
                let switched = switch(terminal, &target.title, true, probes);
                probes.record(Probe::Switch, switched);
                if second % 5 == 4 {
                    selector(terminal, probes);
                }
            }
            Scenario::ReconnectStorm => {
                storm_or_input(fleet, terminal, probes, tokens, second, true);
            }
            Scenario::ContinuousArrivals => {
                if second.is_multiple_of(2) {
                    arrive(self, fleet, sampler, probes, second)?;
                }
                probes.record(Probe::Input, input(terminal, &tokens.next(), PROBE));
            }
            Scenario::StalledNewerStarts => {
                probes.record(Probe::Input, input(terminal, &tokens.next(), PROBE));
                if second % 5 == 1 {
                    selector(terminal, probes);
                }
            }
        }
        Ok(())
    }

    /// N attach clients, each on its own terminal, ready; s001's taking
    /// input.
    fn standalone(
        &self,
        case: &Case,
        fleet: &mut Fleet,
        probes: &mut Probes,
    ) -> io::Result<Measured> {
        let spawned = Instant::now();
        let mut terminals = Vec::new();
        for session in &fleet.sessions {
            terminals.push(self.spawn(case, fleet, &["attach", &session.id])?);
        }
        let live = fleet.live();
        let (_, ready) = connected(fleet.baseline, &live, live.len() as u32, spawned);
        probes.record(Probe::Readiness, Some(ready));
        let mut tokens = Tokens::default();
        ready_for_input(&mut terminals[0], &mut tokens)?;
        let sampler = Sampler::start(
            Watched {
                clients: terminals.iter().map(Terminal::pid).collect(),
                daemons: fleet.daemons(),
                expected: live.len() as u32,
            },
            fleet.baseline,
        );
        let marks: u64 = terminals.iter().map(Terminal::received_total).sum();
        let start = Instant::now();
        let mut second = 0u32;
        while start.elapsed() < self.window {
            until(start + Duration::from_secs(second.into()));
            match case.scenario {
                Scenario::BackgroundOutput | Scenario::StalledNewerStarts => {
                    probes.record(
                        Probe::Input,
                        input(&mut terminals[0], &tokens.next(), PROBE),
                    );
                }
                Scenario::ReconnectStorm => {
                    storm_or_input(fleet, &mut terminals[0], probes, &mut tokens, second, false);
                }
                Scenario::ContinuousArrivals => {
                    if second.is_multiple_of(2) {
                        let start = Instant::now();
                        let session = new_arrival(self, fleet, &sampler, second)?;
                        let terminal = self.spawn(case, fleet, &["attach", &session.id])?;
                        sampler.add_client(terminal.pid());
                        terminals.push(terminal);
                        probes.record(Probe::Arrival, arrival(fleet.baseline, &session, start));
                    }
                    probes.record(
                        Probe::Input,
                        input(&mut terminals[0], &tokens.next(), PROBE),
                    );
                }
                _ => {}
            }
            second += 1;
        }
        let display_bytes = terminals.iter().map(Terminal::received_total).sum::<u64>() - marks;
        for terminal in &mut terminals {
            let _ = terminal.send(format!("{PREFIX}d"));
        }
        for terminal in &mut terminals {
            let _ = terminal.wait_exit(PROBE);
        }
        Ok(Measured {
            admitted: None,
            sampled: sampler.finish(),
            components: None,
            display_bytes,
            notes: vec![
                "standalone clients are the comparison: no selector, switching or toasts".into(),
            ],
        })
    }

    /// The monitor started cold, then warm, until the window ends: each
    /// start until every session is connected, then prefix+x.
    fn monitor_loading(
        &self,
        case: &Case,
        fleet: &mut Fleet,
        probes: &mut Probes,
    ) -> io::Result<Measured> {
        let live = fleet.live();
        let sampler = Sampler::start(
            Watched {
                clients: Vec::new(),
                daemons: fleet.daemons(),
                expected: live.len() as u32,
            },
            fleet.baseline,
        );
        let (mut admitted, mut display_bytes, mut components) = (0, 0, None);
        let mut readiness = Vec::new();
        let start = Instant::now();
        while readiness.len() < 2 || start.elapsed() < self.window {
            let spawned = Instant::now();
            let mut terminal = self.spawn(case, fleet, &["monitor"])?;
            sampler.add_client(terminal.pid());
            let (count, ready) = connected(fleet.baseline, &live, live.len() as u32, spawned);
            admitted = count;
            readiness.push(ready);
            probes.record(Probe::Readiness, Some(ready));
            components = sample::components(terminal.pid()).or(components);
            probes.record(Probe::ExitStart, exit(&mut terminal));
            display_bytes += terminal.received_total();
            disconnected(fleet.baseline, &live)?;
        }
        Ok(Measured {
            admitted: Some(admitted),
            sampled: sampler.finish(),
            components,
            display_bytes,
            notes: vec![format!(
                "{} starts: cold readiness {} ms, then warm",
                readiness.len(),
                readiness[0].as_millis()
            )],
        })
    }

    /// N attach clients started at once, cold then warm, until every one
    /// is connected, then detached, until the window ends.
    fn standalone_loading(
        &self,
        case: &Case,
        fleet: &mut Fleet,
        probes: &mut Probes,
    ) -> io::Result<Measured> {
        let live = fleet.live();
        let sampler = Sampler::start(
            Watched {
                clients: Vec::new(),
                daemons: fleet.daemons(),
                expected: live.len() as u32,
            },
            fleet.baseline,
        );
        let mut display_bytes = 0;
        let mut readiness = Vec::new();
        let start = Instant::now();
        while readiness.len() < 2 || start.elapsed() < self.window {
            let spawned = Instant::now();
            let mut terminals = Vec::new();
            for session in &fleet.sessions {
                let terminal = self.spawn(case, fleet, &["attach", &session.id])?;
                sampler.add_client(terminal.pid());
                terminals.push(terminal);
            }
            let (_, ready) = connected(fleet.baseline, &live, live.len() as u32, spawned);
            readiness.push(ready);
            probes.record(Probe::Readiness, Some(ready));
            for terminal in &mut terminals {
                let _ = terminal.wait_for_input_ready(PROBE);
                let _ = terminal.send(format!("{PREFIX}d"));
            }
            for terminal in &mut terminals {
                let _ = terminal.wait_exit(PROBE);
                display_bytes += terminal.received_total();
            }
            drop(terminals);
            disconnected(fleet.baseline, &live)?;
        }
        Ok(Measured {
            admitted: None,
            sampled: sampler.finish(),
            components: None,
            display_bytes,
            notes: vec![format!(
                "{} starts: cold readiness {} ms, then warm",
                readiness.len(),
                readiness[0].as_millis()
            )],
        })
    }

    /// Measures this platform's requirements in the monitor: descriptors
    /// and threads with no session, one and eleven, each once every session
    /// is connected and a second has passed.
    pub fn qualify(&self, case: &Case) -> io::Result<Qualification> {
        // A session no client ever sees, in a home of its own, gives the
        // descriptors a daemon holds without a client.
        let unseen = Fleet {
            user: User::new(&self.candidate, None)?,
            sessions: Vec::new(),
            stalled: Vec::new(),
            baseline: Baseline(0),
            notes: Vec::new(),
        };
        let session = unseen.user.start(&self.measure, "b001", 0, false, false)?;
        let baseline = Baseline::of(&[session.daemon]);
        unseen.user.end(&[session]);
        let mut fleet = Fleet {
            user: User::new(&self.candidate, None)?,
            sessions: Vec::new(),
            stalled: Vec::new(),
            baseline,
            notes: Vec::new(),
        };
        let mut terminal = self.spawn(case, &fleet, &["monitor"])?;
        terminal.wait_for_text("No sessions to show.", SETUP)?;
        let mut counts = Vec::new();
        for target in [0, 1, 11] {
            while fleet.sessions.len() < target {
                let title = format!("s{:03}", fleet.sessions.len() + 1);
                let session = fleet.user.start(&self.measure, &title, 0, false, false)?;
                fleet.sessions.push(session);
            }
            let daemons = fleet.daemons();
            let (count, _) = connected(fleet.baseline, &daemons, target as u32, Instant::now());
            if count < target as u32 {
                return Err(io::Error::other(format!(
                    "{count} of {target} sessions connected"
                )));
            }
            thread::sleep(Duration::from_secs(1));
            let own = sample::sample(terminal.pid())
                .ok_or_else(|| io::Error::other("the monitor is gone"))?;
            counts.push((own.descriptors, own.threads));
        }
        let _ = exit(&mut terminal);
        let embedded = *Loader::new(self.candidate.home.clone())
            .load(&self.candidate.version)
            .map_err(io::Error::other)?
            .requirements();
        let [(fd0, th0), (fd1, th1), (fd11, th11)] = counts[..] else {
            unreachable!("three counts")
        };
        let per = |more: u32, fewer: u32| u64::from(more.saturating_sub(fewer)).div_ceil(10);
        let instance_descriptors = per(fd11, fd1);
        let threads = per(th11, th1);
        let load_descriptors =
            u64::from(fd1.saturating_sub(fd0)).saturating_sub(instance_descriptors);
        Ok(Qualification {
            requirements: vec![
                Requirement {
                    field: "workers",
                    measured: threads.saturating_sub(1),
                    embedded: embedded.workers.into(),
                },
                Requirement {
                    field: "instance_descriptors",
                    measured: instance_descriptors,
                    embedded: embedded.instance_descriptors.into(),
                },
                Requirement {
                    field: "load_descriptors",
                    measured: load_descriptors,
                    embedded: embedded.load_descriptors.into(),
                },
            ],
            evidence: vec![
                format!(
                    "Monitor at {}: descriptors {fd0}/{fd1}/{fd11} and threads {th0}/{th1}/{th11} \
                     with 0/1/11 connected sessions. Per session: the 10-session difference over \
                     10, rounded up; workers are threads per session less the owner thread; load \
                     descriptors are the first session's increase less one session's.",
                    case.geometry
                ),
                format!(
                    "Not measured from outside the monitor (owner question on #121): release_bytes \
                     {}, instance_bytes {}, owner_stack_bytes {}, worker_stack_bytes {} stay as \
                     embedded.",
                    embedded.release_bytes,
                    embedded.instance_bytes,
                    embedded.owner_stack_bytes,
                    embedded.worker_stack_bytes
                ),
            ],
        })
    }
}

/// What a mode's window measured beside its probes.
struct Measured {
    admitted: Option<u32>,
    sampled: sampler::Sampled,
    components: Option<sample::Components>,
    display_bytes: u64,
    notes: Vec<String>,
}

/// Prefix+s until the selector is painted, then Escape (a global action)
/// until it is gone.
fn selector(terminal: &mut Terminal, probes: &mut Probes) {
    probes.record(Probe::Selector, open_selector(terminal));
    let start = Instant::now();
    let closed = terminal.send(ESCAPE).is_ok()
        && terminal
            .wait_for_screen(PROBE, |screen| !screen.contains('║'))
            .is_ok();
    probes.record(Probe::GlobalAction, closed.then(|| start.elapsed()));
}

fn open_selector(terminal: &mut Terminal) -> Option<Duration> {
    let mark = terminal.received_total();
    let start = Instant::now();
    terminal.send(format!("{PREFIX}s")).ok()?;
    terminal.wait_for_bytes_since(mark, SELECTOR, PROBE).ok()?;
    Some(start.elapsed())
}

/// Whether the screen shows session `title` selected: its title leads the
/// chrome and, for a silent session, its start line is restored (output
/// scrolls a writing session's away).
fn shows(screen: &str, title: &str, silent: bool) -> bool {
    screen.starts_with(&format!("{title} "))
        && (!silent || screen.contains(&format!("started {title}")))
}

/// Selects `title` through the selector's search: Enter until it is shown.
fn switch(
    terminal: &mut Terminal,
    title: &str,
    silent: bool,
    probes: &mut Probes,
) -> Option<Duration> {
    probes.record(Probe::Selector, open_selector(terminal));
    terminal.send(title).ok()?;
    terminal
        .wait_for_screen(PROBE, |screen| screen.contains(&format!(" {title}")))
        .ok()?;
    let start = Instant::now();
    terminal.send("\r").ok()?;
    terminal
        .wait_for_screen(PROBE, |screen| {
            shows(screen, title, silent) && !screen.contains('║')
        })
        .ok()?;
    Some(start.elapsed())
}

fn select(terminal: &mut Terminal, title: &str, silent: bool) -> io::Result<()> {
    let deadline = Instant::now() + SETUP;
    let mut ignored = Probes::default();
    while switch(terminal, title, silent, &mut ignored).is_none() {
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "{title} was never selected:\n{}",
                terminal.screen()
            )));
        }
        let _ = terminal.send(ESCAPE);
    }
    Ok(())
}

/// Prefix+x until the host is released, then the monitor's exit.
fn exit(terminal: &mut Terminal) -> Option<Duration> {
    let mark = terminal.received_total();
    let start = Instant::now();
    terminal.send(format!("{PREFIX}x")).ok()?;
    let released = terminal
        .wait_for_bytes_since(mark, ALTERNATE_OFF, PROBE)
        .ok()
        .map(|()| start.elapsed());
    let _ = terminal.wait_exit(SETUP);
    released
}

/// s002 reports attention: until the host bell of its toast (attention
/// presentation), until the toast's text shows and until it is gone
/// (NOT-02); then it works again.
fn attention(terminal: &mut Terminal, session: &Session, probes: &mut Probes) -> io::Result<()> {
    let before = bells(&terminal.host_signals());
    let start = Instant::now();
    ask(session, "attention")?;
    let rang = terminal
        .wait_for_host_signals(PROBE, |signals| bells(signals) > before)
        .ok()
        .map(|()| start.elapsed());
    probes.record(Probe::Attention, rang);
    if rang.is_some() {
        let bell = Instant::now();
        let shown = terminal
            .wait_for_screen(PROBE, |screen| screen.contains(TOAST_HINT))
            .ok()
            .map(|()| bell.elapsed());
        probes.record(Probe::ToastText, shown);
        let gone = terminal
            .wait_for_screen(Duration::from_secs(10), |screen| {
                !screen.contains(TOAST_HINT)
            })
            .ok()
            .map(|()| bell.elapsed());
        probes.record(Probe::ToastGone, gone);
    }
    ask(session, "working")
}

/// The heuristic session retitles itself and falls silent: until its
/// attention toast shows (STA-02, the daemon's sampler under load).
fn status_sampler(terminal: &mut Terminal, session: &Session) -> io::Result<Option<Duration>> {
    let start = Instant::now();
    ask(session, "title")?;
    let label = format!("{}-changed", session.title);
    Ok(terminal
        .wait_for_screen(SAMPLER, |screen| {
            screen.contains(TOAST_HINT) && screen.contains(&label)
        })
        .ok()
        .map(|()| start.elapsed()))
}

/// Every 15 s from the fifth: every daemon stopped for [`STORM`], the
/// monitor's selector probed meanwhile, then resumed, until typed input
/// echoes again (recovery). Typed input every other second.
fn storm_or_input(
    fleet: &Fleet,
    terminal: &mut Terminal,
    probes: &mut Probes,
    tokens: &mut Tokens,
    second: u32,
    monitor: bool,
) {
    if second % 15 != 5 {
        probes.record(Probe::Input, input(terminal, &tokens.next(), PROBE));
        return;
    }
    let daemons = fleet.daemons();
    let resume = Instant::now() + STORM;
    signal(&daemons, libc::SIGSTOP);
    if monitor {
        selector(terminal, probes);
    }
    until(resume);
    signal(&daemons, libc::SIGCONT);
    probes.record(Probe::Recovery, input(terminal, &tokens.next(), SETUP));
}

/// A new session, a{second}, at the background rate.
fn new_arrival(
    bench: &Bench,
    fleet: &mut Fleet,
    sampler: &Sampler,
    second: u32,
) -> io::Result<Session> {
    let session = fleet
        .user
        .start(&bench.measure, &format!("a{second:03}"), 0, false, false)?;
    sampler.add_session(session.daemon);
    fleet.sessions.push(session.clone());
    Ok(session)
}

/// From `start` until a client is connected to `session`.
fn arrival(baseline: Baseline, session: &Session, start: Instant) -> Option<Duration> {
    while start.elapsed() < SETUP {
        if baseline.connected(session.daemon) {
            return Some(start.elapsed());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

fn arrive(
    bench: &Bench,
    fleet: &mut Fleet,
    sampler: &Sampler,
    probes: &mut Probes,
    second: u32,
) -> io::Result<()> {
    let start = Instant::now();
    let session = new_arrival(bench, fleet, sampler, second)?;
    probes.record(Probe::Arrival, arrival(fleet.baseline, &session, start));
    Ok(())
}

/// The declared environment.
fn environment(candidate: &Candidate, window: Duration) -> Environment {
    let command = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let hardware = if cfg!(target_os = "macos") {
        format!(
            "{}; {} bytes of memory",
            command("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]),
            command("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        )
    } else {
        let cpu = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let memory = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        format!(
            "{}; {}",
            cpu.lines()
                .find(|line| line.starts_with("model name"))
                .unwrap_or(""),
            memory.lines().next().unwrap_or("")
        )
    };
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    let window = if window == args::WINDOW {
        "60 s per scenario (the owner's decision)".to_owned()
    } else {
        format!(
            "{} s per scenario: REDUCED from the decided 60 s, not qualification evidence",
            window.as_secs()
        )
    };
    let runner = ["RUNNER_NAME", "ImageOS", "ImageVersion", "GITHUB_SHA"]
        .map(|name| format!("{name}={}", std::env::var(name).unwrap_or_default()))
        .join(" ");
    Environment {
        entries: vec![
            ("os", command("uname", &["-srv"])),
            ("architecture", std::env::consts::ARCH.to_owned()),
            ("hardware", hardware),
            (
                "processors",
                thread::available_parallelism()
                    .map_or(0, usize::from)
                    .to_string(),
            ),
            ("runner", runner),
            (
                "candidate",
                format!("{} (sha256 {})", candidate.archive, candidate.sha256),
            ),
            ("release", candidate.version.to_string()),
            (
                "configuration",
                format!(
                    "monitor defaults; a repeat with {} is disclosed per run; descriptor limit {} (hard {})",
                    sufficient(),
                    limit.rlim_cur,
                    limit.rlim_max
                ),
            ),
            (
                "workload",
                "sessions of climon2-measure's workload: background output 720 B/s per \
                 unselected session, selected sustained output 262144 B/s, otherwise silent; \
                 all report OSC 7501 working except the heuristic h001"
                    .to_owned(),
            ),
            ("duration", window),
            (
                "sampling",
                format!(
                    "every {} ms: procfs stat and fd (Linux) or proc_pidinfo (macOS) of the \
                     client process trees and the daemons; a session counts as connected while \
                     its daemon holds more descriptors than with no client",
                    sampler::INTERVAL.as_millis()
                ),
            ),
            (
                "latency",
                "from the probe's keys or trigger to the first host bytes or screen state that \
                 show its result, on an emulated host terminal (climon2-test-term); input is the \
                 echo round trip, an upper bound on dispatch"
                    .to_owned(),
            ),
        ],
    }
}

/// Raises the soft descriptor limit to the hard one, for a hundred
/// clients and their terminals; disclosed in the environment.
fn raise_descriptor_limit() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
        limit.rlim_cur = limit.rlim_max.min(65_536);
        // SAFETY: as above.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}

/// Measures every selected case and writes `report.json` and `report.md`
/// to the output directory. Whether every case ran without a harness error.
pub fn measure(options: &args::Run) -> io::Result<bool> {
    raise_descriptor_limit();
    let measure = std::env::current_exe()?;
    let candidate = Candidate::install(&options.archive)?;
    let environment = environment(&candidate, options.window);
    let bench = Bench::new(candidate, measure, options.window);
    let cases: Vec<Case> = matrix(&options.geometries)
        .into_iter()
        .filter(|case| {
            options
                .only
                .as_ref()
                .is_none_or(|only| case.to_string().contains(only))
        })
        .collect();
    let mut clean = true;
    let qualification = match cases.first() {
        Some(case) => bench.qualify(case).unwrap_or_else(|error| {
            clean = false;
            Qualification {
                requirements: Vec::new(),
                evidence: vec![format!("harness error: {error}")],
            }
        }),
        None => Qualification::default(),
    };
    let mut report = Report {
        environment,
        qualification,
        records: Vec::new(),
    };
    for case in cases {
        eprintln!("climon2-measure: {case}");
        let (record, error) = bench.case(case);
        if let Some(error) = error {
            eprintln!("climon2-measure: {case}: {error}");
            clean = false;
        }
        report.records.push(record);
        write(&options.out, &report)?;
    }
    write(&options.out, &report)?;
    Ok(clean)
}

fn write(out: &Path, report: &Report) -> io::Result<()> {
    fs::create_dir_all(out)?;
    fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&report.to_json()).map_err(io::Error::other)?,
    )?;
    fs::write(out.join("report.md"), report.to_markdown())
}
