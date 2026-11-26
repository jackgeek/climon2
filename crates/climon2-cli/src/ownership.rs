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
    /// `--force` releases a home another host owns while it holds sessions.
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
    let owner = match &recorded {
        Ok(Some(owner)) => owner.to_string(),
        Ok(None) => "none (the next command claims this home)".to_owned(),
        Err(error) => format!("unreadable ({error})"),
    };
    let host = match origin::current() {
        Ok(here) => format!("{here} ({})", verdict(&recorded, &here)),
        Err(error) => format!("unknown ({error})"),
    };
    println!("climon2 home: {}", home.display());
    println!("owner: {owner}");
    println!("this host: {host}");
    Ok(0)
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
/// the home is, keeps it while the home holds session records at all, unless
/// `force` says their host is gone.
fn released(home: &Path, force: bool) -> Result<u32, Error> {
    let recorded = owner::read(home);
    if matches!(recorded, Ok(None)) {
        println!("{}", described(home, Released::Nothing));
        return Ok(0);
    }
    let layout = HomeLayout::at(home).map_err(io::Error::other)?;
    let pass = climon2_discovery::discover(&layout, PROBE)?;
    if owns(&recorded) {
        refuse_running(&pass)?;
    } else if !force {
        refuse_held(&pass)?;
    }
    println!("{}", described(home, owner::release(home)?));
    Ok(0)
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
fn refuse_running(pass: &Pass) -> Result<(), Error> {
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
    if running > 0 {
        return Err(Error::new(
            125,
            format!(
                "this host still has {running} climon2 session(s) in this home that may be \
                 running; end them before releasing it"
            ),
        ));
    }
    Ok(())
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
