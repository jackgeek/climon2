//! PROTOTYPE behavioural tests for #50/#44. Run with `-j1 -- --test-threads=1`.
//!
//! Every session is a real `nest-daemon` owning a real PTY. Its command is a tiny
//! sh loop that runs "plan" files the test drops in, so the test can type commands
//! *inside* a session. Every attach therefore runs on the session's own terminal,
//! exactly like a user nesting `climon2 attach` inside a session.
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

const DAEMON: &str = env!("CARGO_BIN_EXE_nest-daemon");
const CLIENT: &str = env!("CARGO_BIN_EXE_nest-client");

/// Socket paths must stay under 103 bytes, so runtime roots live at the worktree root.
fn worktree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

struct Lab {
    root: PathBuf,
    work: PathBuf,
    sessions: Vec<Session>,
}

struct Session {
    sid: String,
    pid: u32,
    child: Child,
    plans: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let base = worktree().join("rt");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join(".gitignore"), "*\n").unwrap();
        let root = base.join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let work = worktree().join("prototypes/nesting-proof/work").join(name);
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work).unwrap();
        Self { root, work, sessions: Vec::new() }
    }

    fn home(&self, name: &str) -> PathBuf {
        let home = self.work.join(name);
        fs::create_dir_all(&home).unwrap();
        home
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.env("NEST_RUNTIME_ROOT", &self.root);
        command
    }

    /// Starts a session in `home`; `legacy` emulates a first-release daemon.
    fn start(&mut self, home: &Path, legacy: bool) -> usize {
        let index = self.sessions.len();
        let plans = self.work.join(format!("s{index}"));
        fs::create_dir_all(&plans).unwrap();
        let ready = plans.join("ready");
        let script = format!(
            "cd '{}'; while :; do for p in plan*; do [ -f \"$p\" ] && mv \"$p\" \"run-$p\" && . \"./run-$p\"; done; sleep 0.02; done",
            plans.display()
        );
        let child = self
            .command(DAEMON)
            .args(["--home", home.to_str().unwrap()])
            .args(["--mode", if legacy { "legacy" } else { "new" }])
            .args(["--ready", ready.to_str().unwrap()])
            .args(["--", "/bin/sh", "-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let text = wait_for(&ready, Duration::from_secs(10));
        let mut parts = text.split_whitespace();
        let sid = parts.next().unwrap().to_owned();
        let pid = parts.next().unwrap().parse().unwrap();
        self.sessions.push(Session { sid, pid, child, plans });
        index
    }

    fn sid(&self, index: usize) -> String {
        self.sessions[index].sid.clone()
    }

    /// Runs `attach` *inside* session `inside`, in the background on its terminal.
    /// Returns the report path the client writes its outcome to.
    fn attach_inside(&self, inside: usize, home: &Path, target: &str, extra: &str) -> PathBuf {
        let session = &self.sessions[inside];
        let report = session.plans.join(format!("report-{}", unique()));
        let plan = format!(
            "{extra} '{CLIENT}' attach --home '{}' {target} --report '{}' </dev/tty >/dev/tty 2>&1 &\n",
            home.display(),
            report.display()
        );
        self.plan(inside, &plan);
        report
    }

    fn plan(&self, inside: usize, body: &str) {
        let session = &self.sessions[inside];
        let staging = session.plans.join(format!("staging-{}", unique()));
        fs::write(&staging, format!("NEST_RUNTIME_ROOT='{}'; export NEST_RUNTIME_ROOT\n{body}", self.root.display())).unwrap();
        fs::rename(&staging, session.plans.join(format!("plan-{}", unique()))).unwrap();
    }

    /// Top-level attach from a terminal climon2 does not own (`script` allocates it).
    fn attach_top_level(&self, home: &Path, target: &str) -> Outcome {
        let report = self.work.join(format!("top-{}", unique()));
        let mut child = self
            .command("/usr/bin/script")
            .args(["-q", "/dev/null", CLIENT, "attach", "--home", home.to_str().unwrap(), target])
            .args(["--report", report.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let outcome = outcome(&report);
        if !outcome.attached() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // An attached client holds until its daemon goes away when the Lab drops.
        outcome
    }

    fn hello(&self, index: usize) -> serde_json::Value {
        let output = self
            .command(CLIENT)
            .args(["hello", &self.sid(index)])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for session in &mut self.sessions {
            unsafe { libc::kill(session.pid as i32, libc::SIGTERM) };
            let _ = session.child.wait();
        }
    }
}

fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst))
}

