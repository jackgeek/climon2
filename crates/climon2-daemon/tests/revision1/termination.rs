//! Command final and standalone Kill (#88): how the end of a session reaches
//! revision-1 connections, beside the private protocol's results.
use super::{assert_final_then_closed, exited, interactive::ready_attachment, monitor, signalled};
use crate::Session;
use climon2_core::protocol::ServerMessage as PrivateMessage;

/// COM-06 (Contract): exit zero, an arbitrary nonzero exit and a signal each
/// reach every opened revision-1 connection as exactly one Command final
/// with that result, and exit 143 is not signal 15. A passive monitor
/// connection's stream then ends; an interactive attachment's control ends
/// with the session (`session_ending`) and its stream closes. The private
/// protocol keeps its mapping: its attachment gets `Exit` with 0, 143 and
/// 143.
#[test]
fn command_final_reports_each_exit_and_signal_once() {
    let cases = [
        ("exit", exited(0), 0),
        ("exit 143", exited(143), 143),
        ("signal 15", signalled(15), 143),
    ];
    for (script, result, private_code) in cases {
        let mut session = Session::start("script");
        let mut private = session.attach();
        session.wait_marker(&mut private);
        let (mut watching, _) = monitor(&session, &[]);
        let mut attached = ready_attachment(&session, &[]);
        assert_eq!(
            crate::until_terminal(&mut private).1,
            PrivateMessage::Revoked
        );
        crate::issue(&session, script);
        assert_final_then_closed(&mut watching, &result);
        attached.ended(&result);
        assert_eq!(
            crate::until_terminal(&mut private).1,
            PrivateMessage::Exit { code: private_code },
            "{script}"
        );
        session.finish();
    }
}
