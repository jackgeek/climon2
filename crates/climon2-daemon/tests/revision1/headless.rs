//! Daemon lifecycle over revision 1 (#103), the headless policy of #78: a
//! headless session's lifetime with zero clients and with revision-1
//! attachments that come and go.
use super::{
    end_by_kill, exited,
    interactive::{Attached, geometry, ready_attachment},
};
use crate::{Session, headless};
use climon2_core::hello::Lifecycle;

/// HDL-01 (Contract layer), zero clients from startup: a headless session
/// advertises `detached` from startup on and outlives the five-second
/// initial attach window. The reference ordinary session starts after it,
/// so its end at its own initial deadline shows that window has passed for
/// the headless one, whose command still runs and reports its exit to a
/// later revision-1 attachment.
#[test]
fn a_headless_session_advertises_detached_and_outlives_the_initial_attach_window() {
    let mut session = headless("exit");
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    let mut ordinary = Session::start("early");
    assert_eq!(
        crate::advertised_state(&ordinary),
        (Lifecycle::Starting, 0, false)
    );
    ordinary.finish();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut attached = ready_attachment(&session, &[]);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    attached.typed(b"go\n");
    attached.ended(&exited(42));
    assert_eq!(crate::count(&attached.display, b"final-bytes"), 1);
    session.finish();
}

/// HDL-01 (Contract layer), command completion with zero clients and no
/// initial attach hold: a headless session whose command ends before
/// anything attaches ends with it, cleaned up, without keeping the exit for
/// an initial attachment. The ordinary reference session starts first and
/// nothing attaches to it, so its initial attach window closes before a
/// later session's would: it still advertising `starting` once the headless
/// session is gone shows the headless one held nothing until its own
/// deadline. A standalone Kill then ends the reference.
#[test]
fn a_headless_session_ends_with_its_command_without_the_initial_attach_hold() {
    let ordinary = Session::start("exit");
    let mut session = headless("early");
    session.finish();
    assert_eq!(
        crate::advertised_state(&ordinary),
        (Lifecycle::Starting, 0, false)
    );
    end_by_kill(ordinary);
}

/// HDL-01 (Contract layer), later attachments come and go: a headless
/// session stays `detached` after its last revision-1 attachment leaves and
/// after its last attachment is lost, a loss that ends an ordinary session
/// (`every_unexpected_departure_hangs_up_then_kills_after_the_real_grace`).
/// A later attachment still restores it, and its command still runs and
/// reports its exit.
#[test]
fn a_headless_session_survives_the_leave_and_the_loss_of_its_last_attachment() {
    let mut session = headless("exit");
    ready_attachment(&session, &[]).leave();
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let second = ready_attachment(&session, &[]);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Attached, 1, true)
    );
    // Shutting the socket down, unlike closing this process's descriptor,
    // which a child forked meanwhile by another test thread shares until it
    // execs, makes its EOF readable before the probe connects. The daemon
    // services connections in acceptance order, so the probe's hello is
    // answered after the loss.
    second.stream.shutdown(std::net::Shutdown::Both).unwrap();
    drop(second);
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );

    let mut third = Attached::open(&session, geometry(91, 37), &[], geometry(91, 37));
    assert_eq!(crate::count(&third.display, b"fixture-ready"), 1);
    third.confirm_ready();
    third.typed(b"go\n");
    third.ended(&exited(42));
    assert_eq!(crate::count(&third.display, b"final-bytes"), 1);
    session.finish();
}

/// HDL-01 (Contract layer), Kill: a headless session with zero clients ends
/// on a standalone Kill, confirmed only after cleanup.
#[test]
fn kill_ends_a_headless_session_with_zero_clients() {
    let session = headless("exit");
    assert_eq!(
        crate::advertised_state(&session),
        (Lifecycle::Detached, 0, false)
    );
    end_by_kill(session);
}
