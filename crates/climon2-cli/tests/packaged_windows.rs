//! #321: the Windows smoke test (docs/for-humans/windows/qualification.md, "Existing
//! scenarios"): the packaged product under the harness's Windows Terminal
//! model. From `pwsh`, `c2 exec pwsh` runs a command, detaches, `c2 list`
//! shows the session, `c2 attach` restores its screen, and `exit 7` in the
//! session reaches the outer shell.
//!
//! The product is the release zip's or this build's (`support::Package`).
//!
//! cargo runs tests in a job that refuses breakaway, and so does the CI
//! runner, so each test runs again outside every job
//! (`climon2_test_term::rerun_outside`), where `c2 exec`'s daemon can leave
//! its host as it does under a real terminal.
#![cfg(windows)]
// Each test binary uses part of it.
#[allow(dead_code)]
mod support;

use climon2_test_term::{Terminal, controlled_pwsh, rerun_outside};
use std::{fs, time::Duration};
use support::{Home, Package, enter, pwsh_arguments, quoted, wait};

climon2_test_term::outside_relay!();

/// How long the whole test may take outside every job.
const OUTSIDE: Duration = Duration::from_secs(240);
/// The escape prefix, Ctrl+], then `d`.
const DETACH: &[u8] = b"\x1dd";
/// Every step waits this long at most.
const TIMEOUT: Duration = support::TIMEOUT;

/// Clears the outer shell's screen, so what shows next was drawn anew.
fn clear(terminal: &mut Terminal) {
    enter(terminal, "Clear-Host");
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("42-ok"))
        .unwrap();
}

#[test]
fn pwsh_from_pwsh_detaches_lists_reattaches_restored_and_exits_seven() {
    if rerun_outside(
        "pwsh_from_pwsh_detaches_lists_reattaches_restored_and_exits_seven",
        OUTSIDE,
    ) {
        return;
    }
    let Some(pwsh) = controlled_pwsh() else {
        return;
    };
    let (package, home) = (Package::new(), Home::new());
    let mut terminal = Terminal::spawn(home.pwsh(&package, &pwsh, "OUTER"), 30, 120).unwrap();
    wait(&mut terminal, &home, "OUTER> ");

    let inner: Vec<String> = pwsh_arguments("INNER")
        .iter()
        .map(|argument| quoted(argument))
        .collect();
    enter(
        &mut terminal,
        &format!(
            "c2 exec {} {}; \"OUTER-STATUS=$LASTEXITCODE\"",
            quoted(&pwsh.to_string_lossy()),
            inner.join(" ")
        ),
    );
    wait(&mut terminal, &home, "INNER> ");
    enter(&mut terminal, "\"$(6*7)-ok\"");
    wait(&mut terminal, &home, "42-ok");
    let [session] = home.sessions().try_into().unwrap_or_else(|_| {
        panic!(
            "{:?} {}",
            fs::read_dir(home.0.path()).map(|d| d.map(|e| e.unwrap().path()).collect::<Vec<_>>()),
            terminal.screen()
        )
    });
    let id = session.session_id.to_string();

    terminal.send(DETACH).unwrap();
    wait(&mut terminal, &home, "OUTER-STATUS=0");
    // The notice is wider than the screen, so it wraps.
    let unwrapped = terminal.screen().replace('\n', "");
    assert!(
        unwrapped.contains(&format!(
            "detached from session {id}; reattach with: c2 attach {id}"
        )),
        "{}",
        terminal.screen()
    );
    clear(&mut terminal);
    enter(&mut terminal, "c2 list");
    wait(&mut terminal, &home, &id[..8]);

    clear(&mut terminal);
    enter(
        &mut terminal,
        &format!(
            "c2 attach {}; \"OUTER-STATUS=$LASTEXITCODE\"; exit $LASTEXITCODE",
            &id[..8]
        ),
    );
    wait(&mut terminal, &home, "42-ok");
    wait(&mut terminal, &home, "INNER> ");
    enter(&mut terminal, "exit 7");
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
    assert!(
        terminal.screen().contains("OUTER-STATUS=7"),
        "{}",
        terminal.screen()
    );
    drop(terminal);
    assert!(home.sessions().is_empty());
}
