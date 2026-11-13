//! Daemon wire revision 1 on the session loop, beside the private protocol:
//! passive monitor connections, their requests and their core metadata.
use super::Session;
use crate::connection::{Connection, frame_limit};
use climon2_core::wire::{
    Advertisement, Available, ControlRecord, Counter, Error, Families, Family, Frame, FrameLimits,
    Guid, InitialMetadata, Kill, Mode, Open, OperationalCode, Outcome, Rejection, Request,
    Response, Success, TitleChanged,
};
use std::{collections::BTreeSet, io};

const HAS_ROLE: &str = "this connection already has a role";
const NO_ROLE: &str = "this connection has no role to leave";
const UNSUPPORTED_REVISION: &str = "this daemon supports revision 1 only";
const UNSUPPORTED_CAPABILITY: &str = "a required capability is not supported";
/// Interactive Open (#87), standalone Kill (#88) and Acquire (#89) are not
/// served yet.
const NOT_YET_SERVED: &str = "this daemon does not serve this request yet";
/// No revision-1 connection is an attachment yet.
const NOT_PERMITTED: &str = "this connection's role does not permit this request";

/// What hello advertises: only the families this daemon implements.
pub(super) fn advertisement() -> Advertisement {
    let limits = FrameLimits {
        max_request_frame_bytes: frame_limit(),
        max_server_frame_bytes: frame_limit(),
    };
    Advertisement::new([Family::MonitorV1, Family::TitleV1], limits)
}

/// A passive monitor connection: registered, never an attachment, receiving
/// core metadata for the families it selected.
pub(super) struct Monitor {
    selected: BTreeSet<Family>,
}

impl<C: FnMut() -> io::Result<()>> Session<'_, C> {
    /// One revision-1 request. A stream that an earlier request in the same
    /// read ended serves nothing more.
    pub(super) fn request(&mut self, id: u64, request: Request) {
        if self.connections.get(&id).is_some_and(Connection::receiving) {
            self.serve(id, request);
        }
    }

    fn serve(&mut self, id: u64, request: Request) {
        match request {
            Request::Open(open) => self.open(id, open),
            Request::Leave(leave) => self.leave(id, leave.request_id),
            Request::Kill(kill) => self.wire_kill(id, kill),
            Request::Input(_)
            | Request::Resize(_)
            | Request::Acquire(_)
            | Request::Ready(_)
            | Request::SetViewing(_)
            | Request::Acknowledge(_)
            | Request::Focus(_) => self.unserved(id, &request),
        }
    }

    /// A request this daemon does not serve yet is rejected without effects;
    /// the stream goes on and a registration stays. Acquire would convert a
    /// passive monitor connection into an attachment, so on one it is
    /// refused until #89 serves it. Every other request, and Acquire without
    /// a registration, is impermissible: no revision-1 connection is an
    /// attachment yet.
    fn unserved(&mut self, id: u64, request: &Request) {
        let error = match (request, self.monitors.contains_key(&id)) {
            (Request::Acquire(_), true) => not_yet(),
            _ => error(OperationalCode::InvalidState, NOT_PERMITTED),
        };
        self.reject_with(id, request.request_id(), error);
    }

    /// A later Open never repurposes an existing role.
    fn open(&mut self, id: u64, open: Open) {
        if self.monitors.contains_key(&id) {
            return self.refuse(id, open.request_id);
        }
        self.first_role(id, open);
    }

    /// An Open that would give the connection its first role.
    fn first_role(&mut self, id: u64, open: Open) {
        let capabilities = &open.capabilities;
        match (
            select(
                open.revision,
                capabilities.required(),
                capabilities.optional(),
            ),
            open.mode,
        ) {
            (Ok(selected), Mode::Monitor) => self.register(id, open.request_id, selected),
            (Ok(_), Mode::Interactive(_)) => self.reject_role(id, open.request_id, not_yet()),
            (Err(error), _) => self.reject_role(id, open.request_id, error),
        }
    }

    /// Success, then the atomic Initial metadata: the registration captures
    /// the current core metadata and receives every later change.
    fn register(&mut self, id: u64, request_id: Counter, selected: BTreeSet<Family>) {
        let opened = Success::Opened {
            connection_id: fresh_connection_id(),
            selected_capabilities: tokens(&selected),
        };
        self.respond(id, request_id, Outcome::Ok(opened));
        let title = selected
            .contains(&Family::TitleV1)
            .then(|| self.vt.application_title().map(str::to_owned));
        self.send_wire(id, &initial_metadata(title).to_frame());
        self.monitors.insert(id, Monitor { selected });
    }

    /// Removes the registration in one step and reports that no attachment
    /// was detached, then closes the stream.
    fn leave(&mut self, id: u64, request_id: Counter) {
        if self.monitors.remove(&id).is_none() {
            return self.reject(id, request_id, OperationalCode::InvalidState, NO_ROLE);
        }
        let left = Success::Left {
            attachment_detached: false,
        };
        self.respond(id, request_id, Outcome::Ok(left));
        self.end_stream(id);
    }

    /// Standalone Kill is validated like any first role; a later Kill never
    /// repurposes an existing role.
    fn wire_kill(&mut self, id: u64, kill: Kill) {
        if self.monitors.contains_key(&id) {
            return self.refuse(id, kill.request_id);
        }
        let none = Families::default();
        let selected = select(kill.revision, &kill.required_capabilities, &none);
        self.reject_role(id, kill.request_id, selected.err().unwrap_or_else(not_yet));
    }

    /// Sends a Title changed to every registration that selected `title-v1`,
    /// in order and whatever the control state.
    pub(super) fn titles(&mut self, titles: Vec<Option<String>>) {
        for title in titles {
            let frame = TitleChanged { title }.to_frame();
            for id in self.subscribers(Family::TitleV1) {
                self.send_wire(id, &frame);
            }
        }
    }

    fn subscribers(&self, family: Family) -> Vec<u64> {
        self.monitors
            .iter()
            .filter(|(_, monitor)| monitor.selected.contains(&family))
            .map(|(id, _)| *id)
            .collect()
    }

    /// An identifiable but impermissible request: `invalid_state`, and the
    /// stream continues.
    fn refuse(&mut self, id: u64, request_id: Counter) {
        self.reject(id, request_id, OperationalCode::InvalidState, HAS_ROLE);
    }

    fn reject(&mut self, id: u64, request_id: Counter, code: OperationalCode, message: &str) {
        self.reject_with(id, request_id, error(code, message));
    }

    /// A rejected first role has no effects and ends its stream.
    fn reject_role(&mut self, id: u64, request_id: Counter, error: Error) {
        self.reject_with(id, request_id, error);
        self.end_stream(id);
    }

    fn reject_with(&mut self, id: u64, request_id: Counter, error: Error) {
        let rejection = Rejection {
            error,
            known_final: None,
        };
        self.respond(id, request_id, Outcome::Rejected(rejection));
    }

    fn respond(&mut self, id: u64, request_id: Counter, outcome: Outcome) {
        self.send_wire(
            id,
            &Response {
                request_id,
                outcome,
            }
            .to_frame(),
        );
    }

    fn end_stream(&mut self, id: u64) {
        if let Some(connection) = self.connections.get_mut(&id) {
            connection.close_after_flush();
        }
    }

    fn send_wire(&mut self, id: u64, frame: &Frame) {
        let Some(connection) = self.connections.get_mut(&id) else {
            return;
        };
        let queued = connection.queue_wire(frame);
        self.queued_or_lost(id, queued);
    }
}

