//! Admission (#120; operational limits, "Memory, history and admission"):
//! pure. Every release load and every instance's create first reserves what
//! it needs against aggregate accounting: memory within
//! `memory.total_bytes` (less what the monitor holds outside admission),
//! and descriptors within the operating system's limit less
//! `resources.descriptor_headroom`. A reservation is admitted whole or not
//! at all, and each holder holds one reservation, so a release's shared
//! resources count once however many instances use it.
//!
//! A release reserves its configured `resources.release_bytes` and
//! `resources.descriptors_per_load`, and its load thread's stack, before
//! the dynamic loader opens it. Once loaded, its qualified requirements
//! (read from its bootstrap) raise that reservation to the greater amount
//! before any create (relaxed pre-load qualification, confirmed
//! 2026-11-08). An instance reserves its library's share of
//! `memory.client_bytes`, owner and worker stacks and instance descriptors,
//! each stack and the descriptors the greater of configured and qualified.
use climon2_core::{
    SessionId, VersionName,
    config::Admission,
    native::{Limits, Requirements},
};
use std::collections::BTreeMap;

/// Who holds a reservation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Holder {
    /// A release's shared resources, held until process exit once its
    /// handle is open.
    Release(VersionName),
    /// The thread loading a release, until its load returns.
    Load(VersionName),
    /// One instance, until its owner is joined.
    Instance(SessionId),
}

/// What one reservation holds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Amount {
    pub(super) bytes: u64,
    pub(super) descriptors: u64,
}

/// What all reservations together may hold now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Capacity {
    pub(super) bytes: u64,
    pub(super) descriptors: u64,
}

/// What one instance of a qualified release reserves, and the stack its
/// owner thread gets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Instance {
    pub(super) amount: Amount,
    pub(super) owner_stack: u64,
}

/// Every reservation admitted and not yet freed.
#[derive(Default)]
pub(super) struct Ledger {
    held: BTreeMap<Holder, Amount>,
}

impl Ledger {
    /// Sets each holder's reservation to its amount in `holdings` if
    /// everything held beside them and those amounts fit `capacity`;
    /// otherwise changes nothing and says what is short.
    pub(super) fn hold(
        &mut self,
        holdings: &[(Holder, Amount)],
        capacity: Capacity,
    ) -> Result<(), String> {
        let others = sum(self
            .held
            .iter()
            .filter(|(key, _)| holdings.iter().all(|(holder, _)| holder != *key))
            .map(|(_, amount)| amount));
        let needed = sum(holdings.iter().map(|(_, amount)| amount));
        fits(
            "not enough memory",
            " bytes",
            needed.bytes,
            capacity.bytes.saturating_sub(others.bytes),
        )?;
        fits(
            "not enough file descriptors",
            "",
            needed.descriptors,
            capacity.descriptors.saturating_sub(others.descriptors),
        )?;
        self.held.extend(holdings.iter().cloned());
        Ok(())
    }

    /// `holder` holds nothing any more.
    pub(super) fn free(&mut self, holder: &Holder) {
        self.held.remove(holder);
    }

    /// Every reservation's bytes together.
    pub(super) fn bytes(&self) -> u64 {
        sum(self.held.values()).bytes
    }
}

/// `amounts` together, saturating.
fn sum<'a>(amounts: impl Iterator<Item = &'a Amount>) -> Amount {
    amounts.fold(Amount::default(), |total, amount| Amount {
        bytes: total.bytes.saturating_add(amount.bytes),
        descriptors: total.descriptors.saturating_add(amount.descriptors),
    })
}

/// Whether `needed` fits `available`, or what is short.
fn fits(short: &str, unit: &str, needed: u64, available: u64) -> Result<(), String> {
    if needed > available {
        return Err(format!(
            "{short}: {needed}{unit} needed, {available}{unit} available"
        ));
    }
    Ok(())
}

/// A release's reservation before it is loaded: the configured one.
pub(super) fn configured(settings: &Admission) -> Amount {
    Amount {
        bytes: settings.release_bytes,
        descriptors: settings.descriptors_per_load.into(),
    }
}

/// The load thread's reservation: its stack.
pub(super) fn loader(settings: &Admission) -> Amount {
    Amount {
        bytes: settings.stack_bytes,
        descriptors: 0,
    }
}

/// A loaded release's reservation: the greater of configured and
/// qualified.
pub(super) fn qualified(settings: &Admission, requirements: &Requirements) -> Amount {
    let configured = configured(settings);
    Amount {
        bytes: configured.bytes.max(requirements.release_bytes),
        descriptors: configured
            .descriptors
            .max(requirements.load_descriptors.into()),
    }
}