fn wait_for(path: &Path, limit: Duration) -> String {
    let start = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            if text.ends_with('\n') {
                return text;
            }
        }
        assert!(start.elapsed() < limit, "timed out waiting for {}", path.display());
        sleep(Duration::from_millis(10));
    }
}

#[derive(Debug)]
struct Outcome {
    line: String,
}

impl Outcome {
    fn attached(&self) -> bool {
        self.line.starts_with("attached ")
    }
    fn rejected(&self) -> bool {
        self.line.starts_with("rejected 125 ")
    }
    fn client_pid(&self) -> i32 {
        self.line.split_whitespace().nth(1).unwrap().parse().unwrap()
    }
}

fn outcome(report: &Path) -> Outcome {
    Outcome { line: wait_for(report, Duration::from_secs(20)).trim().to_owned() }
}

fn detach(outcome: &Outcome) {
    unsafe { libc::kill(outcome.client_pid(), libc::SIGTERM) };
    sleep(Duration::from_millis(300));
}

// ---- Q1/Q2: proof, cycles, homes -------------------------------------------------

#[test]
fn self_attachment_is_rejected_with_125() {
    let mut lab = Lab::new("t01");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let result = outcome(&lab.attach_inside(a, &home, &lab.sid(a), ""));
    assert!(result.rejected(), "{result:?}");
    assert!(result.line.contains("itself"), "{result:?}");
}

#[test]
fn acyclic_nesting_is_admitted_and_reverse_attachment_is_rejected() {
    let mut lab = Lab::new("t02");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    let forward = outcome(&lab.attach_inside(a, &home, &lab.sid(b), ""));
    assert!(forward.attached(), "{forward:?}");
    let reverse = outcome(&lab.attach_inside(b, &home, &lab.sid(a), ""));
    assert!(reverse.rejected() && reverse.line.contains("cycle"), "{reverse:?}");
}

#[test]
fn indirect_cycle_across_three_sessions_is_rejected() {
    let mut lab = Lab::new("t03");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    let c = lab.start(&home, false);
    assert!(outcome(&lab.attach_inside(a, &home, &lab.sid(b), "")).attached());
    assert!(outcome(&lab.attach_inside(b, &home, &lab.sid(c), "")).attached());
    let closing = outcome(&lab.attach_inside(c, &home, &lab.sid(a), ""));
    assert!(closing.rejected() && closing.line.contains("cycle"), "{closing:?}");
}

#[test]
fn cross_home_cycle_is_rejected_and_cross_home_nesting_works() {
    let mut lab = Lab::new("t04");
    let first = lab.home("one");
    let second = lab.home("two");
    let a = lab.start(&first, false);
    let b = lab.start(&second, false);
    assert!(outcome(&lab.attach_inside(a, &second, &lab.sid(b), "")).attached());
    let reverse = outcome(&lab.attach_inside(b, &first, &lab.sid(a), ""));
    assert!(reverse.rejected() && reverse.line.contains("cycle"), "{reverse:?}");
}

#[test]
fn home_aliases_resolve_to_the_same_graph() {
    let mut lab = Lab::new("t05");
    let home = lab.home("real");
    let alias = lab.work.join("alias");
    symlink(&home, &alias).unwrap();
    let a = lab.start(&home, false);
    let b = lab.start(&alias, false);
    assert!(outcome(&lab.attach_inside(a, &alias, &lab.sid(b), "")).attached());
    let reverse = outcome(&lab.attach_inside(b, &home, &lab.sid(a), ""));
    assert!(reverse.rejected() && reverse.line.contains("cycle"), "{reverse:?}");
    // APFS is case-insensitive; canonicalisation returns the on-disk case, so a
    // case alias also lands in the same runtime directory. The graph is keyed by
    // GUID across every runtime directory either way.
    let upper = PathBuf::from(home.to_str().unwrap().replace("/real", "/REAL"));
    assert!(upper.exists());
    let c = lab.start(&upper, false);
    assert!(outcome(&lab.attach_inside(a, &upper, &lab.sid(c), "")).attached());
    let back = outcome(&lab.attach_inside(c, &home, &lab.sid(a), ""));
    assert!(back.rejected() && back.line.contains("cycle"), "{back:?}");
    let runtime_dirs = fs::read_dir(&lab.root).unwrap().flatten().filter(|e| e.path().is_dir()).count();
    assert_eq!(runtime_dirs, 1, "aliases must share one runtime directory");
}

