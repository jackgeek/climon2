//! The session loop: one continuous PTY pump feeding the VT, delivery to the
//! controller, and the control state machine's effects (plan section 3.3).

mod command;
mod delivery;
mod paste;
mod pending;
mod wire;
mod writes;

use crate::{
    args::Options,
    connection::{Connection, Failure, Received},
    control::{
        Authority, CommandEnd, ConnectionId, Control, Effect, Lifecycle, Outcome, Restoration,
    },
    pty::Pty,
};
use climon2_core::{
    hello::{SessionState, advertise_revision1},
    protocol::{
        ClientMessage, Frame, HANGUP_GRACE, HelloReply, MAX_DATA_FRAME_LENGTH, PROTOCOL_VERSION,
        PTY_DRAIN_CAP, ServerMessage, TerminalSize,
    },
    title::default_title,
    wire::{CommandResult, GrantReason, RevocationReason},
};
use climon2_platform::{
    peer,
    runtime::{SocketFreshness, SocketListener, SocketStream},
    terminal::Signals,
};
use climon2_vt::{Geometry, Vt, VtConfig};
use command::{Command, Drain, Read, classify};
use delivery::{Delivery, Out};
use paste::PasteTracker;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::{Duration, Instant},
};
use writes::PtyWrites;

const OUTPUT_CHUNK: usize = 32 * 1024;
/// The terminal bytes one Output or Display frame carries at most.
const DISPLAY_CHUNK: usize = MAX_DATA_FRAME_LENGTH - 1;
const MAX_CONNECTIONS: usize = 64;
const ACCEPTS_PER_TICK: usize = 8;
const INITIAL_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(2);
const FAILURE_MESSAGE: &str = "session failed; see daemon log";
/// The kill requester standing for a termination signal; never a connection.
const SIGNAL: ConnectionId = ConnectionId(u64::MAX);

struct Session<'a, C> {
    options: &'a Options,
    listener: &'a SocketListener,
    pty: &'a mut Pty,
    signals: &'a Signals,
    cleanup: C,
    connections: BTreeMap<u64, Connection>,
    deliveries: BTreeMap<u64, Delivery>,
    /// Revision-1 connections after their first role: passive monitor
    /// connections, interactive attachments and standalone Kills.
    opened: BTreeMap<u64, wire::Opened>,
    /// The command's result, once the daemon knows it.
    command_result: Option<CommandResult>,
    control: Control,
    vt: Vt,
    next_id: u64,
    controller: Option<u64>,
    /// Restorations, of grants and replacements, awaiting their snapshot.
    pending_restorations: BTreeMap<u64, (Authority, TerminalSize)>,
    pending_lost: BTreeSet<u64>,
    pty_writes: PtyWrites,
    paste: PasteTracker,
    command: Command,
    initial_deadline: Instant,
    finish_deadline: Option<Instant>,
    socket_freshness: SocketFreshness,
}

pub(crate) fn serve(
    options: &Options,
    listener: &SocketListener,
    pty: &mut Pty,
    signals: &Signals,
    cleanup: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let vt = session_vt(options)?;
    let result = Session {
        options,
        listener,
        pty: &mut *pty,
        signals,
        cleanup,
        connections: BTreeMap::new(),
        deliveries: BTreeMap::new(),
        opened: BTreeMap::new(),
        command_result: None,
        control: Control::new(options.size, options.lifetime),
        vt,
        next_id: Default::default(),
        controller: None,
        pending_restorations: BTreeMap::new(),
        pending_lost: BTreeSet::new(),
        pty_writes: PtyWrites::default(),
        paste: PasteTracker::default(),
        command: Command::Running,
        initial_deadline: Instant::now() + INITIAL_TIMEOUT,
        finish_deadline: None,
        socket_freshness: SocketFreshness::new(&options.socket, Instant::now()),
    }
    .run();
    end_pty(pty, result)
}

fn session_vt(options: &Options) -> io::Result<Vt> {
    let geometry = Geometry::for_host(options.size.cols, options.size.rows)
        .ok_or_else(|| super::io_error("invalid session geometry"))?;
    let config = VtConfig {
        max_history_rows: options.max_history_rows,
        default_title: default_title(&options.command, &options.arguments),
    };
    Ok(Vt::new(geometry, config))
}

