//! The monitor library driven through its own interface (ADR 0011): a
//! whole monitor on a climon2 home of its own, given the time, a client
//! library loader and a configuration, stepped by a consumer that is no
//! terminal.
use climon2_core::{
    SessionId,
    config::MonitorConfig,
    layout::{ExecutableHome, HomeLayout},
    metadata::SessionMetadata,
    protocol::TerminalSize,
};
use climon2_monitor::{
    Action, Canvas, Choice, Consumer, Drive, Event, Inputs, Look, Monitor, NO_ATTENTION, Paste,
    SessionAt, System, Verdict, load, toast::Toast,
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

/// A consumer with room for 80 by 24, whose window has focus, that places
/// every toast and lets none cover the canvas: a modal of its own covers
/// the selected session while `covered`.
#[derive(Default)]
struct Fake {
    covered: bool,
}

impl Consumer for Fake {
    fn covered(&self) -> bool {
        self.covered
    }

    fn focused(&self) -> bool {
        true
    }

    fn size(&self) -> Option<TerminalSize> {
        Some(TerminalSize { cols: 80, rows: 24 })
    }

    fn fits(&self, _: &Toast) -> bool {
        true
    }

    fn obscures(&self, _: &Toast, _: Option<(u16, u16)>) -> bool {
        false
    }
}

/// A monitor on the home at `root`, whose client libraries come from an
/// empty executable home beside it.
fn started(root: &Path) -> Monitor {
    let home = HomeLayout::at(root).unwrap();
    let executables = root.join("executables");
    fs::create_dir_all(&executables).unwrap();
    let executable = ExecutableHome::new(&executables).unwrap();
    let log = climon2_platform::logging::LogFile::monitor(root).unwrap();
    Monitor::start(Inputs {
        home,
        load: load::loader(executable),
        config: MonitorConfig::parse(b"{}").unwrap(),
        own: None,
        filter: "info".to_owned(),
        dispatch: log.dispatch(Some("info")).unwrap(),
    })
    .unwrap()
}

/// A canonical climon2 home of its own.
fn home() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    (directory, root)
}

/// Shuts `monitor` down, a round at a time, and returns its verdict.
fn leave(monitor: &mut Monitor) -> Verdict {
    monitor.shut_down();
    while monitor.waiting(Instant::now()).is_some() {
        std::thread::sleep(Duration::from_millis(10));
        monitor.waited();
    }
    monitor.left()
}

