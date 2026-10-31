use native_scrollback_prototype::{restore, row, sample};
use std::{error::Error, fs, path::Path};

fn main() {
    if let Err(error) = run() {
        eprintln!("native-scrollback-prototype: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        println!(
            "Throwaway #52 fixture: --write-dir NEW_DIRECTORY (files only; no terminal output)"
        );
        return Ok(());
    }
    if args.len() != 2 || args[0] != "--write-dir" {
        return Err("expected --write-dir NEW_DIRECTORY".into());
    }
    let mut prime = b"SAVED-SENTINEL\r\n".to_vec();
    for n in 0..6 {
        prime.extend_from_slice(format!("VISIBLE-{n}\r\n").as_bytes());
    }
    let normal = restore(&sample())?;
    let mut alternate = sample();
    alternate.alternate = Some((0..6).map(|n| row(&format!("ALT-{n}"))).collect());
    let alternate = restore(&alternate)?;
    let artifacts = [
        ("01-prime.ansi", prime),
        ("02-normal.ansi", normal.clone()),
        ("03-regain-full-resend.ansi", normal),
        ("04-alternate.ansi", alternate),
        ("05-return-normal.ansi", b"\x1b[?1049l".to_vec()),
        (
            "06-application-output.ansi",
            b"\r\nAPPLICATION-ECHO\r\n".to_vec(),
        ),
    ];
    let directory = Path::new(&args[1]);
    fs::create_dir(directory)?;
    for (name, bytes) in artifacts {
        fs::write(directory.join(name), bytes)?;
    }
    fs::write(
        directory.join("MANIFEST.txt"),
        include_str!("../MANIFEST.txt"),
    )?;
    println!("Wrote seven bounded fixture files. Native hosts remain UNTESTED.");
    Ok(())
}
