//! `c2 home` and `c2 home release` (ADR 0011, #403): the climon2 home's owner,
//! shown and released. Neither checks the owner first: they are how a refusal
//! is diagnosed, and how a home is moved or recovered when its owner is gone.
use crate::Error;
use climon2_core::{layout::HomeLayout, metadata::SessionOrigin, owner::OWNER_FILE};
use climon2_discovery::{MetadataLimits, Options, Outcome, Pass, StaleMetadata, Uncertainty};
use climon2_platform::{
    origin,
    owner::{self, Released},
};
use std::{io, path::Path};

/// What `c2 home` was asked to do.
#[derive(Clone, Copy)]
pub enum Action {
    Show,
    /// `--force` releases a home while it holds sessions: another host's, or,
    /// with a warning, this host's own that may still run.
    Release {
        force: bool,
    },
}

/// `release` only probes: it never removes session metadata, whoever wrote it.
const PROBE: Options = Options {
    limits: MetadataLimits::UNBOUNDED,
    stale: StaleMetadata::Retain,
};

pub fn run(home: &Path, action: Action) -> Result<u32, Error> {
    match action {
        Action::Show => show(home),
        Action::Release { force } => released(home, force),
    }
}

/// The home's path, its owner, and this host with whether it is the owner.
/// Nothing is claimed.
fn show(home: &Path) -> Result<u32, Error> {
    let recorded = owner::read(home);
    println!("climon2 home: {}", home.display());
    println!("owner: {}", owner_line(&recorded));
    println!("this host: {}", host_line(&recorded, origin::current()));
    Ok(0)
}

fn owner_line(recorded: &io::Result<Option<SessionOrigin>>) -> String {
    match recorded {
        Ok(Some(owner)) => owner.to_string(),
        Ok(None) => "none (the next command claims this home)".to_owned(),
        Err(error) => format!("unreadable ({error})"),
    }
}

fn host_line(
    recorded: &io::Result<Option<SessionOrigin>>,
    here: io::Result<SessionOrigin>,
) -> String {
    match here {
        Ok(here) => format!("{here} ({})", verdict(recorded, &here)),
        Err(error) => format!("unknown ({error})"),
    }
}

fn verdict(recorded: &io::Result<Option<SessionOrigin>>, here: &SessionOrigin) -> &'static str {
    match recorded {
        Ok(Some(owner)) if here.is_same_place(owner) => "the owner",
        Ok(Some(_)) => "not the owner",
        Ok(None) => "no owner yet",
        Err(_) => "owner unknown",
    }
}

/// Removes the owner record, whoever it names. The owner keeps it while any of
/// its sessions may still run; any other host, or one that cannot tell whose
/// the home is, keeps it while the home holds session records at all. `force`
/// releases it anyway, warning the owner how many of its sessions may run.
fn released(home: &Path, force: bool) -> Result<u32, Error> {
    let recorded = owner::read(home);
    if matches!(recorded, Ok(None)) {
        println!("{}", described(home, Released::Nothing));
        return Ok(0);
    }
    guard(home, &recorded, force)?;
    println!("{}", described(home, owner::release(home)?));
    Ok(0)
}

/// Refuses as `released` says, before anything is removed.
fn guard(
    home: &Path,
    recorded: &io::Result<Option<SessionOrigin>>,
    force: bool,
) -> Result<(), Error> {
    let pass = probe(home)?;
    if owns(recorded) {
        refuse_running(&pass, force)
    } else if force {
        Ok(())
    } else {
        refuse_held(&pass)
    }
}

/// A discovery pass that removes nothing.
fn probe(home: &Path) -> io::Result<Pass> {
    let layout = HomeLayout::at(home).map_err(io::Error::other)?;
    climon2_discovery::discover(&layout, PROBE)
}

/// Whether this host is the one the home records.
fn owns(recorded: &io::Result<Option<SessionOrigin>>) -> bool {
    match (recorded, origin::current()) {
        (Ok(Some(owner)), Ok(here)) => here.is_same_place(owner),
        _ => false,
    }
}

/// The owner refuses while any of its sessions is live or not proven ended,
/// as one whose daemon still runs is; another host's sessions are not its own.
/// With `force` it only warns.
fn refuse_running(pass: &Pass, force: bool) -> Result<(), Error> {
    let running = pass
        .sessions
        .iter()
        .filter(|outcome| {
            !matches!(
                outcome,
                Outcome::Stale(..) | Outcome::Uncertain(_, Uncertainty::CreatedElsewhere)
            )
        })
        .count();
    let message = format!(
        "this host still has {running} climon2 session(s) in this home that may be running"
    );
    match (running, force) {
        (0, _) => Ok(()),
        (_, true) => {
            eprintln!("climon2: warning: {message}; --force releases the home anyway");
            Ok(())
        }
        (_, false) => Err(Error::new(
            125,
            format!("{message}; end them before releasing it"),
        )),
    }
}

/// Any other host refuses while the home holds session records at all: the
/// host that owns it may still be running them.
fn refuse_held(pass: &Pass) -> Result<(), Error> {
    let held = pass.sessions.len() + pass.invalid.len();
    if held > 0 {
        return Err(Error::new(
            125,
            format!(
                "this climon2 home holds {held} session record(s), which the host that owns it \
                 may still be running; if that host is gone, run `c2 home release --force`"
            ),
        ));
    }
    Ok(())
}

fn described(home: &Path, released: Released) -> String {
    let path = home.join(OWNER_FILE);
    match released {
        Released::Nothing => "this climon2 home has no owner record".to_owned(),
        Released::Owner(owner) => format!(
            "removed the owner record of {owner} from {}; the next command claims this home",
            path.display()
        ),
        Released::Unreadable(error) => format!(
            "removed an unreadable owner record ({error}) from {}; the next command claims \
             this home",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `c2 home` names this host's origin, or why it cannot be read, and its
    /// verdict on the recorded owner.
    #[test]
    fn the_host_line_names_this_host_or_why_it_is_unknown() {
        let here = SessionOrigin {
            machine_id: Some("aaaa".parse().unwrap()),
            pid_namespace: None,
        };
        assert_eq!(
            host_line(&Ok(None), Ok(here)),
            "machine aaaa (no owner yet)"
        );
        let unknown = host_line(&Ok(None), Err(io::Error::other("no origin")));
        assert_eq!(unknown, "unknown (no origin)");
    }
}