#[test]
fn orphaned_client_on_the_session_terminal_is_caught_by_the_tty_proof() {
    // Double fork: the client is reparented to launchd, so ancestry is blind, but
    // its stdin/stdout are still session A's terminal.
    let mut lab = Lab::new("t18");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let report = lab.sessions[a].plans.join("orphan-report");
    lab.plan(a, &format!(
        "( '{CLIENT}' attach --home '{}' {} --report '{}' </dev/tty >/dev/tty 2>&1 & )\n",
        home.display(), lab.sid(a), report.display()
    ));
    let result = outcome(&report);
    assert!(result.rejected(), "{result:?}");
    assert!(result.line.contains("(source=tty)"), "{result:?}");
}

#[test]
fn proof_ignores_tampered_environment() {
    let mut lab = Lab::new("t06");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    assert!(outcome(&lab.attach_inside(a, &home, &lab.sid(b), "")).attached());
    let scrubbed = "env -u CLIMON2_SESSION_ID CLIMON2_SESSION_DEPTH=1";
    let reverse = outcome(&lab.attach_inside(b, &home, &lab.sid(a), scrubbed));
    assert!(reverse.rejected(), "scrubbed env must not hide the encloser: {reverse:?}");
}

#[test]
fn forged_environment_does_not_block_safe_attachment() {
    let mut lab = Lab::new("t07");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    let forged = format!("env CLIMON2_SESSION_ID={} CLIMON2_SESSION_DEPTH=9", lab.sid(b));
    let result = outcome(&lab.attach_inside(a, &home, &lab.sid(b), &forged));
    assert!(result.attached(), "{result:?}");
}

#[test]
fn local_relay_inside_a_session_is_caught_by_process_ancestry() {
    let mut lab = Lab::new("t08");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    // `script` puts the client on a fresh PTY that climon2 does not own; only
    // process ancestry still links it to session A.
    let result = outcome(&lab.attach_inside(a, &home, &lab.sid(a), "/usr/bin/script -q /dev/null"));
    assert!(result.rejected(), "{result:?}");
    assert!(result.line.contains("source=ancestry"), "{result:?}");
}

#[test]
fn screen_relay_cycle_is_not_detected() {
    // Documents the gap: a relay whose server is not the client's descendant
    // (screen/tmux started outside, ssh). Session A views a screen session whose
    // window attaches A: a real cycle, but the client's evidence (screen's PTY,
    // screen's server as ancestor) is identical to an unrelated top-level window.
    let mut lab = Lab::new("t09");
    let home = lab.home("h");
    let screens = lab.work.join("screens");
    fs::create_dir_all(&screens).unwrap();
    fs::set_permissions(&screens, fs::Permissions::from_mode(0o700)).unwrap();
    let a = lab.start(&home, false);
    let report = lab.work.join("screen-report");
    let go = lab.work.join("screen-go");
    let window = format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done; exec '{CLIENT}' attach --home '{}' {} --report '{}'",
        go.display(),
        home.display(),
        lab.sid(a),
        report.display()
    );
    let status = lab
        .command("/usr/bin/screen")
        .env("SCREENDIR", &screens)
        .env("TERM", "xterm")
        .args(["-d", "-m", "-S", "relay", "/bin/sh", "-c", &window])
        .status()
        .unwrap();
    assert!(status.success());
    lab.plan(a, &format!("SCREENDIR='{}' TERM=xterm /usr/bin/screen -x relay\n", screens.display()));
    sleep(Duration::from_millis(800));
    let listing = Command::new("/usr/bin/screen").env("SCREENDIR", &screens).arg("-ls").output().unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(listing.contains("relay") && listing.contains("Attached"), "screen not viewed inside A: {listing}");
    fs::write(&go, "").unwrap();
    let result = outcome(&report);
    println!("screen relay outcome: {result:?}");
    assert!(result.attached(), "screen relay was detected after all: {result:?}");
    detach(&result);
    let _ = Command::new("/usr/bin/screen").env("SCREENDIR", &screens).args(["-S", "relay", "-X", "quit"]).status();
}

