//! `c2 home` and `c2 home release` (ADR 0011, #403): the climon2 home's owner,
//! shown and released. Neither checks the owner first: they are how a refusal
//! is diagnosed, and how a home is moved or recovered when its owner is gone.
use crate::Error;
use climon2_core::{layout::HomeLayout, metadata::SessionOrigin, owner::OWNER_FILE};
use climon2_discovery::{MetadataLimits, Options, Outcome, StaleMetadata};
use climon2_platform::{
    origin,
    owner::{self, Released},
};
use std::{io, path::Path};

/// `release` only probes: it never removes another host's session metadata.
const PROBE: Options = Options {
    limits: MetadataLimits::UNBOUNDED,
    stale: StaleMetadata::Retain,
};

pub fn run(home: &Path, release: bool) -> Result<u32, Error> {
    if release { released(home) } else { show(home) }
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

/// Removes the owner record, whoever it names, unless this host still has
/// sessions running in the home.
fn released(home: &Path) -> Result<u32, Error> {
    let layout = HomeLayout::at(home).map_err(io::Error::other)?;
    let pass = climon2_discovery::discover(&layout, PROBE)?;
    let running = pass.sessions.into_iter().filter_map(Outcome::live).count();
    if running > 0 {
        return Err(Error::new(
            125,
            format!(
                "this host still has {running} climon2 session(s) running in this home; end \
                 them before releasing it"
            ),
        ));
    }
    let path = home.join(OWNER_FILE);
    match owner::release(home)? {
        Released::Nothing => println!("this climon2 home has no owner record"),
        Released::Owner(owner) => println!(
            "removed the owner record of {owner} from {}; the next command claims this home",
            path.display()
        ),
        Released::Unreadable(error) => println!(
            "removed an unreadable owner record ({error}) from {}; the next command claims \
             this home",
            path.display()
        ),
    }
    Ok(0)
}
