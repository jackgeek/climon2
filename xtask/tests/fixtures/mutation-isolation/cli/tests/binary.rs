use std::{env, fs, path::Path, process::Command};

#[test]
fn observe_the_real_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_isolation-cli"))
        .output()
        .expect("run actual CLI binary");
    // Drop the package segment lexically: overlay package directories are symlinks.
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("worker workspace root")
            .join("core/src/lib.rs"),
    )
    .expect("read this worker's actual source variant");
    let observations =
        env::var_os("CLIMON2_MUTATION_OBSERVATIONS").expect("probe observation directory");
    fs::write(
        Path::new(&observations).join(format!("{}.txt", std::process::id())),
        format!(
            "{source}\n---\n{}\n---\n{}",
            String::from_utf8_lossy(&output.stdout),
            output.status.code().expect("CLI status"),
        ),
    )
    .expect("record the source and actual binary observation");
    assert_eq!(output.status.code(), Some(125));
    assert_eq!(
        output.stdout,
        b"[false, true, true, false] [None, Some(1), Some(2)]\n"
    );
}