// ---- Q2: atomic admission and edge lifecycle --------------------------------------

fn cross_attach_round(name: &str, extra: &str) -> (Outcome, Outcome) {
    let mut lab = Lab::new(name);
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    let go = lab.work.join("go");
    let barrier = format!("while [ ! -f '{}' ]; do sleep 0.002; done;", go.display());
    let wait = format!("{barrier} {extra}");
    let first = lab.attach_inside(a, &home, &lab.sid(b), &wait);
    let second = lab.attach_inside(b, &home, &lab.sid(a), &wait);
    sleep(Duration::from_millis(300));
    fs::write(&go, "").unwrap();
    let results = (outcome(&first), outcome(&second));
    drop(lab);
    results
}

#[test]
fn simultaneous_cross_attachments_never_both_succeed() {
    for round in 0..8 {
        let (first, second) = cross_attach_round(&format!("c{round}"), "NEST_ADMIT_DELAY_MS=250");
        let admitted = [first.attached(), second.attached()].iter().filter(|x| **x).count();
        let rejected = [first.rejected(), second.rejected()].iter().filter(|x| **x).count();
        assert_eq!((admitted, rejected), (1, 1), "round {round}: {first:?} {second:?}");
    }
}

#[test]
fn without_the_lock_simultaneous_cross_attachments_both_succeed() {
    // Negative control: proves the race window is real and the lock closes it.
    let (first, second) = cross_attach_round("c-nolock", "NEST_ADMIT_DELAY_MS=250 NEST_UNSAFE_NO_LOCK=1");
    assert!(first.attached() && second.attached(), "{first:?} {second:?}");
}

#[test]
fn disconnect_removes_the_edge() {
    let mut lab = Lab::new("t10");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    let forward = outcome(&lab.attach_inside(a, &home, &lab.sid(b), ""));
    assert!(forward.attached());
    assert_eq!(lab.hello(b)["nesting"]["incoming"], serde_json::json!([lab.sid(a)]));
    detach(&forward);
    assert_eq!(lab.hello(b)["nesting"]["incoming"], serde_json::json!([]));
    let reverse = outcome(&lab.attach_inside(b, &home, &lab.sid(a), ""));
    assert!(reverse.attached(), "{reverse:?}");
}

#[test]
fn control_change_keeps_the_edge_and_rejection_disturbs_nothing() {
    let mut lab = Lab::new("t11");
    let home = lab.home("h");
    let a = lab.start(&home, false);
    let b = lab.start(&home, false);
    assert!(outcome(&lab.attach_inside(a, &home, &lab.sid(b), "")).attached());
    let top = lab.attach_top_level(&home, &lab.sid(b));
    assert!(top.attached(), "{top:?}");
    let before = lab.hello(b);
    assert_eq!(before["state"]["attachments"], 2);
    assert_eq!(before["prototype_debug"]["controller_pid"], top.client_pid());
    assert_eq!(before["nesting"]["incoming"], serde_json::json!([lab.sid(a)]));
    let reverse = outcome(&lab.attach_inside(b, &home, &lab.sid(a), ""));
    assert!(reverse.rejected(), "control moved but edge must stay: {reverse:?}");
    assert_eq!(lab.hello(b), before, "rejection disturbed the target");
    let a_state = lab.hello(a);
    assert_eq!(a_state["state"]["attachments"], 0, "rejected attach reached the daemon");
}

// ---- Q3: legacy participants ------------------------------------------------------

#[test]
fn legacy_daemon_cannot_take_part_post_hello() {
    let mut lab = Lab::new("t12");
    let home = lab.home("h");
    let l = lab.start(&home, true);
    let hello = lab.hello(l);
    assert!(hello.get("nesting").is_none() && hello.get("state").is_none(), "{hello}");
    let output = lab.command(CLIENT).args(["raw-nest-attach", &lab.sid(l)]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "closed");
}