/// What each instance of a release with `requirements` reserves, created
/// with `limits`; or why such an instance is never admitted. A qualified
/// worker count above `resources.max_workers_per_client` is rejected, never
/// made room for, as is a library instance that needs more than its share
/// of `memory.client_bytes` (`limits.client_bytes`).
pub(super) fn instance(
    settings: &Admission,
    requirements: &Requirements,
    limits: &Limits,
) -> Result<Instance, String> {
    let maximum = settings.max_workers_per_client;
    if requirements.workers > maximum {
        return Err(format!(
            "the release needs {} worker threads for each client; \
             monitor.limits.resources.max_workers_per_client allows {maximum}",
            requirements.workers
        ));
    }
    let share = [
        limits.request_bytes,
        limits.event_bytes,
        limits.wire_frame_bytes.into(),
        requirements.instance_bytes,
    ]
    .into_iter()
    .fold(0, u64::saturating_add);
    if share > limits.client_bytes {
        return Err(format!(
            "the release needs {share} bytes for each client library; its share of \
             monitor.limits.memory.client_bytes is {}",
            limits.client_bytes
        ));
    }
    let owner_stack = settings.stack_bytes.max(requirements.owner_stack_bytes);
    let worker_stack = settings.stack_bytes.max(requirements.worker_stack_bytes);
    let workers = worker_stack.saturating_mul(requirements.workers.into());
    let descriptors = settings
        .descriptors_per_client
        .max(requirements.instance_descriptors);
    Ok(Instance {
        amount: Amount {
            bytes: limits
                .client_bytes
                .saturating_add(owner_stack)
                .saturating_add(workers),
            descriptors: descriptors.into(),
        },
        owner_stack,
    })
}

#[cfg(test)]
mod tests {
    //! LIM-04 (Monitor): reservations admitted at their capacity and refused
    //! one past it, shared release resources counted once, partially
    //! started instances held until freed; relaxed qualification (the
    //! greater reservation) and LIM-01 (Monitor: unsupported release
    //! requirements rejected explicitly).
    use super::*;

    fn version(name: &str) -> VersionName {
        name.parse().unwrap()
    }

