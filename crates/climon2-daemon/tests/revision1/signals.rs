//! Program status (OSC 7501) and shell markers (OSC 133) drive attention
//! detection (#92): the session daemon interprets them in the command's
//! output, before filtering, and reports the effective status through
//! `status-v1`, driven by the `test-hooks` controllable clock. Time stands
//! still until a test advances it, so a signal takes effect at the clock the
//! test set, and a sample is taken at the first advance at least 1,500 ms
//! after the last sample or fresh baseline, clamped to an armed deadline.
//! The fixture prints each sequence with `print`, `\e` standing for ESC and
//! `\e\` for ST.
use super::{
    advance, clocked, exited,
    interactive::{Attached, ready_attachment},
    status,
    status::{
        acknowledged, all_changed, all_quiet, completed, printed, resized, shown, typed, viewing,
        viewing_ok, watched,
    },
    title_changed,
};
use crate::Session;
use climon2_core::wire::StatusState::{Acknowledged, Idle, NeedsAttention, Running};

/// The exact answer to an OSC 7501 capability query: every state and kind
/// of Program Status Protocol revision 0.4.
const QUERY_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// No observed sequence reached the controller's host output.
fn nothing_observed_displayed(attached: &Attached) {
    for sequence in [&b"\x1b]7501"[..], b"\x1b]133"] {
        assert_eq!(
            crate::count(&attached.display, sequence),
            0,
            "{:?}",
            String::from_utf8_lossy(&attached.display)
        );
    }
}