#[test]
fn nesting_inside_a_legacy_session_fails_closed() {
    let mut lab = Lab::new("t13");
    let home = lab.home("h");
    let l = lab.start(&home, true);
    let t = lab.start(&home, false);
    let result = outcome(&lab.attach_inside(l, &home, &lab.sid(t), ""));
    assert!(result.rejected() && result.line.contains("cannot be established"), "{result:?}");
    assert_eq!(lab.hello(t)["state"]["attachments"], 0);
}

#[test]
fn legacy_target_fails_closed_and_top_level_still_works_beside_legacy() {
    let mut lab = Lab::new("t14");
    let home = lab.home("h");
    let l = lab.start(&home, true);
    let t = lab.start(&home, false);
    let into_legacy = lab.attach_top_level(&home, &lab.sid(l));
    assert!(into_legacy.rejected() && into_legacy.line.contains("does not support"), "{into_legacy:?}");
    let top = lab.attach_top_level(&home, &lab.sid(t));
    assert!(top.attached(), "{top:?}");
    let e = lab.start(&home, false);
    let nested = outcome(&lab.attach_inside(e, &home, &lab.sid(t), ""));
    assert!(nested.attached(), "unrelated legacy session must not block: {nested:?}");
}

#[test]
fn hidden_legacy_edge_cannot_close_a_cycle() {
    // A first-release client attached from new session E to legacy session L is
    // an edge E->L nobody reports. Attaching E from inside L must fail closed.
    let mut lab = Lab::new("t15");
    let home = lab.home("h");
    let e = lab.start(&home, false);
    let l = lab.start(&home, true);
    let result = outcome(&lab.attach_inside(l, &home, &lab.sid(e), ""));
    assert!(result.rejected(), "{result:?}");
}

// ---- Q4: hello compatibility ------------------------------------------------------

#[test]
fn new_list_shows_unknown_state_for_legacy_and_state_for_new() {
    let mut lab = Lab::new("t16");
    let home = lab.home("h");
    let l = lab.start(&home, true);
    let n = lab.start(&home, false);
    let output = lab.command(CLIENT).args(["list", "--home", home.to_str().unwrap()]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let list: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let find = |sid: String| list.iter().find(|row| row["session_id"] == sid.as_str()).cloned().unwrap();
    assert_eq!(find(lab.sid(l))["state"], serde_json::Value::Null);
    assert_eq!(find(lab.sid(n))["state"]["attachments"], 0);
}

#[test]
fn shipped_first_release_list_shows_new_daemons_with_optional_fields() {
    let shipped = worktree().join("target/debug/climon2-cli");
    assert!(shipped.exists(), "build it first: cargo build -j1 -p climon2-cli (from the worktree root)");
    let mut lab = Lab::new("t17");
    let home = lab.home("h");
    let n = lab.start(&home, false);
    let l = lab.start(&home, true);
    let output = Command::new(&shipped)
        .args(["list", "--json"])
        .env("CLIMON2_HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let list: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let ids: Vec<_> = list.iter().map(|row| row["session_id"].as_str().unwrap().to_owned()).collect();
    assert!(ids.contains(&lab.sid(n)) && ids.contains(&lab.sid(l)), "{ids:?}");
}

#[test]
fn enclosing_daemon_outside_the_visible_runtime_directories_fails_closed() {
    // Stands in for sudo/su (another uid's runtime directories are invisible) and
    // any daemon the client cannot enumerate: the kernel still shows a daemon as
    // the terminal's owner, so the client must not treat the attach as top-level.
    let mut visible = Lab::new("t19");
    let home = visible.home("h");
    let t = visible.start(&home, false);
    let mut hidden = Lab::new("t19x");
    let other = hidden.home("h");
    let x = hidden.start(&other, false);
    let extra = format!("NEST_RUNTIME_ROOT='{}'", visible.root.display());
    let result = outcome(&hidden.attach_inside(x, &home, &visible.sid(t), &extra));
    assert!(result.rejected() && result.line.contains("not reachable"), "{result:?}");
    assert_eq!(visible.hello(t)["state"]["attachments"], 0);
}
