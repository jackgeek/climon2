// J3: the one probe deliberately run under cargo. The test process itself sits in cargo's job;
// the probe it starts inherits that job and reports it.
#[test]
fn j3_cargo_test() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_probe"))
        .args(["spawn-test", "J3-cargo-test"])
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&out.stdout));
}
