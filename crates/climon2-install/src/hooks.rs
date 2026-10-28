use std::io;

pub(crate) fn checkpoint(name: &str) -> io::Result<()> {
    let Ok(fault) = std::env::var("CLIMON2_TEST_INSTALL_FAULT") else {
        return Ok(());
    };
    let Some((action, boundary)) = fault.split_once(':') else {
        return Ok(());
    };
    if boundary != name {
        return Ok(());
    }
    trigger(action)
}

fn trigger(action: &str) -> io::Result<()> {
    match action {
        "crash" => std::process::exit(86),
        "fail" => Err(io::Error::other("injected install failure")),
        _ => Ok(()),
    }
}
