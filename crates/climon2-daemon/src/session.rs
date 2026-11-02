//! The session loop: one continuous PTY pump feeding the VT, delivery to the
//! controller, and the control state machine's effects (plan section 3.3).

mod command;
mod delivery;
mod paste;
mod pending;
mod writes;

use crate::{
    args::Options,
    connection::Connection,
    control::{ConnectionId, Control, Effect, Lifecycle, Period},
    pty::Pty,
};
use climon2_core::{
    exit::CLIMON2_ERROR,
    hello::{SessionState, advertise},
    protocol::{
        ClientMessage, Frame, HANGUP_GRACE, HelloReply, MAX_DATA_FRAME_LENGTH, PROTOCOL_VERSION,
        PTY_DRAIN_CAP, ServerMessage, TerminalSize,
    },
    title::default_title,
};
use climon2_platform::{
    peer,
    runtime::{SocketListener, SocketStream},
    terminal::Signals,
};
use climon2_vt::{Geometry, Vt, VtConfig};
use command::{Command, Drain, Read, classify};
use delivery::{Delivery, LIVE_LIMIT, Out};
use paste::PasteTracker;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::{Duration, Instant},
};
use writes::PtyWrites;

const OUTPUT_CHUNK: usize = 32 * 1024;
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
    control: Control,
    vt: Vt,
    next_id: u64,
    controller: Option<u64>,
    pending_grants: BTreeMap<u64, (Period, TerminalSize)>,
    pending_lost: BTreeSet<u64>,
    pty_writes: PtyWrites,
    paste: PasteTracker,
    command: Command,
    initial_deadline: Instant,
    finish_deadline: Option<Instant>,
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
        control: Control::new(options.size),
        vt,
        next_id: 0,
        controller: None,
        pending_grants: BTreeMap::new(),
        pending_lost: BTreeSet::new(),
        pty_writes: PtyWrites::default(),
        paste: PasteTracker::default(),
        command: Command::Running,
        initial_deadline: Instant::now() + INITIAL_TIMEOUT,
        finish_deadline: None,
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
        for _ in 0..ACCEPTS_PER_TICK {
            let Some(stream) = accepted(peer::accept(self.listener))? else {
                break;
            };
            self.add_connection(stream)?;
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

    fn received(&mut self, id: u64, received: io::Result<Vec<ClientMessage>>) -> io::Result<()> {
        match received {
            Ok(messages) => self.messages(id, messages),
            Err(error) => self.protocol_failure(id, &error),
        }
    }

    fn messages(&mut self, id: u64, messages: Vec<ClientMessage>) -> io::Result<()> {
        for message in messages {
            if !self.connections.contains_key(&id) {
                break;
            }
            self.message(id, message)?;
        }
        Ok(())
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
        self.queue_frame(id, advertise(&reply, state)?);
        Ok(())
    }

    /// Input is admitted only from the ready controller of the current period.
    fn input(&mut self, id: u64, bytes: &[u8]) {
        if !self.control.admits_input(ConnectionId(id)) {
            return;
        }
        self.pty_writes.input(id, bytes);
    }

    fn protocol_failure(&mut self, id: u64, error: &io::Error) -> io::Result<()> {
        if let Some(connection) = self.connections.get_mut(&id) {
            connection.error(&protocol_error(error));
            connection.close_after_flush();
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

    fn end_hangup(&mut self, now: Instant) -> io::Result<()> {
        if !self.command.hung_up(now) {
            return Ok(());
        }
        let code = self.pty.end_hangup().unwrap_or(CLIMON2_ERROR);
        self.end_command(code)
    }

    fn end_command(&mut self, code: u32) -> io::Result<()> {
        self.command = Command::Ended;
        self.react(|control| control.command_exited(code))
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
        if let Some(code) = status {
            self.command = Command::Draining {
                code,
                drain: Drain::new(now),
            };
        }
        Ok(())
    }

    fn initial_deadline(&mut self, now: Instant) -> io::Result<()> {
        if self.running() && now >= self.initial_deadline {
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
        self.start_grants(now);
        self.deliver(now);
        Ok(())
    }

    fn read_pty(&mut self, now: Instant) -> io::Result<()> {
        if !self.command.reads(now) {
            return Ok(());
        }
        let mut bytes = vec![0; OUTPUT_CHUNK];
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
        self.route(fed.live);
    }

    fn route(&mut self, live: Vec<u8>) {
        let Some(id) = self.controller else {
            return;
        };
        let queued = self.queued(id);
        let Some(delivery) = self.deliveries.get_mut(&id) else {
            return;
        };
        match delivery.live(live, queued) {
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

    fn start_grants(&mut self, now: Instant) {
        if self.vt.sync_pending(now) {
            return;
        }
        for (id, (period, host)) in std::mem::take(&mut self.pending_grants) {
            self.grant(id, period, host, now);
        }
    }

    fn grant(&mut self, id: u64, period: Period, host: TerminalSize, now: Instant) {
        let Some(delivery) = self.deliveries.get_mut(&id) else {
            return;
        };
        let lead = delivery.grant(&mut self.vt, period, host, now);
        self.controller = Some(id);
        self.reply(id, ServerMessage::Granted { period: period.0 });
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
            Out::RestoreEnd(period) => {
                self.reply(id, ServerMessage::RestoreEnd { period: period.0 })
            }
        }
    }

    /// Committed live bytes beyond the live queue limit lose the attachment.
    fn commit_live(&mut self, id: u64, bytes: Vec<u8>) {
        if self.queued(id) + bytes.len() > LIVE_LIMIT {
            return self.mark_lost(id);
        }
        self.send_output(id, &bytes);
    }

    fn queued(&self, id: u64) -> usize {
        self.connections
            .get(&id)
            .map_or(0, Connection::queued_bytes)
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
        for (id, period) in expired {
            self.react(|control| control.restore_deadline(ConnectionId(id), period))?;
        }
        Ok(())
    }

    /// The exit is reported once output drained, the initial attachment had
    /// its chance and no grant awaits `Ready`.
    fn report_exit(&mut self, now: Instant) -> io::Result<()> {
        let Some(code) = self.command.drained(now) else {
            return Ok(());
        };
        if self.awaiting(now) {
            return Ok(());
        }
        self.end_command(code)
    }

    fn awaiting(&self, now: Instant) -> bool {
        self.starting(now) || self.awaiting_ready()
    }

    fn starting(&self, now: Instant) -> bool {
        self.control.snapshot().lifecycle == Lifecycle::Starting && now < self.initial_deadline
    }

    fn awaiting_ready(&self) -> bool {
        !self.pending_grants.is_empty()
            || self
                .controller
                .is_some_and(|id| !self.control.admits_input(ConnectionId(id)))
    }

    fn finished(&self, now: Instant) -> bool {
        self.finish_deadline.is_some_and(|deadline| {
            now >= deadline || self.connections.values().all(Connection::available)
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
            Effect::Grant { conn, period, host } => {
                self.pending_grants.insert(conn.0, (period, host));
                Ok(())
            }
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
            Effect::Refuse { conn } => self.close_with_error(conn.0, "session is terminating"),
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
            Effect::DeliverExit { conn, code } => {
                self.terminal(conn.0, ServerMessage::Exit { code })
            }
            Effect::DeliverError { conn } => self.terminal(conn.0, failure_message()),
            other => self.final_effect(other),
        }
    }

    fn final_effect(&mut self, effect: Effect) {
        match effect {
            Effect::ConfirmKill { conn, code } => {
                self.terminal(conn.0, ServerMessage::Terminated { code });
            }
            Effect::Finish => self.finish(),
            // Every other effect was dispatched earlier in the chain.
            _ => {}
        }
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
    /// are served while the command session hangs up.
    fn begin_shutdown(&mut self) -> io::Result<()> {
        if self.pty.hang_up().is_err() {
            return self.end_command(CLIMON2_ERROR);
        }
        self.command = Command::HangingUp {
            until: Instant::now() + HANGUP_GRACE,
        };
        Ok(())
    }

    fn run_cleanup(&mut self) -> io::Result<()> {
        let effects = match (self.cleanup)() {
            Ok(()) => self.control.cleanup_finished(),
            Err(_) => self.control.authoritative_failure(),
        };
        self.effects(effects)
    }

    fn finish(&mut self) {
        self.finish_deadline
            .get_or_insert_with(|| Instant::now() + PTY_DRAIN_CAP);
    }

    fn admit(&mut self, id: u64) {
        let delivery = Delivery::new(self.control.session_size());
        self.deliveries.insert(id, delivery);
        self.reply(id, ServerMessage::Attached);
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
    }

    fn release(&mut self, id: u64) {
        self.pending_grants.remove(&id);
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            let bytes = delivery.release();
            self.send_output(id, &bytes);
        }
    }

    fn revoke(&mut self, id: u64) {
        self.pending_grants.remove(&id);
        if let Some(delivery) = self.deliveries.get_mut(&id) {
            let bytes = delivery.end_sync();
            delivery.retire();
            self.send_output(id, &bytes);
        }
        self.forget_controller(id);
        self.reply(id, ServerMessage::Revoked);
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

    fn send_output(&mut self, id: u64, bytes: &[u8]) {
        for chunk in bytes.chunks(MAX_DATA_FRAME_LENGTH - 1) {
            self.reply(id, ServerMessage::Output(chunk.to_vec()));
        }
    }

    fn reply(&mut self, id: u64, message: ServerMessage) {
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

    fn remove_delivery(&mut self, id: u64) {
        self.deliveries.remove(&id);
        self.pty_writes.discard(id);
        self.pending_grants.remove(&id);
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
        ClientMessage::Ready { period } => control.ready(conn, Period(period)),
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

fn failure_message() -> ServerMessage {
    ServerMessage::Error {
        message: FAILURE_MESSAGE.into(),
    }
}

fn protocol_error(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::InvalidData {
        return error.to_string();
    }
    "invalid session protocol".into()
}