fn end_pty(pty: &mut Pty, result: io::Result<()>) -> io::Result<()> {
    if result.is_err() {
        pty.abandon();
        return result;
    }
    pty.finish()
}

impl<C: FnMut() -> io::Result<()>> Session<'_, C> {
    fn run(&mut self) -> io::Result<()> {
        while !self.tick(Instant::now())? {
            self.pty.wait_readable(TICK);
        }
        Ok(())
    }

    fn tick(&mut self, now: Instant) -> io::Result<bool> {
        self.socket_freshness.refresh(now)?;
        self.incoming()?;
        self.pump(now)?;
        self.outgoing(now)?;
        Ok(self.finished(now))
    }

    fn pump(&mut self, now: Instant) -> io::Result<()> {
        self.lifecycle(now)?;
        self.output(now)?;
        self.write_input()
    }

    // Incoming connections and frames.

    fn incoming(&mut self) -> io::Result<()> {
        self.accept()?;
        for id in self.connection_ids() {
            self.service(id)?;
        }
        Ok(())
    }

    fn connection_ids(&self) -> Vec<u64> {
        self.connections.keys().copied().collect()
    }

    fn accept(&mut self) -> io::Result<()> {
        let listener = self.listener;
        let pending = std::iter::from_fn(|| accepted(peer::accept(listener)).transpose());
        for stream in pending.take(ACCEPTS_PER_TICK) {
            self.add_connection(stream?)?;
        }
        Ok(())
    }

    /// Beyond the connection cap, a new connection is closed unanswered.
    fn add_connection(&mut self, stream: SocketStream) -> io::Result<()> {
        if self.connections.len() >= MAX_CONNECTIONS {
            return Ok(());
        }
        self.connections
            .insert(self.next_id, Connection::new(stream)?);
        self.next_id += 1;
        Ok(())
    }

    fn service(&mut self, id: u64) -> io::Result<()> {
        let Some(connection) = self.connections.get_mut(&id) else {
            return Ok(());
        };
        if connection.flush().is_err() {
            return self.lost(id);
        }
        let received = connection.receive_available();
        self.received(id, received)?;
        self.close_if_done(id);
        Ok(())
    }

    fn received(
        &mut self,
        id: u64,
        (messages, failure): (Vec<Received>, Option<Failure>),
    ) -> io::Result<()> {
        self.messages(id, messages)?;
        match failure {
            Some(failure) => self.protocol_failure(id, failure),
            None => Ok(()),
        }
    }

    fn messages(&mut self, id: u64, messages: Vec<Received>) -> io::Result<()> {
        for message in messages {
            if !self.connections.contains_key(&id) {
                break;
            }
            self.dispatch(id, message)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, id: u64, received: Received) -> io::Result<()> {
        match received {
            Received::Private(message) => self.message(id, message),
            Received::Request(request) => self.request(id, request),
        }
    }

    fn message(&mut self, id: u64, message: ClientMessage) -> io::Result<()> {
        match message {
            ClientMessage::Hello { .. } => self.hello(id),
            ClientMessage::Input(bytes) => {
                self.input(id, &bytes);
                Ok(())
            }
            other => self.control_message(ConnectionId(id), other),
        }
    }

    fn control_message(&mut self, conn: ConnectionId, message: ClientMessage) -> io::Result<()> {
        if let ClientMessage::Resize(size) = message {
            self.control.resize(conn, size);
            return Ok(());
        }
        let effects = control_effects(&mut self.control, conn, message);
        self.effects(effects)
    }

    fn hello(&mut self, id: u64) -> io::Result<()> {
        let state = self.control.snapshot();
        let attachments = u32::try_from(state.attachments).unwrap_or(u32::MAX);
        let state = SessionState::new(state.lifecycle, attachments, state.controlled)
            .ok_or_else(|| super::io_error("invalid session state"))?;
        let reply = HelloReply {
            session_id: self.options.id.clone(),
            climon2_version: self.options.version.clone(),
            protocol: PROTOCOL_VERSION,
        };
        self.queue_frame(
            id,
            advertise_revision1(&reply, state, &wire::advertisement())?,
        );
        Ok(())
    }

    /// Input is admitted only from the ready controller of the current period.
    fn input(&mut self, id: u64, bytes: &[u8]) {
        if !self.control.admits_input(ConnectionId(id)) {
            return;
        }
        self.pty_writes.input(id, bytes);
    }

    fn protocol_failure(&mut self, id: u64, failure: Failure) -> io::Result<()> {
        if let Some(connection) = self.connections.get_mut(&id) {
            connection.fail(failure);
        }
        self.remove_delivery(id);
        self.react(|control| control.lost(ConnectionId(id)))
    }

    fn close_if_done(&mut self, id: u64) {
        if self
            .connections
            .get(&id)
            .is_some_and(Connection::should_close)
        {
            self.remove_connection(id);
        }
    }

    // Lifecycle.

    fn lifecycle(&mut self, now: Instant) -> io::Result<()> {
        self.terminate_on_signal()?;
        self.react(|control| control.apply_pending())?;
        self.observe_command(now)?;
        self.end_hangup(now)?;
        self.initial_deadline(now)
    }

    /// A hang-up the daemon cannot confirm keeps only what it already knew.
    fn end_hangup(&mut self, now: Instant) -> io::Result<()> {
        if !self.command.hung_up(now) {
            return Ok(());
        }
        let end = match self.pty.end_hangup() {
            Ok(result) => {
                self.publish(&result);
                CommandEnd::Confirmed(result)
            }
            Err(_) => self.unconfirmed(),
        };
        self.end_command(end)
    }

    /// The command's end, unconfirmed, with any result already known.
    fn unconfirmed(&self) -> CommandEnd {
        CommandEnd::Unconfirmed(self.command_result.clone())
    }

    fn end_command(&mut self, command: CommandEnd) -> io::Result<()> {
        self.command = Command::Ended;
        self.react(|control| control.command_exited(command))
    }

    fn terminate_on_signal(&mut self) -> io::Result<()> {
        if self.signals.termination_signal().is_some() && self.running() {
            return self.react(|control| control.kill(SIGNAL));
        }
        Ok(())
    }

    fn running(&self) -> bool {
        matches!(self.command, Command::Running)
    }

    fn observe_command(&mut self, now: Instant) -> io::Result<()> {
        if !self.running() {
            return Ok(());
        }
        let observed = self.pty.observe_jobs().and_then(|()| self.pty.try_wait());
        let Ok(status) = observed else {
            return self.fail("cannot observe the command");
        };
        if let Some(result) = status {
            self.publish(&result);
            self.command = Command::Draining {
                result,
                drain: Drain::new(now),
            };
        }
        Ok(())
    }

    fn initial_deadline(&mut self, now: Instant) -> io::Result<()> {
        if self.running() && super::passed(self.initial_deadline, now) {
            return self.react(|control| control.initial_deadline());
        }
        Ok(())
    }

    fn fail(&mut self, reason: &'static str) -> io::Result<()> {
        tracing::error!(reason, "session failed");
        self.react(|control| control.authoritative_failure())
    }

    // Command output.

    fn output(&mut self, now: Instant) -> io::Result<()> {
        self.read_pty(now)?;
        self.end_sync(now);
        self.start_restorations(now);
        self.deliver(now);
        Ok(())
    }

    fn read_pty(&mut self, now: Instant) -> io::Result<()> {
        if !self.command.reads(now) {
            return Ok(());
        }
        let mut bytes = vec![u8::default(); OUTPUT_CHUNK];
        let read = classify(self.pty.read_output(&mut bytes));
        self.command.observe(&read, now);
        self.after_read(read, &bytes, now)
    }

    fn after_read(&mut self, read: Read, bytes: &[u8], now: Instant) -> io::Result<()> {
        match read {
            Read::Bytes(count) => {
                self.fed(&bytes[..count], now);
                Ok(())
            }
            Read::Failed => self.fail("cannot read the command terminal"),
            Read::End | Read::Idle => Ok(()),
        }
    }

    fn fed(&mut self, bytes: &[u8], now: Instant) {
        let fed = self.vt.feed(bytes, now);
        self.pty_writes.daemon(&fed.replies);
        if fed.focus_enabled {
            self.focus_enabled();
        }
        self.route(fed.live);
        self.titles(fed.titles);
    }

    fn route(&mut self, live: Vec<u8>) {
        let Some(id) = self.controller else {
            return;
        };
        let queued = self.queued(id);
        let Some(delivery) = self.deliveries.get_mut(&id) else {
            return;
        };
        match delivery.live(&self.vt, live, queued) {
            Ok(bytes) => self.commit_live(id, bytes),
            Err(delivery::Overflow) => self.mark_lost(id),
        }
    }

    fn end_sync(&mut self, now: Instant) {
        if !self.vt.tick(now) {
            return;
        }
        let Some(id) = self.controller else {
            return;
        };
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            let bytes = delivery.end_sync();
            self.send_output(id, &bytes);
        }
    }

    fn start_restorations(&mut self, now: Instant) {
        if self.vt.sync_pending(now) {
            return;
        }
        for (id, (authority, host)) in std::mem::take(&mut self.pending_restorations) {
            self.snapshot(id, authority, host, now);
        }
    }

    /// The snapshot for a restoration, of a grant or a replacement. The
    /// private protocol's `period` field carries the restoration ID.
    fn snapshot(&mut self, id: u64, authority: Authority, host: TerminalSize, now: Instant) {
        let Some(delivery) = self.deliveries.get_mut(&id) else {
            return;
        };
        let lead = delivery.restore(&mut self.vt, authority, host, now);
        self.controller = Some(id);
        let restoration = authority.restoration.0;
        self.reply(
            id,
            ServerMessage::Granted {
                period: restoration,
            },
        );
        self.send_output(id, &lead);
    }

    fn deliver(&mut self, now: Instant) {
        let ids: Vec<_> = self.deliveries.keys().copied().collect();
        for id in ids {
            self.deliver_to(id, now);
        }
    }

    fn deliver_to(&mut self, id: u64, now: Instant) {
        let queued = self.queued(id);
        let sync_pending = self.vt.sync_pending(now);
        let Some(delivery) = self.deliveries.get_mut(&id) else {
            return;
        };
        for out in delivery.produce(&mut self.vt, queued, sync_pending) {
            self.commit(id, out);
        }
    }

    fn commit(&mut self, id: u64, out: Out) {
        match out {
            Out::Prefix(bytes) => self.send_output(id, &bytes),
            Out::Live(bytes) => self.commit_live(id, bytes),
            Out::RestoreEnd(authority) => self.restored(id, authority),
        }
    }

    fn restored(&mut self, id: u64, authority: Authority) {
        let restoration = authority.restoration.0;
        self.reply(
            id,
            ServerMessage::RestoreEnd {
                period: restoration,
            },
        );
        self.report_restored(id, authority);
        self.control.restored(ConnectionId(id), authority);
    }

    /// Committed live bytes beyond the live queue limit lose the attachment.
    fn commit_live(&mut self, id: u64, bytes: Vec<u8>) {
        if delivery::within_limit(self.queued(id) + bytes.len()).is_err() {
            return self.mark_lost(id);
        }
        self.send_output(id, &bytes);
    }

    /// Queued output; a connection lost earlier in this tick has none and accepts nothing,
    /// so the value then is immaterial.
    fn queued(&self, id: u64) -> usize {
        self.connections
            .get(&id)
            .map(Connection::queued_bytes)
            .unwrap_or_default()
    }

    // PTY input.

    fn write_input(&mut self) -> io::Result<()> {
        if self.write_pty().is_err() {
            return self.fail("cannot write to the command terminal");
        }
        Ok(())
    }

    /// One ordered queue: replies and paste cleanup before later input, and
    /// a partly written sequence always completes first.
    fn write_pty(&mut self) -> io::Result<()> {
        let pty = &mut *self.pty;
        let written = self.pty_writes.write_with(|bytes| pty.write(bytes))?;
        self.paste.written(&written);
        Ok(())
    }

    // Outgoing frames and deadlines.

    fn outgoing(&mut self, now: Instant) -> io::Result<()> {
        self.flush_connections()?;
        self.lose_marked()?;
        self.deadlines(now)?;
        self.report_exit(now)
    }

    fn flush_connections(&mut self) -> io::Result<()> {
        for id in self.connection_ids() {
            self.flush(id)?;
        }
        Ok(())
    }

    fn flush(&mut self, id: u64) -> io::Result<()> {
        let Some(connection) = self.connections.get_mut(&id) else {
            return Ok(());
        };
        if connection.flush().is_err() {
            return self.lost(id);
        }
        self.close_if_done(id);
        Ok(())
    }

    fn lose_marked(&mut self) -> io::Result<()> {
        for id in std::mem::take(&mut self.pending_lost) {
            self.react(|control| control.lost(ConnectionId(id)))?;
        }
        Ok(())
    }

    fn deadlines(&mut self, now: Instant) -> io::Result<()> {
        self.stalled(now)?;
        self.restore_deadlines(now)
    }

    /// A reader that took no queued bytes for the no-progress period is lost.
    fn stalled(&mut self, now: Instant) -> io::Result<()> {
        let stalled: Vec<_> = self
            .connections
            .iter()
            .filter_map(|(id, connection)| connection.stalled(now).then_some(*id))
            .collect();
        for id in stalled {
            self.lost(id)?;
        }
        Ok(())
    }

    fn restore_deadlines(&mut self, now: Instant) -> io::Result<()> {
        let expired: Vec<_> = self
            .deliveries
            .iter()
            .filter_map(|(id, delivery)| Some((*id, delivery.expired(now)?)))
            .collect();
        for (id, authority) in expired {
            self.react(|control| control.restore_deadline(ConnectionId(id), authority))?;
        }
        Ok(())
    }

    /// The exit is reported once output drained, the initial attachment had
    /// its chance and no grant awaits `Ready`.
    fn report_exit(&mut self, now: Instant) -> io::Result<()> {
        let Some(result) = self.command.drained(now) else {
            return Ok(());
        };
        if self.awaiting(now) {
            return Ok(());
        }
        self.end_command(CommandEnd::Confirmed(result))
    }

    fn awaiting(&self, now: Instant) -> bool {
        self.starting(now) || self.awaiting_ready()
    }

    fn starting(&self, now: Instant) -> bool {
        holds_exit(
            self.control.snapshot().lifecycle,
            self.initial_deadline,
            now,
        )
    }

    fn awaiting_ready(&self) -> bool {
        !self.pending_restorations.is_empty()
            || self
                .controller
                .is_some_and(|id| !self.control.admits_input(ConnectionId(id)))
    }

    fn finished(&self, now: Instant) -> bool {
        finished_at(self.finish_deadline, now, || {
            self.connections.values().all(Connection::available)
        })
    }

    // Effects of the control state machine, dispatched a few at a time.

    fn react(&mut self, event: impl FnOnce(&mut Control) -> Vec<Effect>) -> io::Result<()> {
        let effects = event(&mut self.control);
        self.effects(effects)
    }

    fn effects(&mut self, effects: Vec<Effect>) -> io::Result<()> {
        for effect in effects {
            self.effect(effect)?;
        }
        Ok(())
    }

    fn effect(&mut self, effect: Effect) -> io::Result<()> {
        match effect {
            Effect::ResizeSession { size } => self.resize_session(size),
            other => self.shutdown_effect(other),
        }
    }

    fn shutdown_effect(&mut self, effect: Effect) -> io::Result<()> {
        match effect {
            Effect::BeginShutdown => self.begin_shutdown(),
            Effect::Cleanup => self.run_cleanup(),
            other => {
                self.attachment_effect(other);
                Ok(())
            }
        }
    }

    fn attachment_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Admit { conn } => self.admit(conn.0),
            Effect::Refuse { conn } => self.refuse(conn.0),
            other => self.input_effect(other),
        }
    }

    fn input_effect(&mut self, effect: Effect) {
        match effect {
            Effect::EnableInput { conn } => self.enable_input(conn.0),
            Effect::DiscardPendingInput { conn } => self.discard_input(conn.0),
            other => self.presentation_effect(other),
        }
    }

    fn presentation_effect(&mut self, effect: Effect) {
        match effect {
            Effect::ClosePaste => self.close_paste(),
            Effect::ResizeHost { conn, host } => self.resize_host(conn.0, host),
            other => self.period_effect(other),
        }
    }

    fn period_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Release { conn } => self.release(conn.0),
            Effect::Revoke { conn } => self.revoke(conn.0),
            other => self.departure_effect(other),
        }
    }

    fn departure_effect(&mut self, effect: Effect) {
        match effect {
            Effect::AcceptDetach { conn } => self.terminal(conn.0, ServerMessage::Detached),
            Effect::Disconnect { conn } => self.remove_connection(conn.0),
            other => self.result_effect(other),
        }
    }

    fn result_effect(&mut self, effect: Effect) {
        match effect {
            Effect::DeliverExit { conn, code } => self.ending(conn.0, ServerMessage::Exit { code }),
            Effect::DeliverError { conn } => self.ending(conn.0, failure_message()),
            other => self.final_effect(other),
        }
    }

    fn final_effect(&mut self, effect: Effect) {
        match effect {
            Effect::ConfirmKill { conn, outcome } => self.confirm_kill(conn.0, &outcome),
            Effect::Finish => self.finish(),
            other => self.grant_effect(other),
        }
    }

    fn grant_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Grant {
                conn,
                authority,
                host,
                reason,
            } => self.granted(conn.0, authority, host, reason),
            Effect::Restore {
                conn,
                authority,
                host,
            } => self.replaced(conn.0, authority, host),
            // Every other effect was dispatched earlier in the chain.
            _ => {}
        }
    }

    /// Control is granted at once; the snapshot waits for the pending size
    /// to apply and for any synchronized update to end.
    fn granted(&mut self, id: u64, authority: Authority, host: TerminalSize, reason: GrantReason) {
        self.report_grant(id, authority, host, reason);
        self.pending_restorations.insert(id, (authority, host));
    }

    /// A replaced restoration's output ends at once.
    fn replaced(&mut self, id: u64, authority: Authority, host: TerminalSize) {
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            delivery.retire();
        }
        self.report_restoration(id, authority, host);
        self.pending_restorations.insert(id, (authority, host));
    }

    fn resize_session(&mut self, size: TerminalSize) -> io::Result<()> {
        if self.pty.resize(size).is_err() {
            return self.fail("cannot resize the command terminal");
        }
        if let Some(geometry) = Geometry::for_host(size.cols, size.rows) {
            self.vt.resize(geometry);
        }
        Ok(())
    }

    /// The grace is a loop deadline, so probes, refusals and joining kills
    /// are served while the command session hangs up. A failed hang-up
    /// confirms nothing.
    fn begin_shutdown(&mut self) -> io::Result<()> {
        if self.pty.hang_up().is_err() {
            return self.end_command(self.unconfirmed());
        }
        self.command = Command::HangingUp {
            until: Instant::now() + HANGUP_GRACE,
        };
        Ok(())
    }

    fn run_cleanup(&mut self) -> io::Result<()> {
        let effects = match self.clean_up() {
            Ok(()) => self.control.cleanup_finished(),
            Err(_) => self.control.authoritative_failure(),
        };
        self.effects(effects)
    }

    /// Removes the session metadata and socket.
    fn clean_up(&mut self) -> io::Result<()> {
        #[cfg(feature = "test-hooks")]
        if crate::startup::selected_fault("cleanup") {
            return Err(super::io_error("controlled cleanup fault"));
        }
        (self.cleanup)()
    }

    fn finish(&mut self) {
        self.finish_deadline
            .get_or_insert_with(|| Instant::now() + PTY_DRAIN_CAP);
    }

    fn admit(&mut self, id: u64) {
        let delivery = Delivery::new(self.control.session_size());
        self.deliveries.insert(id, delivery);
        self.reply(id, ServerMessage::Attached);
        self.admitted(id);
    }

    fn refuse(&mut self, id: u64) {
        self.refused(id);
        self.close_with_error(id, "session is terminating");
    }

    fn close_with_error(&mut self, id: u64, message: &str) {
        self.terminal(
            id,
            ServerMessage::Error {
                message: message.into(),
            },
        );
    }

    fn enable_input(&mut self, id: u64) {
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            delivery.ready();
        }
    }

    fn discard_input(&mut self, id: u64) {
        self.pty_writes.discard(id);
    }

    fn close_paste(&mut self) {
        let bytes = self.paste.cleanup(self.vt.bracketed_paste());
        self.pty_writes.daemon(&bytes);
    }

    fn resize_host(&mut self, id: u64, host: TerminalSize) {
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            delivery.resize(host);
        }
        self.report_geometry(id);
    }

    fn release(&mut self, id: u64) {
        self.pending_restorations.remove(&id);
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            let bytes = delivery.release();
            self.send_output(id, &bytes);
        }
    }

    fn revoke(&mut self, id: u64) {
        self.pending_restorations.remove(&id);
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            let bytes = delivery.end_sync();
            delivery.retire();
            self.send_output(id, &bytes);
        }
        self.forget_controller(id);
        self.reply(id, ServerMessage::Revoked);
        self.report_revocation(id, RevocationReason::Takeover);
    }

    /// A kill requester's answer: a standalone Kill's Response, or the
    /// private protocol's confirmation.
    fn confirm_kill(&mut self, id: u64, outcome: &Outcome) {
        match self.kill_request(id) {
            Some(request_id) => self.answer_kill(id, request_id, outcome),
            None => self.terminal(id, private_confirmation(outcome)),
        }
    }

    /// An attachment's result: control it still holds ends as the session
    /// ends, then the connection's last message.
    fn ending(&mut self, id: u64, message: ServerMessage) {
        self.report_revocation(id, RevocationReason::SessionEnding);
        self.terminal(id, message);
    }

    /// Sends a connection's last message and stops its session output.
    fn terminal(&mut self, id: u64, message: ServerMessage) {
        self.reply(id, message);
        if let Some(connection) = self.connections.get_mut(&id) {
            connection.close_after_flush();
        }
        self.remove_delivery(id);
    }

    // Connection bookkeeping.

    /// Session output in order: Display under an interactive attachment's
    /// authority, private Output otherwise. Nothing is sent for no bytes.
    fn send_output(&mut self, id: u64, bytes: &[u8]) {
        let authority = self.display_authority(id);
        for chunk in bytes.chunks(DISPLAY_CHUNK) {
            match authority {
                Some(authority) => self.display(id, authority, chunk),
                None => self.reply(id, ServerMessage::Output(chunk.to_vec())),
            }
        }
    }

    /// A private message; a revision-1 connection never receives one.
    fn reply(&mut self, id: u64, message: ServerMessage) {
        if self.opened.contains_key(&id) {
            return;
        }
        let Some(connection) = self.connections.get_mut(&id) else {
            return;
        };
        let queued = connection.queue(message);
        self.queued_or_lost(id, queued);
    }

    fn queue_frame(&mut self, id: u64, frame: Frame) {
        let Some(connection) = self.connections.get_mut(&id) else {
            return;
        };
        let queued = connection.queue_frame(frame);
        self.queued_or_lost(id, queued);
    }

    fn queued_or_lost(&mut self, id: u64, queued: io::Result<()>) {
        if let Err(error) = queued {
            tracing::debug!(connection_id = id, %error, "session output dropped");
            self.mark_lost(id);
        }
    }

    fn lost(&mut self, id: u64) -> io::Result<()> {
        self.react(|control| control.lost(ConnectionId(id)))?;
        self.remove_connection(id);
        Ok(())
    }

    /// Loses a connection from inside output routing; the state machine
    /// hears of it after this iteration's output.
    fn mark_lost(&mut self, id: u64) {
        self.pending_lost.insert(id);
        self.remove_connection(id);
    }

    fn remove_connection(&mut self, id: u64) {
        self.connections.remove(&id);
        self.remove_delivery(id);
    }

    /// Stops all session output to a connection, including a passive
    /// monitor connection's core metadata.
    fn remove_delivery(&mut self, id: u64) {
        self.deliveries.remove(&id);
        self.opened.remove(&id);
        self.pty_writes.discard(id);
        self.pending_restorations.remove(&id);
        self.forget_controller(id);
    }

    fn forget_controller(&mut self, id: u64) {
        if self.controller == Some(id) {
            self.controller = None;
        }
    }
}

