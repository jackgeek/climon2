use crate::control::Authority;
use climon2_core::protocol::{MAX_DATA_FRAME_LENGTH, TerminalSize};
use climon2_vt::{Mode, Presentation, Restoration, Vt};
use std::time::{Duration, Instant};

/// Committed live bytes plus the backlog one attachment may have queued.
pub(super) const LIVE_LIMIT: usize = 16 * 1024 * 1024;
const RESTORE_WINDOW: usize = 1024 * 1024;
const RESTORE_TIMEOUT: Duration = Duration::from_secs(30);
const RESTORE_RATE: usize = 4 * 1024 * 1024;
const FRAME_BUDGET: usize = MAX_DATA_FRAME_LENGTH - 1;

/// What a delivery pass commits to the attachment, in order.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Out {
    /// Restoration prefix bytes, never subject to the live queue limit.
    Prefix(Vec<u8>),
    /// Live bytes: the 16 MiB live queue limit applies.
    Live(Vec<u8>),
    RestoreEnd(Authority),
}

/// The backlog and committed queue together passed the live queue limit.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Overflow;

/// One attachment's presentation and its current control period (plan
/// section 3.3, delivery per attachment).
pub(super) struct Delivery {
    presentation: Presentation,
    granted: Option<Granted>,
    backlog: Vec<u8>,
    /// Output arrived since the last derived update. A derive with nothing new emits
    /// nothing, so this only spares an unchanged screen its diff every tick.
    stale: bool,
    resized: Option<TerminalSize>,
}

struct Granted {
    authority: Authority,
    awaiting_ready: Option<Instant>,
    prefix_bytes: usize,
    restoration: Option<Restoration>,
}

impl Delivery {
    pub fn new(host: TerminalSize) -> Self {
        Self {
            presentation: Presentation::new(host.cols, host.rows),
            granted: None,
            backlog: Vec::new(),
            // A grant's restoration starts from the current screen, not from this.
            stale: Default::default(),
            resized: None,
        }
    }

    /// Retires any previous period and returns phase A of the new one.
    /// Never call while the VT's synchronized update is pending.
    pub fn grant(
        &mut self,
        vt: &mut Vt,
        authority: Authority,
        host: TerminalSize,
        now: Instant,
    ) -> Vec<u8> {
        self.retire();
        self.presentation.resize_host(host.cols, host.rows);
        let (lead, restoration) = vt.grant(&mut self.presentation);
        self.granted = Some(Granted {
            authority,
            awaiting_ready: Some(now),
            prefix_bytes: lead.len(),
            restoration: Some(restoration),
        });
        lead
    }

    /// The controller confirmed readiness: its restoration deadline ends.
    pub fn ready(&mut self) {
        if let Some(granted) = &mut self.granted {
            granted.awaiting_ready = None;
        }
    }

    pub fn retire(&mut self) {
        self.granted = None;
        self.backlog.clear();
    }

    /// Retires the period and returns the bytes that hand the host back.
    pub fn release(&mut self) -> Vec<u8> {
        self.retire();
        self.presentation.release()
    }

    /// The synchronized-update latch expired: end any update open on the host.
    pub fn end_sync(&mut self) -> Vec<u8> {
        self.presentation.end_sync()
    }

    /// The ready controller's host size changed; presented by the next
    /// [`Delivery::produce`] outside a synchronized update.
    pub fn resize(&mut self, host: TerminalSize) {
        self.resized = Some(host);
    }

    /// Routes one read's live output. Direct output is committed unless a
    /// restoration is in progress, when it waits in the backlog; Derived
    /// output is rendered later by [`Delivery::produce`].
    pub fn live(&mut self, vt: &Vt, live: Vec<u8>, queued: usize) -> Result<Vec<u8>, Overflow> {
        if self.granted.is_none() {
            return Ok(Vec::new());
        }
        self.stale = true;
        if self.presentation.mode() == Mode::Derived {
            return Ok(Vec::new());
        }
        self.direct(vt, live, queued)
    }

    fn direct(&mut self, vt: &Vt, live: Vec<u8>, queued: usize) -> Result<Vec<u8>, Overflow> {
        if self.restoring() {
            self.backlog.extend(live);
            return within_limit(queued + self.backlog.len()).map(|()| Vec::new());
        }
        vt.commit_live(&mut self.presentation, &live);
        Ok(live)
    }

