//! Both post-hello protocols at once: one session control shared by
//! private-protocol and revision-1 connections. These tests prove the
//! coexistence itself, and the private protocol's own answers where a
//! revision-1 test drives the session, so #104 deletes this module with
//! the private protocol; every revision-1 behaviour they touch has its own
//! revision-1 test.
use super::{
    committed_kill, exited,
    interactive::{geometry, ready_attachment},
    kill,
    limits::{LAST, issued},
    peer, request, terminating,
};
use crate::Session;
use climon2_core::{
    hello::Lifecycle,
    protocol::{ClientMessage, ServerMessage as PrivateMessage, TerminalSize},
    wire::{GrantReason, RevocationReason},
};

/// COM-01 (Contract, ordinary attach parity) and COM-03 (Contract, Acquire
/// and Leave): takeover, hand-back and the leave policy match the private
/// protocol, across both protocols. An attach or Acquire takes control at
/// once and revokes the previous controller after its release; a departing
/// controller, by Leave or by loss, hands control back to the most recently
/// controlling remaining attachment (`hand_back`); a final deliberate Leave
/// keeps the session running, detached, and a final unexpected departure
/// ends it.
#[test]
fn takeover_hand_back_and_leave_policy_match_the_private_protocol() {
    let mut session = Session::start("exit");
    let mut first = ready_attachment(&session, &[]);
    let mut private = crate::attach_at(&session, TerminalSize { cols: 91, rows: 37 });
    first.revoked(RevocationReason::Takeover);
    let (restoration, _) = crate::restoration(&mut private);
    assert_eq!(restoration, 2, "the private protocol names restoration 2");
    crate::send(
        &mut private,
        ClientMessage::Ready {
            period: restoration,
        },
    );

    first.acquire(geometry(91, 37));
    assert_eq!((first.period, first.restoration), (3, 3));
    let (released, revoked) = crate::until_terminal(&mut private);
    assert_eq!(revoked, PrivateMessage::Revoked);
    assert!(
        released.ends_with(&crate::release_at(3, 14)),
        "{released:?}"
    );
    first.confirm_ready();

    let mut second = ready_attachment(&session, &[]);
    first.revoked(RevocationReason::Takeover);
    assert_eq!((second.period, second.restoration), (4, 4));
    second.leave();
    first.granted(GrantReason::HandBack, geometry(91, 37));
    assert_eq!((first.period, first.restoration), (5, 5));
    first.confirm_ready();
    let third = ready_attachment(&session, &[]);
    first.revoked(RevocationReason::Takeover);
    drop(third);
    first.granted(GrantReason::HandBack, geometry(91, 37));
    assert_eq!((first.period, first.restoration), (7, 7));
    first.confirm_ready();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 2, true)
    );
    crate::send(&mut private, ClientMessage::Detach);
    assert_eq!(
        crate::until_terminal(&mut private).1,
        PrivateMessage::Detached
    );
    first.leave();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let last = ready_attachment(&session, &[]);
    assert_eq!((last.period, last.restoration), (8, 8));
    drop(last);
    session.finish();
}

/// LIM-05 (Contract and Fault, through the `CLIMON2_DAEMON_ISSUED` test
/// hook), private protocol: once the last control period and restoration ID
/// are issued to a revision-1 controller, a private attach is refused with
/// `resource_limit`, as an interactive Open is.
#[test]
fn a_private_attach_is_refused_once_the_last_period_is_issued() {
    let mut session = issued(LAST - 1, LAST - 1);
    let mut last = ready_attachment(&session, &[]);
    let mut private = session.connect();
    let size = TerminalSize { cols: 91, rows: 37 };
    crate::send(&mut private, ClientMessage::Attach(size));
    let refusal = PrivateMessage::Error {
        message: "attach refused: resource_limit".into(),
    };
    assert_eq!(crate::receive(&mut private), refusal);
    last.typed(b"end\n");
    last.ended(&exited(0));
    session.finish();
}

/// COM-03 (Contract, Kill cases), private protocol: a private kill joins the
/// termination a standalone Kill committed, or that a session failure began,
/// and gets the private protocol's answer for how it ended: `Terminated`
/// 137 for the SIGKILL that ends a command ignoring SIGHUP, `Terminated` 125
/// when the end of the hang-up fails (`end-hangup`), and `Error` when the
/// cleanup (`cleanup`) or a write to the command's terminal (`pty-write`)
/// fails.
#[test]
fn a_private_kill_joins_a_revision1_termination_with_its_own_answer() {
    let cases = [
        (None, PrivateMessage::Terminated { code: 137 }),
        (Some("end-hangup"), PrivateMessage::Terminated { code: 125 }),
        (
            Some("cleanup"),
            PrivateMessage::Error {
                message: "session failed; see daemon log".into(),
            },
        ),
        (
            Some("pty-write"),
            PrivateMessage::Error {
                message: "session failed; see daemon log".into(),
            },
        ),
    ];
    for (fault, answer) in cases {
        let mut session = Session::configured("ignore-hup", |command, _| {
            if let Some(fault) = fault {
                command.env("CLIMON2_DAEMON_FAULT", fault);
            }
        });
        let mut controlling = ready_attachment(&session, &[]);
        let killer = if fault == Some("pty-write") {
            controlling.typed(b"go\n");
            terminating(&session);
            let mut killer = peer(&session);
            request(&mut killer, &kill(1, 1, &[]));
            killer
        } else {
            committed_kill(&session)
        };
        let mut private = session.connect();
        crate::send(&mut private, ClientMessage::Kill);
        assert_eq!(crate::receive(&mut private), answer, "{fault:?}");
        drop((killer, controlling));
        session.finish();
    }
}
