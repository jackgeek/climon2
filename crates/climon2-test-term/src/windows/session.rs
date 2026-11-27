//! A test's commands start outside any climon2 session (#437). A test run
//! from inside one inherits the variables that name it, and `c2 exec` would
//! then warn that it starts nested and wait at the prompt for Enter.

use portable_pty::CommandBuilder;
use std::process::Command;

/// The variables that tell `c2 exec` it runs inside a session.
pub const SESSION_VARIABLES: [&str; 2] = ["CLIMON2_SESSION_ID", "CLIMON2_SESSION_DEPTH"];

/// Removes [`SESSION_VARIABLES`] from what `command` inherits. A test that
/// means to start nested sets them again afterwards.
pub fn outside_any_session(command: &mut CommandBuilder) {
    for name in SESSION_VARIABLES {
        command.env_remove(name);
    }
}

/// [`outside_any_session`] for a process run without a terminal.
pub fn process_outside_any_session(command: &mut Command) -> &mut Command {
    for name in SESSION_VARIABLES {
        command.env_remove(name);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn a_command_outside_any_session_names_none() {
        let mut command = CommandBuilder::new("c2");
        command.env("CLIMON2_SESSION_ID", "outer");
        command.env("climon2_session_depth", "1");
        command.env("CLIMON2_HOME", "home");
        outside_any_session(&mut command);
        assert_eq!(command.get_env("CLIMON2_SESSION_ID"), None);
        assert_eq!(command.get_env("CLIMON2_SESSION_DEPTH"), None);
        assert_eq!(command.get_env("CLIMON2_HOME"), Some("home".as_ref()));
    }

    #[test]
    fn a_process_outside_any_session_names_none() {
        let mut command = Command::new("c2");
        command.env("CLIMON2_SESSION_ID", "outer");
        command.env("CLIMON2_HOME", "home");
        process_outside_any_session(&mut command);
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(
            envs,
            [
                (OsStr::new("CLIMON2_HOME"), Some(OsStr::new("home"))),
                (OsStr::new("CLIMON2_SESSION_DEPTH"), None),
                (OsStr::new("CLIMON2_SESSION_ID"), None),
            ]
        );
    }
}