/// Steps `monitor` until `done` holds of it, at most 10 s, gathering the
/// events each step returned.
fn until(monitor: &mut Monitor, consumer: &Fake, done: impl Fn(&Monitor) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    while !done(monitor) {
        assert!(Instant::now() < deadline, "the monitor never got there");
        events.extend(monitor.step(&System, consumer));
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

/// NOT-03 and NOT-04 through the library (ADR 0011): with no session, a
/// painted frame tells the host once that nothing needs attention, the
/// jump selects nothing and shows its notice for 3 s from the time it is
/// given, and a monitor with no client leaves cleanly.
#[test]
fn a_monitor_with_no_session_reports_no_attention_and_leaves_cleanly() {
    let (_directory, root) = home();
    let mut monitor = started(&root);
    let consumer = Fake::default();
    assert_eq!(monitor.step(&System, &consumer), []);
    monitor.frame(Instant::now(), &consumer);
    assert_eq!(monitor.painted(&consumer), [Event::Attention(false)]);
    assert_eq!(monitor.painted(&consumer), []);
    let at = Instant::now() + Duration::from_secs(60);
    assert_eq!(monitor.act(Action::Choose(Choice::Jump), at, &consumer), []);
    assert_eq!(monitor.selected(), None);
    assert_eq!(monitor.notice(at), Some(NO_ATTENTION));
    let later = at + Duration::from_millis(2999);
    assert_eq!(monitor.notice(later), Some(NO_ATTENTION));
    assert_eq!(monitor.notice(at + Duration::from_secs(3)), None);
    assert_eq!(leave(&mut monitor), Verdict::Clean);
}

/// Session metadata in `home` that discovery cannot judge: nothing listens
/// on its socket, but its daemon cannot be proved dead, so the session is
/// uncertain and its row unavailable, and no client starts for it. Its
/// daemon is this test's own process, which is alive, and on Linux has no
/// Linux identity, so is unknown. A PID of 0 would not do: Windows reports
/// no such process, which proves the session stale.
fn uncertain(home: &Path, id: &SessionId) {
    let sessions = home.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let socket = home.join("absent.sock");
    let pid = std::process::id();
    let started = climon2_platform::process::start_time(pid).unwrap().unwrap();
    let metadata: SessionMetadata = serde_json::from_value(serde_json::json!({
        "format_version": 1, "session_id": id.as_str(), "socket_path": socket,
        "daemon_pid": pid, "daemon_start_time": started, "command": "bash", "args": [],
        "cwd": home, "depth": 1, "created_at": "2026-11-28T10:00:00.000Z",
        "climon2_version": "0.1.0"
    }))
    .unwrap();
    let path = sessions.join(format!("{id}.pty"));
    fs::write(path, metadata.to_json().unwrap()).unwrap();
}

/// Spec #73 "Enter" and REC-02 through the library: a discovered session
/// gets a row; selecting it, which no consumer modal stops, is heard by
/// the consumer, and with no client and no history its canvas says it has
/// no content, under its status; its read-only history is offered only
/// while no modal covers it, and it has none.
#[test]
fn a_discovered_session_is_listed_and_selected_through_the_library() {
    let (_directory, root) = home();
    let session: SessionId = "0d5c0000-0000-4000-8000-0000000004a0".parse().unwrap();
    uncertain(&root, &session);
    let id = SessionAt::local(session);
    let mut monitor = started(&root);
    let consumer = Fake::default();
    until(&mut monitor, &consumer, |monitor| monitor.listed(&id));
    let rows = monitor.rows();
    let shown: Vec<_> = rows
        .iter()
        .map(|row| (&row.id, row.status.as_str(), row.label.as_str()))
        .collect();
    assert_eq!(shown, [(&id, "Unavailable", "bash")]);
    assert_eq!(monitor.selected(), None);
    let events = monitor.act(
        Action::Choose(Choice::Select(id.clone())),
        Instant::now(),
        &consumer,
    );
    assert_eq!(events, [Event::Selected]);
    assert_eq!(monitor.selected(), Some(&id));
    let rows = monitor.rows();
    let session = monitor.session(&rows).unwrap();
    assert_eq!(session.id, &id);
    assert_eq!(session.control, "Not controlling");
    assert!(matches!(
        session.canvas,
        Canvas::NoContent("Unavailable", None)
    ));
    assert!(monitor.taking(&consumer).is_none());
    assert!(monitor.history(&consumer).is_none());
    assert_eq!(leave(&mut monitor), Verdict::Clean);
}

/// Spec #73 "Paste rules" and "Delete" through the library: a selected
/// session the monitor does not control takes nothing that would drive it,
/// and looking at it changes nothing it shows; a confirmed Delete is then
/// under way, so its row offers no second one.
#[test]
fn an_uncontrolled_session_takes_no_input_and_its_delete_goes_under_way() {
    let (_directory, root) = home();
    let session: SessionId = "0d5c0000-0000-4000-8000-0000000004a1".parse().unwrap();
    uncertain(&root, &session);
    let id = SessionAt::local(session);
    let mut monitor = started(&root);
    let consumer = Fake::default();
    until(&mut monitor, &consumer, |monitor| monitor.listed(&id));
    let now = Instant::now();
    let select = Action::Choose(Choice::Select(id.clone()));
    assert_eq!(monitor.act(select, now, &consumer), [Event::Selected]);
    let paste = Paste {
        starts: true,
        bytes: b"pasted".to_vec(),
        ends: true,
    };
    for action in [
        Action::Drive(Drive::TakeControl),
        Action::Drive(Drive::Input(b"typed".to_vec())),
        Action::Drive(Drive::Paste(paste)),
        Action::Drive(Drive::Resize),
        Action::Look(Look::Scroll(1)),
        Action::Look(Look::Dismiss),
        Action::Look(Look::Acknowledge),
    ] {
        assert_eq!(
            monitor.act(action.clone(), now, &consumer),
            [],
            "{action:?}"
        );
        let rows = monitor.rows();
        let session = monitor.session(&rows).unwrap();
        assert_eq!(session.control, "Not controlling", "{action:?}");
        assert!(
            matches!(session.canvas, Canvas::NoContent(..)),
            "{action:?}"
        );
        assert!(monitor.taking(&consumer).is_none(), "{action:?}");
    }
    assert!(monitor.offer(&id).is_some());
    let delete = Action::Choose(Choice::Delete(id.clone()));
    assert_eq!(monitor.act(delete, now, &consumer), []);
    assert_eq!(monitor.offer(&id), None);
    assert_eq!(monitor.selected(), Some(&id));
    leave(&mut monitor);
}