/// `WouldBlock` ends this iteration's accepting, as does a peer of another
/// user, refused by authentication.
fn accepted(result: io::Result<SocketStream>) -> io::Result<Option<SocketStream>> {
    result.map(Some).or_else(|error| {
        let refused = matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::PermissionDenied
        );
        if refused { Ok(None) } else { Err(error) }
    })
}

fn control_effects(
    control: &mut Control,
    conn: ConnectionId,
    message: ClientMessage,
) -> Vec<Effect> {
    match message {
        ClientMessage::Attach(size) => control.attach(conn, size),
        ClientMessage::Ready {
            period: restoration,
        } => control.ready(conn, Restoration(restoration)),
        other => departure_effects(control, conn, other),
    }
}

fn departure_effects(
    control: &mut Control,
    conn: ConnectionId,
    message: ClientMessage,
) -> Vec<Effect> {
    match message {
        ClientMessage::TakeControl => control.take_control(conn),
        ClientMessage::Detach => control.detach(conn),
        other => ending_effects(control, conn, &other),
    }
}

/// `Kill`, or a client `Error`, which is an unexpected loss.
fn ending_effects(
    control: &mut Control,
    conn: ConnectionId,
    message: &ClientMessage,
) -> Vec<Effect> {
    if matches!(message, ClientMessage::Kill) {
        return control.kill(conn);
    }
    control.lost(conn)
}