    /// A pending host resize; the next restoration frame while `queued` is
    /// under the window, then `RestoreEnd` and the backlog; for a Derived
    /// controller past its restoration, the derived update when output
    /// changed the engine.
    pub fn produce(&mut self, vt: &mut Vt, queued: usize, sync_pending: bool) -> Vec<Out> {
        let mut out = Vec::new();
        if !sync_pending {
            self.present(vt, &mut out);
        }
        if queued < RESTORE_WINDOW {
            self.restore(vt, &mut out);
        }
        if self.derives(sync_pending) {
            out.push(Out::Live(vt.derive(&mut self.presentation)));
        }
        out.retain(|out| !matches!(out, Out::Prefix(bytes) | Out::Live(bytes) if bytes.is_empty()));
        out
    }

    fn present(&mut self, vt: &mut Vt, out: &mut Vec<Out>) {
        if let Some(host) = self.resized.take() {
            let bytes = vt.present(&mut self.presentation, host.cols, host.rows);
            out.push(Out::Live(bytes));
        }
    }

    fn restore(&mut self, vt: &mut Vt, out: &mut Vec<Out>) {
        let Some(granted) = &mut self.granted else {
            return;
        };
        let Some(restoration) = &mut granted.restoration else {
            return;
        };
        let chunk = vt.produce(restoration, &mut self.presentation, FRAME_BUDGET);
        granted.prefix_bytes += chunk.bytes.len();
        out.push(Out::Prefix(chunk.bytes));
        if chunk.done {
            granted.restoration = None;
            out.push(Out::RestoreEnd(granted.authority));
            self.flush_backlog(vt, out);
        }
    }

    fn flush_backlog(&mut self, vt: &Vt, out: &mut Vec<Out>) {
        let backlog = std::mem::take(&mut self.backlog);
        if !backlog.is_empty() {
            vt.commit_live(&mut self.presentation, &backlog);
        }
        out.push(Out::Live(backlog));
    }

    /// A Derived controller past its restoration gets one update for the output since its
    /// last, never during a synchronized update. Output during a restoration or an update
    /// keeps the screen stale until then.
    fn derives(&mut self, sync_pending: bool) -> bool {
        !sync_pending
            && self.presentation.mode() == Mode::Derived
            && self
                .granted
                .as_ref()
                .is_some_and(|granted| granted.restoration.is_none())
            && std::mem::take(&mut self.stale)
    }

    fn restoring(&self) -> bool {
        self.granted
            .as_ref()
            .is_some_and(|granted| granted.restoration.is_some())
    }

    /// The authority whose restoration deadline (section 1.10) passed
    /// without `Ready`.
    pub fn expired(&self, now: Instant) -> Option<Authority> {
        let granted = self.granted.as_ref()?;
        let started = granted.awaiting_ready?;
        (now >= deadline(started, granted.prefix_bytes)).then_some(granted.authority)
    }
}

/// 30 s plus one second per 4 MiB of committed prefix.
fn deadline(started: Instant, prefix_bytes: usize) -> Instant {
    let allowance = Duration::from_secs((prefix_bytes / RESTORE_RATE) as u64);
    started + RESTORE_TIMEOUT + allowance
}

