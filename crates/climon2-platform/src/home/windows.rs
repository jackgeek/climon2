//! The Windows names of the two homes (#313; Windows packaging and
//! distribution): the climon2 home is `%USERPROFILE%\.climon2` unless
//! `CLIMON2_HOME` overrides it, and the default executable home is
//! `%LOCALAPPDATA%\climon2`.

/// The variable naming the user's profile, which holds `.climon2`.
pub(super) const USER_HOME: &str = "USERPROFILE";
/// The variable naming the directory that holds the default executable home.
pub(super) const EXECUTABLE_HOME_BASE: &str = "LOCALAPPDATA";
/// The default executable home within that directory.
pub(super) const EXECUTABLE_HOME: &str = "climon2";
