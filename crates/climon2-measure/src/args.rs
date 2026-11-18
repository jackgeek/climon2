//! The harness's command line, parsed once where it enters.

use crate::{
    matrix::{GEOMETRIES, Geometry},
    workload::Workload,
};
use std::{path::PathBuf, time::Duration};

pub const USAGE: &str = "\
Usage: climon2-measure run --archive <climon2-*.tar.gz> --out <directory>
                           [--seconds <1..=600>] [--geometry <80x24|200x60|500x200>]... [--only <text>]
       climon2-measure workload --title <title> --rate <bytes/s> [--attention <fifo>] [--heuristic]

run measures the PERF-01 matrix on the exact candidate archive and writes report.json and
report.md to the output directory. --seconds sets each scenario's window (default 60, the
owner's decision; shorter windows are for local proof only and are declared in the report).
--geometry limits the run to those geometries (default all three); --only to case IDs
containing the text. A missed target is a failed Measure case in the report, not a failure
of this command; only a harness error is.
workload is the measured sessions' application.";

/// The default and owner-decided window per scenario (#121, 2026-11-16).
pub const WINDOW: Duration = Duration::from_secs(60);

/// What `run` measures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub archive: PathBuf,
    pub out: PathBuf,
    pub window: Duration,
    pub geometries: Vec<Geometry>,
    pub only: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Run(Run),
    Workload(Workload),
}

/// The value after each flag, or why the arguments are not a command.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let (command, flags) = args.split_first().ok_or("missing command")?;
    let mut values: Vec<(&str, &str)> = Vec::new();
    let mut heuristic = false;
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        if command == "workload" && flag == "--heuristic" {
            heuristic = true;
            continue;
        }
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        values.push((flag, value));
    }
    match command.as_str() {
        "run" => run(&values).map(Command::Run),
        "workload" => workload(&values, heuristic).map(Command::Workload),
        _ => Err(format!("unknown command {command:?}")),
    }
}

/// The one value of `flag`, if given.
fn single<'a>(values: &[(&str, &'a str)], flag: &str) -> Result<Option<&'a str>, String> {
    let mut found = values.iter().filter(|(name, _)| *name == flag);
    let first = found.next().map(|(_, value)| *value);
    if found.next().is_some() {
        return Err(format!("{flag} given twice"));
    }
    Ok(first)
}

fn only_known(values: &[(&str, &str)], known: &[&str]) -> Result<(), String> {
    match values.iter().find(|(flag, _)| !known.contains(flag)) {
        Some((flag, _)) => Err(format!("unknown flag {flag}")),
        None => Ok(()),
    }
}

fn run(values: &[(&str, &str)]) -> Result<Run, String> {
    only_known(
        values,
        &["--archive", "--out", "--seconds", "--geometry", "--only"],
    )?;
    let seconds = match single(values, "--seconds")? {
        None => WINDOW.as_secs(),
        Some(text) => text
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=600).contains(seconds))
            .ok_or("--seconds must be 1 to 600")?,
    };
    let mut geometries: Vec<Geometry> = values
        .iter()
        .filter(|(flag, _)| *flag == "--geometry")
        .map(|(_, text)| text.parse())
        .collect::<Result<_, _>>()?;
    if geometries.is_empty() {
        geometries = GEOMETRIES.to_vec();
    }
    Ok(Run {
        archive: single(values, "--archive")?
            .ok_or("--archive is required")?
            .into(),
        out: single(values, "--out")?.ok_or("--out is required")?.into(),
        window: Duration::from_secs(seconds),
        geometries,
        only: single(values, "--only")?.map(str::to_owned),
    })
}

fn workload(values: &[(&str, &str)], heuristic: bool) -> Result<Workload, String> {
    only_known(values, &["--title", "--rate", "--attention"])?;
    let rate = single(values, "--rate")?.ok_or("--rate is required")?;
    Ok(Workload {
        title: single(values, "--title")?
            .ok_or("--title is required")?
            .to_owned(),
        rate: rate
            .parse()
            .map_err(|_| "--rate must be bytes per second")?,
        attention: single(values, "--attention")?.map(PathBuf::from),
        heuristic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Command, String> {
        parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    /// The whole matrix for 60 seconds per scenario unless narrowed.
    #[test]
    fn run_defaults_to_every_geometry_and_the_60_second_window() {
        assert_eq!(
            parsed(&["run", "--archive", "a.tar.gz", "--out", "o"]),
            Ok(Command::Run(Run {
                archive: "a.tar.gz".into(),
                out: "o".into(),
                window: Duration::from_secs(60),
                geometries: GEOMETRIES.to_vec(),
                only: None,
            }))
        );
        assert_eq!(
            parsed(&[
                "run",
                "--out",
                "o",
                "--geometry",
                "200x60",
                "--seconds",
                "5",
                "--archive",
                "a",
                "--only",
                "/1/",
                "--geometry",
                "80x24",
            ]),
            Ok(Command::Run(Run {
                archive: "a".into(),
                out: "o".into(),
                window: Duration::from_secs(5),
                geometries: vec![GEOMETRIES[1], GEOMETRIES[0]],
                only: Some("/1/".to_owned()),
            }))
        );
    }

    /// The window is 1 to 600 seconds; required and repeated flags are
    /// checked.
    #[test]
    fn run_rejects_missing_repeated_unknown_and_out_of_range_values() {
        let base = ["run", "--archive", "a", "--out", "o"];
        let with = |extra: &[&str]| parsed(&[&base[..], extra].concat());
        assert!(with(&["--seconds", "1"]).is_ok());
        assert!(with(&["--seconds", "600"]).is_ok());
        assert_eq!(
            with(&["--seconds", "0"]),
            Err("--seconds must be 1 to 600".to_owned())
        );
        assert_eq!(
            with(&["--seconds", "601"]),
            Err("--seconds must be 1 to 600".to_owned())
        );
        assert_eq!(with(&["--out", "p"]), Err("--out given twice".to_owned()));
        assert_eq!(
            with(&["--fast", "1"]),
            Err("unknown flag --fast".to_owned())
        );
        assert!(with(&["--geometry", "1x1"]).is_err());
        assert_eq!(with(&["--only"]), Err("--only needs a value".to_owned()));
        assert_eq!(
            parsed(&["run", "--out", "o"]),
            Err("--archive is required".to_owned())
        );
        assert_eq!(
            parsed(&["run", "--archive", "a"]),
            Err("--out is required".to_owned())
        );
        assert_eq!(parsed(&[]), Err("missing command".to_owned()));
        assert_eq!(
            parsed(&["walk"]),
            Err("unknown command \"walk\"".to_owned())
        );
    }

    #[test]
    fn workload_takes_its_title_rate_fifo_and_heuristic_flag() {
        assert_eq!(
            parsed(&["workload", "--title", "s001", "--rate", "720"]),
            Ok(Command::Workload(Workload {
                title: "s001".to_owned(),
                rate: 720,
                attention: None,
                heuristic: false,
            }))
        );
        assert_eq!(
            parsed(&[
                "workload",
                "--heuristic",
                "--rate",
                "0",
                "--title",
                "h",
                "--attention",
                "f"
            ]),
            Ok(Command::Workload(Workload {
                title: "h".to_owned(),
                rate: 0,
                attention: Some("f".into()),
                heuristic: true,
            }))
        );
        assert!(parsed(&["workload", "--title", "s", "--rate", "fast"]).is_err());
        assert!(parsed(&["workload", "--rate", "1"]).is_err());
        assert!(parsed(&["run", "--heuristic", "x"]).is_err());
    }
}
