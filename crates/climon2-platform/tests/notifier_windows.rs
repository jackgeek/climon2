//! #318: the client library's notifier on Windows. The notification
//! descriptor is one unnamed, non-inheritable manual-reset event carried in
//! the frozen `int32_t` output, which the caller sign-extends back to a
//! `HANDLE` (ADR 0008); the waker is a socket a `WSAPoll` wakes for.
#![cfg(windows)]
use climon2_platform::{
    notifier::{Notifier, Waker, wait_raised},
    waiting,
};
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::WaitForSingleObject,
};

/// The event behind a notification descriptor, sign-extended as ADR 0008
/// tells every caller to.
fn handle(notifier: &Notifier) -> HANDLE {
    notifier.descriptor() as isize as HANDLE
}

/// Whether the event is signalled now, by a zero wait on the handle itself.
fn signalled(notifier: &Notifier) -> bool {
    // SAFETY: the notifier owns the event for the call.
    match unsafe { WaitForSingleObject(handle(notifier), 0) } {
        WAIT_OBJECT_0 => true,
        WAIT_TIMEOUT => false,
        other => panic!("waiting on the notification descriptor answered {other}"),
    }
}

#[test]
fn the_notification_descriptor_is_a_manual_reset_event_in_the_int32_output() {
    let notifier = Notifier::new().unwrap();
    assert!(notifier.descriptor() > 0);
    assert!(!signalled(&notifier));
    notifier.set(true);
    // A manual-reset event stays signalled however often it is waited on.
    assert!(signalled(&notifier));
    assert!(signalled(&notifier));
    notifier.set(false);
    assert!(!signalled(&notifier));
}

#[test]
fn the_notification_event_is_not_inherited() {
    let notifier = Notifier::new().unwrap();
    let mut flags = u32::MAX;
    // SAFETY: the notifier owns the event; the call writes one DWORD.
    assert_ne!(
        unsafe { GetHandleInformation(handle(&notifier), &mut flags) },
        0
    );
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
}

#[test]
fn a_wait_on_the_notification_descriptor_ends_when_it_is_raised_or_at_the_timeout() {
    let notifier = Notifier::new().unwrap();
    let timeout = Duration::from_millis(100);
    let started = Instant::now();
    // SAFETY (both): the notifier keeps the descriptor open for the call.
    unsafe { wait_raised(notifier.descriptor(), timeout) }.unwrap();
    assert!(started.elapsed() >= timeout, "{:?}", started.elapsed());

    notifier.set(true);
    let started = Instant::now();
    unsafe { wait_raised(notifier.descriptor(), Duration::from_secs(30)) }.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_wait_on_what_is_not_a_notification_descriptor_fails() {
    // A handle value no process is given (handles are multiples of 4).
    // SAFETY: WaitForSingleObject rejects it without using it.
    let error = unsafe { wait_raised(0x7fff_fff1, Duration::ZERO) }.unwrap_err();
    assert!(error.raw_os_error().is_some(), "{error}");
}

fn woken(waker: &Waker) -> bool {
    let mut watches = [waker.watch()];
    waiting::wait(&mut watches, Duration::ZERO).unwrap();
    watches[0].readable()
}

#[test]
fn a_waker_wakes_a_socket_wait_from_a_raise_until_cleared() {
    let waker: Waker = Notifier::waker().unwrap();
    assert!(!woken(&waker));
    waker.raise();
    waker.raise();
    assert!(woken(&waker));
    waker.clear();
    assert!(!woken(&waker));
}

#[test]
fn a_waker_never_blocks() {
    let waker = Notifier::waker().unwrap();
    waker.clear();
    for _ in 0..100_000 {
        waker.raise();
    }
    assert!(woken(&waker));
    waker.clear();
    assert!(!woken(&waker));
}
