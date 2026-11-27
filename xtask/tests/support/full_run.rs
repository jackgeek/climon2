//! What the tests of `tools/quality-full-run.py` share: running the script, with API
//! reads answered from fixtures and its writes recorded.
// Each test crate that includes this module uses only part of it.
#![allow(dead_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

pub const REPO: &str = "owner/repo";

pub fn python() -> PathBuf {
    env::var_os("CLIMON2_QUALITY_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
        .into()
}

pub struct Outcome {
    pub stdout: String,
    pub output: String,
    pub summary: String,
    /// Each API write, one JSON object per line.
    pub writes: Vec<Value>,
}

/// Runs the script with `args` in `dir`, answering API reads from `responses`.
pub fn script(dir: &Path, args: &[&str], responses: Option<Value>) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("output");
    let summary = temp.path().join("summary");
    let record = temp.path().join("record");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/quality-full-run.py");
    let mut command = Command::new(python());
    command.arg(path).args(args).current_dir(dir);
    if let Some(responses) = responses {
        let fixtures = temp.path().join("responses.json");
        fs::write(&fixtures, responses.to_string()).unwrap();
        command
            .arg("--fixtures")
            .arg(&fixtures)
            .arg("--record")
            .arg(&record);
    }
    let result = command
        .env("GITHUB_OUTPUT", &output)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .output()
        .unwrap();
    // Python's text files and standard output end lines with CRLF on Windows.
    let read = |path: &Path| {
        fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n")
    };
    let stdout = String::from_utf8(result.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        result.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Outcome {
        output: read(&output),
        summary: read(&summary),
        writes: read(&record)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
        stdout,
    }
}

/// The JSON list of the one slice `index`.
pub fn one(index: u32) -> String {
    format!("[\"{index}\"]")
}
