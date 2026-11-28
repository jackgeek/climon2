//! Loading a release's client library: the one shared, validated load of
//! `climon2_platform::library::Loader`, and what a refusal means for the
//! release's reservation and its retry.
use crate::instance::Retry;
use climon2_core::{VersionName, layout::ExecutableHome, native};
use climon2_platform::library::{Failure, Functions, Loader};

/// Loads a release's validated client library.
pub type Load = Box<dyn Fn(&VersionName) -> Result<Loaded, Refusal> + Send + Sync>;

/// A loaded release: its calls and the qualified requirements its
/// bootstrap describes.
#[derive(Clone, Copy)]
pub struct Loaded {
    pub functions: &'static Functions,
    pub requirements: native::Requirements,
}

/// Why a release cannot be used. The loader's [`Failure`] says whether a
/// later load could succeed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The validated loader refused the library.
    Library(Failure, String),
    /// No worker thread could be started to load it.
    Thread(String),
    /// Its qualified requirements are ones the configured limits never
    /// admit (#120).
    Unqualified(String),
}

impl Refusal {
    pub fn message(&self) -> &str {
        match self {
            Self::Library(_, message) | Self::Thread(message) | Self::Unqualified(message) => {
                message
            }
        }
    }

    /// Whether the dynamic loader opened the library's handle, which then
    /// stays, with its release's reservation, until process exit.
    pub fn opened(&self) -> bool {
        !matches!(
            self,
            Self::Library(
                Failure::MissingLibrary
                    | Failure::MissingDependency
                    | Failure::IncompatibleArchitecture,
                _,
            ) | Self::Thread(_)
        )
    }

    /// A library the dynamic loader could not open (missing, missing a
    /// dependency or of another architecture), a contained bootstrap
    /// failure and a load that could not start may load later, once the
    /// installation is repaired; any other refusal is permanent.
    pub fn retry(&self) -> Retry {
        match self {
            Self::Library(
                Failure::MissingLibrary
                | Failure::MissingDependency
                | Failure::IncompatibleArchitecture
                | Failure::BootstrapFailure,
                _,
            )
            | Self::Thread(_) => Retry::Passive,
            Self::Library(..) | Self::Unqualified(_) => Retry::Permanent,
        }
    }
}

/// A process's one validated loader, shared by every release it loads
/// from `executables`.
pub fn loader(executables: ExecutableHome) -> Load {
    let loader = Loader::new(executables);
    Box::new(move |version| {
        loader
            .load(version)
            .map(|library| Loaded {
                functions: library.functions(),
                requirements: *library.requirements(),
            })
            .map_err(|error| Refusal::Library(error.failure(), error.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILURES: [Failure; 9] = [
        Failure::MissingLibrary,
        Failure::MissingDependency,
        Failure::IncompatibleArchitecture,
        Failure::BootstrapFailure,
        Failure::SubstitutedImage,
        Failure::MissingSymbol,
        Failure::UnsupportedRevision,
        Failure::WrongRelease,
        Failure::MalformedDescription,
    ];

    fn library(failure: Failure) -> Refusal {
        Refusal::Library(failure, "refused".into())
    }

    /// REC-01 (pure supplement): only a library the dynamic loader could
    /// not open, a contained bootstrap failure and a load that could not
    /// start may load later; every other refusal is permanent.
    #[test]
    fn only_a_refusal_a_later_load_may_not_repeat_is_retried() {
        let retried = FAILURES.map(|failure| library(failure).retry());
        let passive = [Retry::Passive; 4];
        let permanent = [Retry::Permanent; 5];
        assert_eq!(retried[..4], passive);
        assert_eq!(retried[4..], permanent);
        assert_eq!(Refusal::Thread("no thread".into()).retry(), Retry::Passive);
        let unqualified = Refusal::Unqualified("too many workers".into());
        assert_eq!(unqualified.retry(), Retry::Permanent);
    }

    /// LIM-04 (pure supplement): the dynamic loader opened no handle for a
    /// library that is missing, misses a dependency or is of another
    /// architecture, nor for a load that never started; after any other
    /// refusal its handle, and so its reservation, stays.
    #[test]
    fn only_a_handle_the_loader_opened_keeps_its_reservation() {
        let opened = FAILURES.map(|failure| library(failure).opened());
        assert_eq!(opened[..3], [false; 3]);
        assert_eq!(opened[3..], [true; 6]);
        assert!(!Refusal::Thread("no thread".into()).opened());
        assert!(Refusal::Unqualified("too many workers".into()).opened());
    }

    /// Every refusal says why in its own words.
    #[test]
    fn a_refusal_says_why() {
        assert_eq!(library(Failure::WrongRelease).message(), "refused");
        assert_eq!(Refusal::Thread("no thread".into()).message(), "no thread");
        let unqualified = Refusal::Unqualified("too many workers".into());
        assert_eq!(unqualified.message(), "too many workers");
    }
}