/// The families a first role request selects: every required family, which
/// this daemon must support, and the supported optional ones. Unknown or
/// unsupported optional families are not selected.
fn select(
    revision: u32,
    required: &Families,
    optional: &Families,
) -> Result<BTreeSet<Family>, Error> {
    if revision != 1 {
        return Err(error(
            OperationalCode::UnsupportedRevision,
            UNSUPPORTED_REVISION,
        ));
    }
    let required: Option<Vec<Family>> = required
        .tokens()
        .iter()
        .map(|token| supported(token))
        .collect();
    let required = required.ok_or_else(|| {
        error(
            OperationalCode::UnsupportedCapability,
            UNSUPPORTED_CAPABILITY,
        )
    })?;
    let optional = optional
        .tokens()
        .iter()
        .filter_map(|token| supported(token));
    Ok(required.into_iter().chain(optional).collect())
}

fn supported(token: &str) -> Option<Family> {
    Family::from_token(token).filter(|family| advertisement().supports(*family))
}

fn tokens(selected: &BTreeSet<Family>) -> Families {
    let tokens = selected.iter().map(|family| family.token().to_owned());
    Families::new(tokens.collect()).expect("a set holds each token once")
}

/// The title by value when `title-v1` is selected (`Some(None)` is unset),
/// with status and progress explicitly unavailable. No command result is
/// reported: the exit-or-signal result it needs arrives with #88.
fn initial_metadata(title: Option<Option<String>>) -> InitialMetadata {
    let title = title.map_or(Available::Unavailable, Available::Available);
    InitialMetadata::new(title, Available::Unavailable, Available::Unavailable, None)
        .expect("unavailable status agrees with any command result")
}

/// A fresh logical connection identity, never reused.
fn fresh_connection_id() -> Guid {
    uuid::Uuid::new_v4()
        .to_string()
        .parse()
        .expect("a v4 UUID is a canonical lowercase GUID")
}

fn error(code: OperationalCode, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}

fn not_yet() -> Error {
    error(OperationalCode::InternalError, NOT_YET_SERVED)
}