/// `Terminated` with the private protocol's status, or `Error`.
fn private_confirmation(outcome: &Outcome) -> ServerMessage {
    match outcome.private_code() {
        Some(code) => ServerMessage::Terminated { code },
        None => failure_message(),
    }
}

fn failure_message() -> ServerMessage {
    ServerMessage::Error {
        message: FAILURE_MESSAGE.into(),
    }
}

/// An ending session finishes at its drain deadline, with output still queued for a client
/// that stopped reading, or sooner once every connection has drained.
fn finished_at(deadline: Option<Instant>, now: Instant, drained: impl FnOnce() -> bool) -> bool {
    deadline.is_some_and(|deadline| super::passed(deadline, now) || drained())
}

/// Only a session still awaiting its initial attachment keeps an early command exit for
/// that attachment, and only until its initial deadline. A headless session is never
/// `Starting`, so it never holds one.
fn holds_exit(lifecycle: Lifecycle, initial_deadline: Instant, now: Instant) -> bool {
    lifecycle == Lifecycle::Starting && !super::passed(initial_deadline, now)
}

#[cfg(test)]
mod tests {
    use super::{finished_at, holds_exit};
    use crate::control::Control;
    use climon2_core::{lifetime::Lifetime, protocol::TerminalSize};
    use std::time::{Duration, Instant};

    /// HDL-01 (pure decision): an ordinary session keeps an
    /// early command exit for its first attachment until its initial attach
    /// window closes; a headless session, `detached` from startup, never holds it.
    #[test]
    fn only_an_unattached_ordinary_session_holds_an_early_exit_until_its_initial_deadline() {
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        let unattached = |lifetime| {
            Control::new(TerminalSize { cols: 80, rows: 24 }, lifetime)
                .snapshot()
                .lifecycle
        };
        assert!(holds_exit(unattached(Lifetime::Ordinary), later, now));
        assert!(
            !holds_exit(unattached(Lifetime::Ordinary), now, now),
            "window closed"
        );
        assert!(
            !holds_exit(unattached(Lifetime::Headless), later, now),
            "headless"
        );
    }

    #[test]
    fn an_ending_session_finishes_at_its_deadline_or_once_drained() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert!(!finished_at(None, now, || true), "not ending");
        assert!(!finished_at(Some(later), now, || false), "draining");
        assert!(finished_at(Some(later), now, || true), "drained early");
        assert!(finished_at(Some(now), now, || false), "deadline reached");
    }
}