/// The one live queue limit: queued bytes up to `LIVE_LIMIT` are within it.
pub(super) fn within_limit(bytes: usize) -> Result<(), Overflow> {
    if bytes > LIVE_LIMIT {
        return Err(Overflow);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{Period, Restoration};
    use climon2_vt::{Geometry, VtConfig};

    fn authority(number: u64) -> Authority {
        Authority {
            period: Period(number),
            restoration: Restoration(number),
        }
    }

    const NOW: fn() -> Instant = Instant::now;

    fn size(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize { cols, rows }
    }

    fn vt(cols: u16, rows: u16) -> Vt {
        let geometry = Geometry::for_host(cols, rows).unwrap();
        let config = VtConfig {
            max_history_rows: 100,
            default_title: "t".into(),
        };
        let mut vt = Vt::new(geometry, config);
        vt.feed(b"history\r\n", NOW());
        vt
    }

    fn text(out: &[Out]) -> String {
        out.iter()
            .map(|out| match out {
                Out::Prefix(bytes) => format!("P[{}]", String::from_utf8_lossy(bytes)),
                Out::Live(bytes) => format!("L[{}]", String::from_utf8_lossy(bytes)),
                Out::RestoreEnd(authority) => format!("E{}", authority.restoration.0),
            })
            .collect()
    }

    /// These restorations take a few frames; the bound turns a lost `RestoreEnd` into a
    /// failure instead of a hang.
    const MAX_FRAMES: usize = 1000;

    /// Produces until `RestoreEnd`, returning everything produced.
    fn restore(delivery: &mut Delivery, vt: &mut Vt) -> Vec<Out> {
        let mut all = Vec::new();
        for _ in 0..MAX_FRAMES {
            all.extend(delivery.produce(vt, 0, false));
            if all.iter().any(|out| matches!(out, Out::RestoreEnd(_))) {
                return all;
            }
        }
        panic!("no RestoreEnd in {MAX_FRAMES} frames: {}", text(&all));
    }

    fn granted(vt: &mut Vt, host: TerminalSize) -> Delivery {
        let mut delivery = Delivery::new(host);
        delivery.grant(vt, authority(1), host, NOW());
        delivery
    }

    #[test]
    fn live_output_waits_in_the_backlog_until_restore_end() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        assert_eq!(delivery.live(&vt, b"one".to_vec(), 0), Ok(Vec::new()));
        assert_eq!(delivery.live(&vt, b"two".to_vec(), 0), Ok(Vec::new()));
        let restored = text(&restore(&mut delivery, &mut vt));
        assert!(restored.starts_with("P["), "{restored}");
        assert!(restored.ends_with("]E1L[onetwo]"), "{restored}");
        assert_eq!(
            delivery.live(&vt, b"three".to_vec(), 0),
            Ok(b"three".to_vec())
        );
        assert!(delivery.produce(&mut vt, 0, false).is_empty());
    }

    #[test]
    fn without_a_grant_nothing_is_delivered() {
        let mut vt = vt(20, 4);
        let mut delivery = Delivery::new(size(20, 4));
        assert_eq!(delivery.live(&vt, b"x".to_vec(), 0), Ok(Vec::new()));
        assert!(delivery.produce(&mut vt, 0, false).is_empty());
        assert_eq!(delivery.expired(NOW() + Duration::from_secs(3600)), None);
    }

    #[test]
    fn the_backlog_counts_toward_the_live_queue_limit() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        assert_eq!(
            delivery.live(&vt, vec![b'x'; 10], LIVE_LIMIT - 10),
            Ok(Vec::new())
        );
        assert_eq!(
            delivery.live(&vt, vec![b'x'], LIVE_LIMIT - 20),
            Ok(Vec::new())
        );
        assert_eq!(
            delivery.live(&vt, vec![b'x'], LIVE_LIMIT - 11),
            Err(Overflow)
        );
    }

    #[test]
    fn restoration_waits_while_the_window_is_full() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        assert!(delivery.produce(&mut vt, RESTORE_WINDOW, false).is_empty());
        assert!(
            !delivery
                .produce(&mut vt, RESTORE_WINDOW - 1, false)
                .is_empty()
        );
    }

    #[test]
    fn a_replacement_grant_drops_the_backlog_and_leaves_the_alternate_screen_once() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        restore(&mut delivery, &mut vt);
        delivery.live(&vt, b"\x1b[?1049h".to_vec(), 0).unwrap();
        let leaves = |lead: Vec<u8>| {
            String::from_utf8_lossy(&lead)
                .matches("\x1b[?1049l")
                .count()
        };
        assert_eq!(
            leaves(delivery.grant(&mut vt, authority(2), size(20, 4), NOW())),
            1
        );
        delivery.live(&vt, b"dropped".to_vec(), 0).unwrap();
        assert_eq!(
            leaves(delivery.grant(&mut vt, authority(3), size(20, 4), NOW())),
            0
        );
        let restored = text(&restore(&mut delivery, &mut vt));
        assert!(restored.ends_with("E3"), "{restored}");
        assert!(!restored.contains("dropped"));
    }

    #[test]
    fn release_leaves_the_alternate_screen_only_when_this_stream_entered_it() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        restore(&mut delivery, &mut vt);
        assert!(!String::from_utf8_lossy(&delivery.release()).contains("1049"));
        let mut delivery = granted(&mut vt, size(20, 4));
        restore(&mut delivery, &mut vt);
        delivery.live(&vt, b"\x1b[?1049h".to_vec(), 0).unwrap();
        assert!(String::from_utf8_lossy(&delivery.release()).contains("\x1b[?1049l"));
        assert_eq!(delivery.live(&vt, b"after".to_vec(), 0), Ok(Vec::new()));
    }

    #[test]
    fn a_derived_controller_is_rendered_after_restore_end_and_never_while_synchronized() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(30, 6));
        assert_eq!(delivery.live(&vt, b"x".to_vec(), 0), Ok(Vec::new()));
        restore(&mut delivery, &mut vt);
        vt.feed(b"derived", NOW());
        delivery.live(&vt, b"derived".to_vec(), 0).unwrap();
        assert!(delivery.produce(&mut vt, 0, true).is_empty());
        let derived = text(&delivery.produce(&mut vt, 0, false));
        assert!(
            derived.starts_with("L[") && derived.contains("derived"),
            "{derived}"
        );
        assert!(delivery.produce(&mut vt, 0, false).is_empty());
        delivery.end_sync();
        assert!(delivery.produce(&mut vt, 0, false).is_empty());
    }

    #[test]
    fn a_derived_restoration_does_not_commit_a_cursor_from_unpainted_output() {
        let mut vt = vt(20, 6);
        vt.feed(b"\x1b[3;7H", NOW());
        let mut delivery = granted(&mut vt, size(30, 8));
        let live = vt.feed(b"\x1b[?2026h\x1b[5;11H", NOW()).live;
        assert!(delivery.live(&vt, live, 0).unwrap().is_empty());
        let out = delivery.produce(&mut vt, 0, true);
        assert!(out.contains(&Out::RestoreEnd(authority(1))));
        assert!(delivery.release().ends_with(b"\x1b[3;7H"));
    }

    #[test]
    fn end_sync_closes_an_update_open_on_a_direct_host() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        restore(&mut delivery, &mut vt);
        delivery.live(&vt, b"\x1b[?2026h".to_vec(), 0).unwrap();
        assert_eq!(delivery.end_sync(), b"\x1b[?2026l");
        assert!(delivery.end_sync().is_empty());
    }

    #[test]
    fn a_resized_host_is_presented_outside_synchronized_updates() {
        let mut vt = vt(20, 4);
        let mut delivery = granted(&mut vt, size(20, 4));
        restore(&mut delivery, &mut vt);
        delivery.resize(size(20, 4));
        assert!(delivery.produce(&mut vt, 0, false).is_empty());
        delivery.resize(size(30, 6));
        assert!(delivery.produce(&mut vt, 0, true).is_empty());
        let presented = text(&delivery.produce(&mut vt, 0, false));
        assert!(presented.starts_with("L[\x1b[?2026l"), "{presented}");
        assert_eq!(delivery.live(&vt, b"x".to_vec(), 0), Ok(Vec::new()));
    }

    #[test]
    fn the_restoration_deadline_runs_from_the_grant_until_ready() {
        let mut vt = vt(20, 4);
        let start = NOW();
        let mut delivery = Delivery::new(size(20, 4));
        let lead = delivery.grant(&mut vt, authority(4), size(20, 4), start);
        let due = deadline(start, lead.len());
        assert_eq!(delivery.expired(due - Duration::from_nanos(1)), None);
        assert_eq!(delivery.expired(due), Some(authority(4)));
        delivery.ready();
        assert_eq!(delivery.expired(due), None);
    }

    #[test]
    fn the_deadline_allows_a_second_per_four_mebibytes_of_prefix() {
        let start = NOW();
        assert_eq!(deadline(start, RESTORE_RATE - 1), start + RESTORE_TIMEOUT);
        assert_eq!(
            deadline(start, 2 * RESTORE_RATE),
            start + RESTORE_TIMEOUT + Duration::from_secs(2)
        );
    }
}
