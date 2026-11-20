//! Windows lifecycle probes for issue #300. Throwaway measuring code.
//! Each subcommand is one probe (or a helper process a probe starts).
//! Every result is one JSON object per line: to PROBE_OUT when set, else stdout.
#![allow(clippy::missing_safety_doc)]

mod probes;
mod pty;
mod util;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--out") {
        let p = args[1].clone();
        std::env::set_var("PROBE_OUT", p);
        args.drain(0..2);
    }
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    probes::dispatch(&a);
}
