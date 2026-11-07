use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use monitor_selector_prototype::{
    State,
    input::{self, Prefix},
    samples, ui,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    error::Error,
    io,
    time::{Duration, Instant},
};

struct Host;

impl Host {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let host = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        Ok(host)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Err(error) = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show
        ) {
            eprintln!("Prototype terminal cleanup failed: {error}");
        }
        if let Err(error) = terminal::disable_raw_mode() {
            eprintln!("Prototype raw-mode cleanup failed: {error}");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut state = State::new(samples());
    state.tick = 200;
    let mut prefix = Prefix::CtrlBracket;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-hints" => state.hints = false,
            "--escape-prefix" => {
                prefix = match args.next().as_deref() {
                    Some("Ctrl+]") => Prefix::CtrlBracket,
                    Some("Ctrl+[") => Prefix::Escape,
                    _ => {
                        return Err(
                            "Prototype presets: --escape-prefix 'Ctrl+]' or 'Ctrl+['".into()
                        );
                    }
                }
            }
            "--help" => {
                println!(
                    "Throwaway Ratatui selector. No real sessions or daemon/client integration.\n\
                    cargo run --manifest-path prototypes/selector/Cargo.toml -- [--no-hints] [--escape-prefix 'Ctrl+[']\n\
                    Ctrl+], then s: selector; x: exit; j: newest attention. F3: hints.\n\
                    F4: attention/toast; F5: title; F6: connection; F7: remove highlighted row; F8: reset."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown prototype option: {arg}").into()),
        }
    }
    state.prefix_label = prefix.label().into();
    state.open();
    let host = Host::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut last_tick = Instant::now();
    let started = Instant::now();
    let mut hits = ui::Hits::default();
    while !state.exiting {
        state.tick_toast(started.elapsed().as_millis() as u64);
        terminal.draw(|frame| hits = ui::paint(frame, &state))?;
        let size = terminal.size()?;
        let usable = size.width >= 24 && size.height >= 13;
        if event::poll(Duration::from_millis(16))? {
            input::handle(&mut state, event::read()?, &hits, prefix, usable);
        }
        if last_tick.elapsed() >= Duration::from_secs(1) {
            state.advance();
            last_tick = Instant::now();
        }
    }
    drop(host);
    println!(
        "Selector prototype exited; synthetic input events: {}",
        state.input.len()
    );
    Ok(())
}
