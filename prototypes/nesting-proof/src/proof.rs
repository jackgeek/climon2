//! PROTOTYPE: which sessions enclose this client, from kernel facts only.
//!
//! * `Tty`: the session leader of the terminal on stdin/stdout. Every climon2
//!   daemon (first release included) is the parent of its terminal's leader.
//! * `Ancestry`: any daemon among this process's ancestors (catches relays that
//!   run inside the session, such as `script` or `expect`).
use crate::{graph::Node, sys};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Source {
    Tty,
    Ancestry,
}

pub fn candidates() -> BTreeMap<u32, BTreeSet<Source>> {
    let mut found: BTreeMap<u32, BTreeSet<Source>> = BTreeMap::new();
    for fd in [0, 1] {
        if let Some(parent) = sys::terminal_leader(fd).and_then(sys::proc_info).map(|i| i.ppid) {
            found.entry(parent).or_default().insert(Source::Tty);
        }
    }
    let mut pid = unsafe { libc::getppid() } as u32;
    while pid > 1 {
        found.entry(pid).or_default().insert(Source::Ancestry);
        match sys::proc_info(pid) {
            Some(info) => pid = info.ppid,
            None => break,
        }
    }
    found
}

pub struct Enclosers {
    pub sessions: BTreeMap<String, BTreeSet<Source>>,
    /// Candidate processes that look like climon2 daemons but are not reachable
    /// in this user's runtime directories (another uid, root, or an unknown home).
    pub unreachable: Vec<u32>,
}

pub fn resolve(candidates: &BTreeMap<u32, BTreeSet<Source>>, nodes: &[Node]) -> Enclosers {
    let mut sessions = BTreeMap::new();
    let mut unreachable = Vec::new();
    for (pid, sources) in candidates {
        match nodes.iter().find(|n| n.pid() == Some(*pid)) {
            Some(node) => match node.sid() {
                Some(sid) => {
                    sessions.insert(sid.to_owned(), sources.clone());
                }
                None => unreachable.push(*pid),
            },
            None if looks_like_daemon(*pid) => unreachable.push(*pid),
            None => {}
        }
    }
    Enclosers { sessions, unreachable }
}

fn looks_like_daemon(pid: u32) -> bool {
    matches!(sys::exe_name(pid).as_deref(), Some("climon2-daemon" | "nest-daemon"))
}

pub fn describe(sources: &BTreeSet<Source>) -> String {
    sources
        .iter()
        .map(|s| match s {
            Source::Tty => "tty",
            Source::Ancestry => "ancestry",
        })
        .collect::<Vec<_>>()
        .join(",")
}