/// STA-03 (Contract, controllable clock): a capability query and child
/// reports, `working` under `id=build` and `done` under `id=build/test`,
/// neither activate explicit detection nor aggregate into activity or
/// attention: heuristics still see the output after them. A root `working`
/// report then stays running silently, past the ten-second inactivity
/// deadline and to 40 seconds, with no heartbeat. Root `done` requests
/// attention; repeated `done` and `blocked` join that episode; after
/// acknowledgement root `idle` starts the next one. `done` is no session
/// completion: no Command final comes, and accepted input clears it to
/// running and then idle. Root `error` requests attention too. No observed
/// sequence reaches host output.
#[test]
fn root_reports_drive_explicit_detection_while_children_and_queries_do_not() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let ignored = concat!(
        r"\e]7501;?\e\",
        r"\e]7501;state=working:id=build\e\",
        r"\e]7501;state=done:id=build/test\e\",
        "busy",
    );
    printed(&session, &mut attached, ignored, 1);
    advance(&session, 1_500);
    let all = &mut [&mut watcher];
    all_changed(&mut attached, all, &status(Running, 1_500, None));
    printed(&session, &mut attached, r"\e]7501;state=working\e\", 2);
    for ms in [11_500, 25_000, 40_000] {
        advance(&session, ms);
        all_quiet(&mut attached, all);
    }

    printed(&session, &mut attached, r"\e]7501;state=done\e\", 3);
    let first = status(NeedsAttention, 40_000, Some(1));
    all_changed(&mut attached, all, &first);
    let repeated = concat!(r"\e]7501;state=done\e\", r"\e]7501;state=blocked\e\");
    printed(&session, &mut attached, repeated, 4);
    all_quiet(&mut attached, all);
    assert_eq!(viewing(&mut attached, true), viewing_ok(true, true));
    let acknowledged_status = status(Acknowledged, 40_000, Some(1));
    acknowledged(&mut attached, &acknowledged_status);
    all[0].changed(&acknowledged_status);
    printed(&session, &mut attached, r"\e]7501;state=idle\e\", 5);
    all_changed(&mut attached, all, &status(NeedsAttention, 40_000, Some(2)));

    let running = status(Running, 40_000, None);
    typed(&mut attached, b"x", Some(&running));
    all[0].changed(&running);
    advance(&session, 42_000);
    all_changed(&mut attached, all, &status(Idle, 42_000, None));
    printed(&session, &mut attached, r"\e]7501;state=error\e\", 6);
    all_changed(&mut attached, all, &status(NeedsAttention, 42_000, Some(3)));
    nothing_observed_displayed(&attached);
    completed(session, &mut attached, all, 42_000);
}

/// STA-03 and STA-05 (Contract, controllable clock): the user window's end
/// restores the remembered root `working` of the current explicit scope,
/// without the application repeating it, so the session stays running. An
/// OSC 133 `D` retires that scope to idle, with no attention; the next
/// window then ends idle.
#[test]
fn user_window_expiry_restores_explicit_working_only_in_its_scope() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let all = &mut [&mut watcher];
    printed(&session, &mut attached, r"\e]7501;state=working\e\", 1);
    all_changed(&mut attached, all, &status(Running, 0, None));
    typed(&mut attached, b"a", None);
    for ms in [1_999, 2_000, 30_000] {
        advance(&session, ms);
        all_quiet(&mut attached, all);
    }
    printed(&session, &mut attached, r"\e]133;D\e\", 2);
    all_changed(&mut attached, all, &status(Idle, 30_000, None));
    let running = status(Running, 30_000, None);
    typed(&mut attached, b"b", Some(&running));
    all[0].changed(&running);
    advance(&session, 32_000);
    all_changed(&mut attached, all, &status(Idle, 32_000, None));
    completed(session, &mut attached, all, 32_000);
}

/// STA-04 (Contract, controllable clock): root `clear` ends explicit
/// working to idle, without attention, and starts the ten-second fallback
/// quiet deadline. Changed characters, then a changed title, then a resize,
/// each seen a millisecond before the deadline then current, restart it
/// without becoming agent work; it ends at exactly ten seconds of quiet,
/// with no attention request, and heuristics then see the next change.
/// Root `error` requests attention and starts the grace again; input during
/// it is user work whose two-second window it does not extend, and its
/// expiry requests nothing.
#[test]
fn error_and_clear_start_the_fallback_quiet_deadline() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let all = &mut [&mut watcher];
    printed(&session, &mut attached, r"\e]7501;state=working\e\", 1);
    all_changed(&mut attached, all, &status(Running, 0, None));
    advance(&session, 1_000);
    printed(&session, &mut attached, r"\e]7501;state=clear\e\", 2);
    all_changed(&mut attached, all, &status(Idle, 1_000, None));

    shown(&session, &mut attached, "grace-change");
    advance(&session, 10_999);
    all_quiet(&mut attached, all);
    crate::script(&session, "title grace-title");
    assert_eq!(
        title_changed(&mut all[0].stream).as_deref(),
        Some("grace-title")
    );
    advance(&session, 20_998);
    all_quiet(&mut attached, all);
    resized(&mut attached, 100, 30);
    advance(&session, 30_997);
    all_quiet(&mut attached, all);
    for ms in [40_996, 40_997] {
        advance(&session, ms);
        all_quiet(&mut attached, all);
    }
    shown(&session, &mut attached, "after-grace");
    advance(&session, 42_497);
    all_changed(&mut attached, all, &status(Running, 42_497, None));

    printed(&session, &mut attached, r"\e]7501;state=error\e\", 3);
    all_changed(&mut attached, all, &status(NeedsAttention, 42_497, Some(1)));
    let running = status(Running, 42_497, None);
    typed(&mut attached, b"x", Some(&running));
    all[0].changed(&running);
    advance(&session, 44_497);
    all_changed(&mut attached, all, &status(Idle, 44_497, None));
    advance(&session, 60_000);
    all_quiet(&mut attached, all);
    completed(session, &mut attached, all, 60_000);
}

/// STA-04 (Contract, controllable clock): in heuristic detection a full
/// reset (`ESC c`) only refreshes the baseline, so the screen it cleared is
/// no change and the agent deadline it found still requests attention on
/// time; a soft reset (`CSI ! p`) changes nothing. In explicit detection a
/// full reset behaves as root `clear`: idle, no attention, and the grace,
/// which an OSC 133 `D` ends at once, so heuristics see the next change.
/// A fresh root report ends the grace too. The command's exit during the
/// grace is completed, carried by Command final, and nothing follows it.
#[test]
fn resets_reports_markers_and_the_final_exit_end_or_keep_detection_as_specified() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let all = &mut [&mut watcher];
    shown(&session, &mut attached, "before-reset");
    advance(&session, 1_500);
    all_changed(&mut attached, all, &status(Running, 1_500, None));
    printed(&session, &mut attached, r"\e[!p\ec", 1);
    for ms in [3_000, 11_499] {
        advance(&session, ms);
        all_quiet(&mut attached, all);
    }
    advance(&session, 11_500);
    all_changed(&mut attached, all, &status(NeedsAttention, 11_500, Some(1)));

    let running = status(Running, 11_500, None);
    typed(&mut attached, b"x", Some(&running));
    all[0].changed(&running);
    printed(&session, &mut attached, r"\e]7501;state=working\e\\ec", 2);
    all_changed(&mut attached, all, &status(Idle, 11_500, None));
    advance(&session, 21_499);
    all_quiet(&mut attached, all);
    printed(&session, &mut attached, r"\e]133;D\e\", 3);
    all_quiet(&mut attached, all);
    shown(&session, &mut attached, "after-boundary");
    advance(&session, 22_999);
    all_changed(&mut attached, all, &status(Running, 22_999, None));

    printed(&session, &mut attached, r"\e]7501;state=clear\e\", 4);
    all_changed(&mut attached, all, &status(Idle, 22_999, None));
    printed(&session, &mut attached, r"\e]7501;state=blocked\e\", 5);
    all_changed(&mut attached, all, &status(NeedsAttention, 22_999, Some(2)));
    let running = status(Running, 22_999, None);
    typed(&mut attached, b"y", Some(&running));
    all[0].changed(&running);
    advance(&session, 24_999);
    all_changed(&mut attached, all, &status(Idle, 24_999, None));
    printed(&session, &mut attached, r"\e]7501;state=error\e\", 6);
    all_changed(&mut attached, all, &status(NeedsAttention, 24_999, Some(3)));
    completed(session, &mut attached, all, 24_999);
}

/// STA-05 (Contract, controllable clock): OSC 133 `A` retires explicit
/// working to idle, with no attention, and prompt rendering after it is no
/// activity at any sample; accepted input and a root report during it still
/// apply. `C` retires the scope and ends the exclusion, so the command's
/// output after it is agent work. `D`, with an exit status of 1, retires
/// that work to idle with no attention, no failure and no Command final,
/// and a repeated `D` changes nothing; no deadline is left. `B` ends a later
/// prompt's exclusion with a fresh baseline, so the prompt rendered before
/// it is no change but output after it is. No observed sequence reaches
/// host output.
#[test]
fn shell_markers_retire_scopes_and_exclude_prompt_rendering() {
    let session = clocked("script");
    let (mut attached, mut watcher) = watched(&session);
    let all = &mut [&mut watcher];
    printed(&session, &mut attached, r"\e]7501;state=working\e\", 1);
    all_changed(&mut attached, all, &status(Running, 0, None));
    printed(&session, &mut attached, r"\e]133;A\e\first-prompt$", 2);
    all_changed(&mut attached, all, &status(Idle, 0, None));
    for ms in [1_500, 5_000] {
        advance(&session, ms);
        all_quiet(&mut attached, all);
    }
    let running = status(Running, 5_000, None);
    typed(&mut attached, b"l", Some(&running));
    all[0].changed(&running);
    advance(&session, 7_000);
    all_changed(&mut attached, all, &status(Idle, 7_000, None));
    printed(&session, &mut attached, r"\e]7501;state=done\e\", 3);
    all_changed(&mut attached, all, &status(NeedsAttention, 7_000, Some(1)));
    let running = status(Running, 7_000, None);
    typed(&mut attached, b"s", Some(&running));
    all[0].changed(&running);
    advance(&session, 9_000);
    all_changed(&mut attached, all, &status(Idle, 9_000, None));

    printed(&session, &mut attached, r"\e]133;C\e\command-output", 4);
    all_quiet(&mut attached, all);
    advance(&session, 10_500);
    all_changed(&mut attached, all, &status(Running, 10_500, None));
    printed(&session, &mut attached, r"\e]133;D;1\e\", 5);
    all_changed(&mut attached, all, &status(Idle, 10_500, None));
    printed(&session, &mut attached, r"\e]133;D\e\", 6);
    all_quiet(&mut attached, all);
    advance(&session, 25_000);
    all_quiet(&mut attached, all);

    printed(&session, &mut attached, r"\e]133;A\e\second-prompt$", 7);
    advance(&session, 26_500);
    all_quiet(&mut attached, all);
    printed(&session, &mut attached, r"\e]133;B\e\", 8);
    advance(&session, 28_000);
    all_quiet(&mut attached, all);
    shown(&session, &mut attached, "after-input-began");
    advance(&session, 29_500);
    all_changed(&mut attached, all, &status(Running, 29_500, None));
    nothing_observed_displayed(&attached);
    completed(session, &mut attached, all, 29_500);
}

/// STA-03 (Contract): the session daemon answers an OSC 7501 capability
/// query itself, with every state and kind it supports, before the engine's
/// answer to the device-attributes query that follows it, as the upstream
/// detection procedure needs. The program receives exactly those replies,
/// and the query never reaches the controller.
#[test]
fn program_status_queries_are_answered_locally_in_order() {
    let mut session = Session::start("status-query-record");
    let mut attached = ready_attachment(&session, &[]);
    attached.typed(b"x");
    let path = session.directory.path().join("recorded");
    crate::wait_contents(&path, |contents| contents.ends_with(b"\x1b[?6c"));
    attached.typed(b"END");
    attached.ended(&exited(0));
    session.finish();
    let expected = [&b"x"[..], QUERY_REPLY, b"\x1b[?6c", b"END"].concat();
    let recorded = std::fs::read(&path).unwrap();
    assert_eq!(
        recorded,
        expected,
        "{:?}",
        String::from_utf8_lossy(&recorded)
    );
    nothing_observed_displayed(&attached);
}
