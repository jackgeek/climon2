//! Coverage runs that keep counters from processes that exec or are killed.
//!
//! The CLI execs into its client, and readiness failures SIGKILL daemons, so
//! profiles written only at exit undercount or truncate. LLVM continuous mode
//! (`%c`) maps the counters into the profile file; the `%m` merge pool gives one
//! file per binary, so PID reuse cannot collide. Darwin requires the profile
//! sections to be page aligned for continuous mode (16 KiB covers arm64 too).
use crate::quality::{coverage_command, output, run};
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

const SECTIONS: [&str; 3] = ["__llvm_prf_cnts", "__llvm_prf_data", "__llvm_prf_bits"];
const PROFILE: &str = "climon2-%c%m.profraw";

pub(crate) struct Coverage {
    root: PathBuf,
    build: PathBuf,
    environment: Vec<(String, String)>,
}

fn parse_environment(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let value = value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .unwrap_or(value);
            (key.to_owned(), value.to_owned())
        })
        .collect()
}

fn continuous_profile(environment: &mut [(String, String)]) -> io::Result<String> {
    let (_, value) = environment
        .iter_mut()
        .find(|(key, _)| key == "LLVM_PROFILE_FILE")
        .ok_or_else(|| io::Error::other("llvm-cov did not supply its profile filename"))?;
    let directory = Path::new(value.as_str())
        .parent()
        .ok_or_else(|| io::Error::other("llvm-cov profile filename has no directory"))?;
    *value = directory.join(PROFILE).to_string_lossy().into_owned();
    Ok(value.clone())
}

fn aligned_sections() -> String {
    SECTIONS
        .iter()
        .map(|section| format!("-Clink-arg=-Wl,-sectalign,__DATA,{section},0x4000"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Coverage {
    /// Starts a clean coverage session whose builds and profiles live under `build`.
    pub(crate) fn prepare(root: &Path, build: &Path) -> io::Result<Self> {
        run(coverage_command(root, build).args(["clean", "--workspace"]))?;
        let shown = output(coverage_command(root, build).arg("show-env"))?;
        let mut environment =
            parse_environment(&String::from_utf8(shown.stdout).map_err(io::Error::other)?);
        continuous_profile(&mut environment)?;
        environment.push(("RUSTFLAGS".to_owned(), aligned_sections()));
        Ok(Self {
            root: root.to_owned(),
            build: build.to_owned(),
            environment,
        })
    }

    /// The profile pattern for externally started instrumented binaries.
    pub(crate) fn profile(&self) -> &str {
        self.environment
            .iter()
            .find(|(key, _)| key == "LLVM_PROFILE_FILE")
            .map(|(_, value)| value.as_str())
            .expect("prepared coverage has a profile")
    }

    /// A `cargo` command built and run with instrumentation in this session.
    pub(crate) fn cargo(&self) -> Command {
        let mut command = Command::new(env!("CARGO"));
        command
            .current_dir(&self.root)
            .env_remove("CARGO_BUILD_TARGET_DIR")
            .env_remove("CARGO_BUILD_BUILD_DIR")
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .env("CARGO_TARGET_DIR", &self.build)
            .env("CARGO_LLVM_COV_TARGET_DIR", &self.build)
            .env("CARGO_LLVM_COV_BUILD_DIR", &self.build);
        command
    }

    /// Exports every profile written so far as LCOV.
    pub(crate) fn report(&self, lcov: &Path) -> io::Result<()> {
        run(coverage_command(&self.root, &self.build)
            .args(["report", "--lcov", "--output-path"])
            .arg(lcov))
    }
}
