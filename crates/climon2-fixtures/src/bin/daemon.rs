use std::{
    fs::File,
    io::{self, Write},
    os::unix::ffi::OsStrExt,
};

fn field(file: &mut File, bytes: &[u8]) {
    file.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
    file.write_all(bytes).unwrap();
}

fn main() {
    let report_path = std::path::PathBuf::from(std::env::var_os("DAEMON_REPORT").unwrap());
    std::fs::write(
        report_path.with_file_name("fixture-pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    let mut report = File::create(std::env::var_os("DAEMON_REPORT").unwrap()).unwrap();
    field(
        &mut report,
        std::env::current_dir().unwrap().as_os_str().as_bytes(),
    );
    for arg in std::env::args_os() {
        field(&mut report, arg.as_os_str().as_bytes());
    }
    field(&mut report, b"ENVIRONMENT");
    for (key, value) in std::env::vars_os() {
        field(&mut report, key.as_os_str().as_bytes());
        field(&mut report, value.as_os_str().as_bytes());
    }
    drop(report);
    std::fs::write(
        std::env::var_os("DAEMON_REPORT")
            .unwrap()
            .into_string()
            .unwrap()
            + ".ready",
        b"",
    )
    .unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    print!("literal-argv-done");
}
