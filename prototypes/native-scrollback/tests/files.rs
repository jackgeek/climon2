use std::{fs, process::Command};

#[test]
fn writes_artifacts_only_and_refuses_to_overwrite_an_existing_directory() {
    let directory = std::env::temp_dir().join(format!("climon2-native-52-{}", std::process::id()));
    let first = Command::new(env!("CARGO_BIN_EXE_native-scrollback-prototype"))
        .arg("--write-dir")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(first.status.success(), "{:?}", first.stderr);
    assert!(!first.stdout.contains(&0x1b));
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 7);
    let normal = fs::read(directory.join("02-normal.ansi")).unwrap();
    let repeated = Command::new(env!("CARGO_BIN_EXE_native-scrollback-prototype"))
        .arg("--write-dir")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(!repeated.stderr.is_empty());
    assert_eq!(fs::read(directory.join("02-normal.ansi")).unwrap(), normal);
    for entry in fs::read_dir(&directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}
