//! PROTOTYPE: pure admission decision over a snapshot of the per-user nesting graph.
//!
//! Edges are held by the *target* daemon (one per active nested attachment, until
//! disconnect), so the snapshot is "every live daemon and the enclosers of its
//! attachments". Adding edges E->T creates a cycle iff T already reaches E, i.e.
//! T is found walking E's incoming edges backwards.
use std::collections::{BTreeSet, VecDeque};

pub type Sid = String;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Node {
    /// Hello advertised `nesting.incoming`: every encloser of every attachment.
    Capable { sid: Sid, pid: u32, incoming: Vec<Sid> },
    /// Answered hello without the optional field: cannot take part.
    Legacy { sid: Sid, pid: u32 },
    /// A socket in a runtime directory that did not answer hello in time.
    Unresponsive { pid: Option<u32> },
}

impl Node {
    pub fn sid(&self) -> Option<&str> {
        match self {
            Self::Capable { sid, .. } | Self::Legacy { sid, .. } => Some(sid),
            Self::Unresponsive { .. } => None,
        }
    }

    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Capable { pid, .. } | Self::Legacy { pid, .. } => Some(*pid),
            Self::Unresponsive { pid } => *pid,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reject {
    UnknownTarget,
    UnsupportedTarget,
    SelfAttach,
    Cycle { via: Sid },
    /// Safety cannot be established; fail closed.
    Unprovable(String),
}

impl std::fmt::Display for Reject {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTarget => write!(out, "unknown target session"),
            Self::UnsupportedTarget => write!(out, "target session does not support nesting-safe attach"),
            Self::SelfAttach => write!(out, "cannot attach a session to itself"),
            Self::Cycle { via } => write!(out, "attachment would create a cycle through {via}"),
            Self::Unprovable(why) => write!(out, "nesting safety cannot be established: {why}"),
        }
    }
}

/// Enclosers must already be resolved to session IDs of live nodes.
pub fn admit(target: &str, enclosers: &BTreeSet<Sid>, nodes: &[Node]) -> Result<(), Reject> {
    match find(nodes, target) {
        None => return Err(Reject::UnknownTarget),
        Some(Node::Capable { .. }) => {}
        Some(_) => return Err(Reject::UnsupportedTarget),
    }
    if enclosers.contains(target) {
        return Err(Reject::SelfAttach);
    }
    let any_unresponsive = nodes.iter().any(|node| matches!(node, Node::Unresponsive { .. }));
    let mut seen = BTreeSet::new();
    let mut queue: VecDeque<Sid> = enclosers.iter().cloned().collect();
    while let Some(sid) = queue.pop_front() {
        if !seen.insert(sid.clone()) {
            continue;
        }
        if sid == target {
            return Err(Reject::Cycle { via: sid });
        }
        match find(nodes, &sid) {
            Some(Node::Capable { incoming, .. }) => queue.extend(incoming.iter().cloned()),
            Some(_) => return Err(Reject::Unprovable(format!("session {sid} cannot report its attachments"))),
            None if any_unresponsive => {
                return Err(Reject::Unprovable(format!("session {sid} is unresolved while a daemon is unresponsive")));
            }
            // Every socket answered and none is this session: it has ended, so it
            // cannot be on a path any more.
            None => {}
        }
    }
    Ok(())
}

fn find<'a>(nodes: &'a [Node], sid: &str) -> Option<&'a Node> {
    nodes.iter().find(|node| node.sid() == Some(sid))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(sid: &str, incoming: &[&str]) -> Node {
        Node::Capable { sid: sid.into(), pid: 0, incoming: incoming.iter().map(|s| s.to_string()).collect() }
    }
    fn enc(sids: &[&str]) -> BTreeSet<Sid> {
        sids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn top_level_attachment_is_admitted() {
        assert_eq!(admit("b", &enc(&[]), &[cap("b", &[])]), Ok(()));
    }

    #[test]
    fn acyclic_nesting_is_admitted() {
        assert_eq!(admit("b", &enc(&["a"]), &[cap("a", &[]), cap("b", &[])]), Ok(()));
    }

    #[test]
    fn self_attachment_is_rejected() {
        assert_eq!(admit("a", &enc(&["a"]), &[cap("a", &[])]), Err(Reject::SelfAttach));
    }

    #[test]
    fn indirect_cycle_is_rejected() {
        // a->b, b->c exist; c->a would close the loop.
        let nodes = [cap("a", &[]), cap("b", &["a"]), cap("c", &["b"])];
        assert_eq!(admit("a", &enc(&["c"]), &nodes), Err(Reject::Cycle { via: "a".into() }));
    }

    #[test]
    fn legacy_encloser_fails_closed() {
        let nodes = [Node::Legacy { sid: "l".into(), pid: 0 }, cap("t", &[])];
        assert!(matches!(admit("t", &enc(&["l"]), &nodes), Err(Reject::Unprovable(_))));
    }

    #[test]
    fn legacy_ancestor_on_path_fails_closed() {
        let nodes = [Node::Legacy { sid: "l".into(), pid: 0 }, cap("e", &["l"]), cap("t", &[])];
        assert!(matches!(admit("t", &enc(&["e"]), &nodes), Err(Reject::Unprovable(_))));
    }

    #[test]
    fn unrelated_legacy_session_does_not_block() {
        let nodes = [Node::Legacy { sid: "l".into(), pid: 0 }, cap("e", &[]), cap("t", &[])];
        assert_eq!(admit("t", &enc(&["e"]), &nodes), Ok(()));
    }

    #[test]
    fn legacy_target_is_unsupported() {
        let nodes = [Node::Legacy { sid: "l".into(), pid: 0 }];
        assert_eq!(admit("l", &enc(&[]), &nodes), Err(Reject::UnsupportedTarget));
    }

    #[test]
    fn ended_source_is_pruned_only_when_every_daemon_answered() {
        let nodes = [cap("e", &["gone"]), cap("t", &[])];
        assert_eq!(admit("t", &enc(&["e"]), &nodes), Ok(()));
        let with_hung = [cap("e", &["gone"]), cap("t", &[]), Node::Unresponsive { pid: None }];
        assert!(matches!(admit("t", &enc(&["e"]), &with_hung), Err(Reject::Unprovable(_))));
    }
}