    fn session(index: u8) -> SessionId {
        format!("00000000-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    const fn amount(bytes: u64, descriptors: u64) -> Amount {
        Amount { bytes, descriptors }
    }

    const CAPACITY: Capacity = Capacity {
        bytes: 100,
        descriptors: 10,
    };

    /// Memory and descriptors are each admitted exactly at what is left
    /// and refused one past it, saying what is short; a refusal holds
    /// nothing.
    #[test]
    fn a_reservation_is_admitted_at_its_capacity_and_refused_one_past_it() {
        let mut ledger = Ledger::default();
        let release = Holder::Release(version("1.0.0"));
        assert_eq!(ledger.hold(&[(release, amount(60, 4))], CAPACITY), Ok(()));
        let first = Holder::Instance(session(1));
        assert_eq!(
            ledger.hold(&[(first.clone(), amount(41, 1))], CAPACITY),
            Err("not enough memory: 41 bytes needed, 40 bytes available".into())
        );
        assert_eq!(
            ledger.hold(&[(first.clone(), amount(1, 7))], CAPACITY),
            Err("not enough file descriptors: 7 needed, 6 available".into())
        );
        assert_eq!(ledger.bytes(), 60);
        assert_eq!(ledger.hold(&[(first, amount(40, 6))], CAPACITY), Ok(()));
        assert_eq!(ledger.bytes(), 100);
        assert_eq!(
            ledger.hold(&[(Holder::Instance(session(2)), amount(0, 1))], CAPACITY),
            Err("not enough file descriptors: 1 needed, 0 available".into())
        );
        assert_eq!(
            ledger.hold(&[(Holder::Instance(session(2)), amount(1, 0))], CAPACITY),
            Err("not enough memory: 1 bytes needed, 0 bytes available".into())
        );
    }

    /// One holder holds one reservation: holding again replaces it, so a
    /// release loaded again or raised to its qualified amount counts once,
    /// and a raise that does not fit keeps what was held. Freeing returns
    /// it; a capacity that shrank below what is held admits nothing more.
    #[test]
    fn each_holder_counts_once_until_freed() {
        let mut ledger = Ledger::default();
        let release = Holder::Release(version("1.0.0"));
        let load = Holder::Load(version("1.0.0"));
        ledger
            .hold(&[(release.clone(), amount(30, 2))], CAPACITY)
            .unwrap();
        ledger
            .hold(&[(release.clone(), amount(30, 2))], CAPACITY)
            .unwrap();
        ledger
            .hold(&[(load.clone(), amount(20, 0))], CAPACITY)
            .unwrap();
        assert_eq!(ledger.bytes(), 50);
        assert_eq!(
            ledger.hold(&[(release.clone(), amount(81, 2))], CAPACITY),
            Err("not enough memory: 81 bytes needed, 80 bytes available".into())
        );
        assert_eq!(ledger.bytes(), 50);
        ledger
            .hold(&[(release.clone(), amount(80, 10))], CAPACITY)
            .unwrap();
        assert_eq!(ledger.bytes(), 100);
        ledger.free(&load);
        assert_eq!(ledger.bytes(), 80);
        let small = Capacity {
            bytes: 50,
            descriptors: 10,
        };
        assert_eq!(
            ledger.hold(&[(Holder::Instance(session(1)), amount(0, 0))], small),
            Ok(())
        );
        assert_eq!(
            ledger.hold(&[(Holder::Instance(session(2)), amount(1, 0))], small),
            Err("not enough memory: 1 bytes needed, 0 bytes available".into())
        );
        ledger.free(&release);
        assert_eq!(ledger.bytes(), 0);
    }

    /// A load reserves its release and its thread together, or neither.
    #[test]
    fn holdings_reserved_together_are_admitted_whole_or_not_at_all() {
        let mut ledger = Ledger::default();
        let release = Holder::Release(version("1.0.0"));
        let load = Holder::Load(version("1.0.0"));
        let both = |bytes| {
            [
                (release.clone(), amount(50, 4)),
                (load.clone(), amount(bytes, 6)),
            ]
        };
        assert_eq!(
            ledger.hold(&both(51), CAPACITY),
            Err("not enough memory: 101 bytes needed, 100 bytes available".into())
        );
        assert_eq!(ledger.bytes(), 0);
        assert_eq!(ledger.hold(&both(50), CAPACITY), Ok(()));
        assert_eq!(ledger.bytes(), 100);
        assert_eq!(ledger.hold(&both(50), CAPACITY), Ok(()));
        ledger.free(&load);
        assert_eq!(ledger.bytes(), 50);
    }

    const SETTINGS: Admission = Admission {
        total_bytes: 1 << 40,
        stack_bytes: 2000,
        max_workers_per_client: 3,
        descriptors_per_client: 8,
        descriptors_per_load: 4,
        descriptor_headroom: 64,
        release_bytes: 5000,
        admission_ms: 1000,
    };

    const REQUIREMENTS: Requirements = Requirements {
        release_bytes: 7000,
        instance_bytes: 100,
        owner_stack_bytes: 3000,
        worker_stack_bytes: 1000,
        workers: 3,
        instance_descriptors: 5,
        load_descriptors: 6,
        reserved: 0,
    };

    fn limits(client_bytes: u64) -> Limits {
        Limits {
            client_bytes,
            request_bytes: 10,
            event_bytes: 20,
            wire_frame_bytes: 30,
            ..climon2_core::config::Limits::default().library()
        }
    }

    /// Before a load: the configured release and the loader's stack; after
    /// it, the greater of configured and qualified for each resource.
    #[test]
    fn a_release_reserves_its_configured_amount_then_the_greater_qualified_one() {
        assert_eq!(configured(&SETTINGS), amount(5000, 4));
        assert_eq!(loader(&SETTINGS), amount(2000, 0));
        assert_eq!(qualified(&SETTINGS, &REQUIREMENTS), amount(7000, 6));
        let smaller = Requirements {
            release_bytes: 4999,
            load_descriptors: 3,
            ..REQUIREMENTS
        };
        assert_eq!(qualified(&SETTINGS, &smaller), amount(5000, 4));
    }

    /// An instance reserves its library's share, the greater owner stack,
    /// each qualified worker's greater stack and the greater descriptors.
    #[test]
    fn an_instance_reserves_its_share_stacks_and_descriptors() {
        assert_eq!(
            instance(&SETTINGS, &REQUIREMENTS, &limits(160)),
            Ok(Instance {
                amount: amount(160 + 3000 + 3 * 2000, 8),
                owner_stack: 3000,
            })
        );
        let larger = Requirements {
            owner_stack_bytes: 1500,
            worker_stack_bytes: 2500,
            workers: 2,
            instance_descriptors: 9,
            ..REQUIREMENTS
        };
        assert_eq!(
            instance(&SETTINGS, &larger, &limits(160)),
            Ok(Instance {
                amount: amount(160 + 2000 + 2 * 2500, 9),
                owner_stack: 2000,
            })
        );
    }

    /// Qualified workers up to `max_workers_per_client` are admitted and one
    /// more is rejected; a library share that holds the queues, one frame
    /// and the qualified instance allocation exactly is admitted and one
    /// byte less is rejected.
    #[test]
    fn unsupported_release_requirements_are_rejected_explicitly() {
        let more = Requirements {
            workers: 4,
            ..REQUIREMENTS
        };
        assert_eq!(
            instance(&SETTINGS, &more, &limits(160)),
            Err("the release needs 4 worker threads for each client; \
                 monitor.limits.resources.max_workers_per_client allows 3"
                .into())
        );
        assert!(instance(&SETTINGS, &REQUIREMENTS, &limits(160)).is_ok());
        assert_eq!(
            instance(&SETTINGS, &REQUIREMENTS, &limits(159)),
            Err(
                "the release needs 160 bytes for each client library; its share of \
                 monitor.limits.memory.client_bytes is 159"
                    .into()
            )
        );
    }
}
