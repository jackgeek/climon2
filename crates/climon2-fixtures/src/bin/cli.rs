use std::{
    io::{self, Read, Write},
    os::unix::ffi::OsStrExt,
};

fn hex(value: &std::ffi::OsStr) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn environment() -> std::collections::BTreeMap<String, String> {
    std::env::vars_os()
        .map(|(key, value)| (hex(&key), hex(&value)))
        .collect()
}

fn main() {
    if let Some(path) = std::env::var_os("FIXTURE_PID_PATH") {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        Some("environment") => println!("{}", serde_json::to_string(&environment()).unwrap()),
        Some("report") => {
            let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
            assert_eq!(
                unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
                0
            );
            let size = unsafe { size.assume_init() };
            let report = serde_json::json!({
                "argv": args[1..].iter().map(|arg| hex(arg)).collect::<Vec<_>>(),
                "cwd": hex(std::env::current_dir().unwrap().as_os_str()),
                "env": environment(), "cols": size.ws_col, "rows": size.ws_row
            })
            .to_string();
            if let Some(path) = std::env::var_os("FIXTURE_REPORT_PATH") {
                std::fs::write(path, &report).unwrap();
            }
            println!("{report}");
            println!("FIXTURE_READY");
            io::stdout().flush().unwrap();
            if std::env::var_os("FIXTURE_REPORT_NO_WAIT").is_none() {
                let mut line = String::new();
                io::stdin().read_line(&mut line).unwrap();
            }
        }
        Some("exit") => std::process::exit(args[1].to_str().unwrap().parse().unwrap()),
        Some("size") => {
            let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
            assert_eq!(
                unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
                0
            );
            let size = unsafe { size.assume_init() };
            println!("SIZE:{}x{}", size.ws_col, size.ws_row);
            io::stdout().flush().unwrap();
            let mut line = String::new();
            io::stdin().read_line(&mut line).unwrap();
        }
        Some("removed-cwd") => {
            use std::os::unix::process::CommandExt;
            std::fs::remove_dir(std::env::current_dir().unwrap()).unwrap();
            let error = std::process::Command::new(&args[1])
                .arg("exec")
                .arg(&args[2])
                .arg("size")
                .exec();
            panic!("{error}");
        }
        Some("signal") => {
            unsafe { libc::raise(args[1].to_str().unwrap().parse().unwrap()) };
        }
        Some("hold") => {
            println!("FIXTURE_READY");
            io::stdout().flush().unwrap();
            let mut line = String::new();
            loop {
                line.clear();
                if io::stdin().read_line(&mut line).unwrap() == 0 {
                    break;
                }
                if line.trim() == "exit" {
                    break;
                }
                let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
                assert_eq!(
                    unsafe { libc::ioctl(0, libc::TIOCGWINSZ, size.as_mut_ptr()) },
                    0
                );
                let size = unsafe { size.assume_init() };
                println!("SIZE:{}x{}", size.ws_col, size.ws_row);
                io::stdout().flush().unwrap();
            }
        }
        Some("tui") => {
            let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
            assert_eq!(unsafe { libc::tcgetattr(0, termios.as_mut_ptr()) }, 0);
            let mut termios = unsafe { termios.assume_init() };
            unsafe {
                libc::cfmakeraw(&mut termios);
            }
            assert_eq!(unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) }, 0);
            print!(
                "\x1b[?1049h\x1b[?1000h\x1b[?2004h\x1b[?1h\
                 \x1b[2J\x1b[4;5HRESTORED-DISPLAY"
            );
            io::stdout().flush().unwrap();
            std::fs::write("draw-count", b"1").unwrap();
            if let Some(path) = std::env::var_os("FIXTURE_TUI_READY") {
                std::fs::write(path, b"").unwrap();
            }
            let mut byte = [0];
            io::stdin().read_exact(&mut byte).unwrap();
            std::fs::write("input-accepted", byte).unwrap();
            print!("INPUT-ACCEPTED");
            std::process::exit(42);
        }
        _ => panic!("unknown fixture mode"),
    }
}
