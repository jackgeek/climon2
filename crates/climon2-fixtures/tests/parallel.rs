use climon2_test_term::{Isolation, Terminal, bin_path};
use std::{sync::Barrier, time::Duration};

/// #21.5: concurrent real PTYs start independently and leave no children behind.
#[test]
fn parallel_terminals_keep_their_own_children_and_output() {
    for burst in 0..4 {
        let start = Barrier::new(32);
        std::thread::scope(|scope| {
            for worker in 0..32 {
                let start = &start;
                scope.spawn(move || {
                    let isolation = Isolation::new();
                    let marker = format!("parallel-{burst}-{worker}");
                    let mut command = isolation.command(bin_path("climon2-fixture"));
                    command.args(["print", &marker, "exit", "7"]);
                    start.wait();
                    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
                    terminal
                        .wait_for_text(&marker, Duration::from_secs(5))
                        .unwrap();
                    assert_eq!(terminal.wait_exit(Duration::from_secs(5)).unwrap(), 7);
                    drop(terminal);
                    isolation.assert_clean();
                });
            }
        });
    }
}
