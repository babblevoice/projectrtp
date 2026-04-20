// Channel actor — the tokio task that owns ChannelState while Local.
//
// The actor has two modes:
//
//   Local  — owns Box<ChannelState> + Subsystems and runs its own 20 ms
//            ticker. Handles every command in-process.
//   Mixed  — state + subs have migrated into a MixGroup actor. The channel
//            actor becomes a forwarder: JS commands get relayed into the
//            mixer via `MixHandle::forward()`. Close / LeaveMix pull the
//            Member back out of the mixer and restore Local ownership.
//
// This is the "bite the bullet" design the MIGRATE.md note describes: the
// mixer owns the tick so all the cross-channel math (summed_minus, DTMF fan-
// out) runs in lockstep, free of the old deposit/emit race.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{interval, Interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::commands::{Command, Handle};
use super::dtmf::{DtmfReceiver, DtmfSender};
use super::mixer::{Member, MixHandle};
use super::player::Player;
use super::recorder::{FinishReason, Recorder, RecorderState};
use super::state::{ChannelState, CloseInfo};
use super::tick::{self, TickOutcome};

pub const TICK_MS: u64 = 20;
pub const DEFAULT_CMD_QUEUE_DEPTH: usize = 64;

/// Events the actor emits back to the outside world. The JS facade (Task #9)
/// will bridge these into a napi `ThreadsafeFunction`.
#[derive(Debug, Clone, Default)]
pub struct ChannelStats {
    pub in_count: u64,
    pub in_dropped: u64,
    pub in_skip: u64,
    pub out_count: u64,
}

impl ChannelStats {
    /// Compute MOS from packet loss, using the ITU-T G.107 inspired formula
    /// from the C++ codebase (originally borrowed from FreeSWITCH).
    /// Returns 0.0 when no packets were received.
    pub fn mos(&self) -> f64 {
        if self.in_count == 0 { return 0.0; }
        let r = ((self.in_count - self.in_skip) as f64 / self.in_count as f64) * 100.0;
        let r = r.clamp(0.0, 100.0);
        1.0 + (0.035 * r) + (0.000007 * r * (r - 60.0) * (100.0 - r))
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Close { reason: String, stats: ChannelStats },
    Play { state: PlayState, reason: Option<String> },
    Record { state: RecordState, reason: Option<String>, file: Option<String>, filesize: Option<u64> },
    TelephoneEvent { digit: char },
    Mix { state: String },
}

#[derive(Debug, Clone)]
pub enum PlayState { Start, End }

#[derive(Debug, Clone)]
pub enum RecordState { Recording, Finished }

/// Abstract sink for events — a trait so tests can capture events into a
/// channel and the napi facade can forward into a ThreadsafeFunction.
pub trait EventSink: Send + Sync + 'static {
    fn post(&self, ev: Event);
}

pub struct SpawnConfig {
    pub id: u64,
    #[allow(dead_code)]
    pub bind_addr: SocketAddr,
    pub ssrc: u32,
    pub events: Arc<dyn EventSink>,
    /// `Some` when the port came from the managed pool — handed to
    /// ChannelState so the port is returned on actor exit.
    pub port_reservation: Option<crate::portpool::PortReservation>,
    /// Our own ICE password — used for STUN Binding Request integrity checks.
    pub local_icepwd: String,
}

#[cfg(test)]
pub async fn spawn(cfg: SpawnConfig) -> std::io::Result<Handle> {
    let rtp_sock = UdpSocket::bind(cfg.bind_addr).await?;
    let local_addr = rtp_sock.local_addr()?;
    let rtcp_port = local_addr.port().checked_add(1).unwrap_or(local_addr.port());
    let rtcp_sock = UdpSocket::bind(SocketAddr::new(local_addr.ip(), rtcp_port)).await?;
    spawn_with_sockets(cfg, rtp_sock, rtcp_sock, local_addr)
}

/// Sync variant: takes already-bound sockets. Used by the JS facade so
/// openchannel() can return a value (not a Promise) that index.js can read
/// `.local.port` on immediately.
pub fn spawn_with_sockets(
    cfg: SpawnConfig,
    rtp_sock: UdpSocket,
    rtcp_sock: UdpSocket,
    local_addr: SocketAddr,
) -> std::io::Result<Handle> {
    let mut state = ChannelState::new(cfg.id, local_addr, rtp_sock, rtcp_sock, cfg.ssrc);
    state.port_reservation = cfg.port_reservation;
    *state.local_icepwd.lock() = cfg.local_icepwd;

    // Spawn the recv_loop — reads the socket continuously, classifies
    // STUN/DTLS/RTP and feeds jitter/DTLS-mpsc immediately.
    let cancel = CancellationToken::new();
    super::recv_loop::spawn(super::recv_loop::RecvLoopConfig {
        sock: state.rtp_sock.clone(),
        jitter: state.jitter.clone(),
        remote_addr: state.remote_addr.clone(),
        in_count: state.in_count.clone(),
        local_icepwd: state.local_icepwd.clone(),
        dtls_tx: state.dtls_inbound_tx.clone(),
        cancel: cancel.clone(),
    });
    state.recv_cancel = Some(cancel);

    let (tx, rx) = mpsc::channel::<Command>(DEFAULT_CMD_QUEUE_DEPTH);
    let events = cfg.events;
    let handle_cmd = tx.clone();
    tokio::spawn(async move { run(Box::new(state), rx, events, handle_cmd).await });
    Ok(Handle { id: cfg.id, cmd: tx })
}

/// Per-channel subsystems the tick pipeline hands off to. Keeping them in a
/// sibling struct (not in ChannelState) avoids mixing pipeline-owned and
/// control-owned state — the mixer will move ChannelState but leaves these
/// with the channel actor, since they're driven by commands more than ticks.
#[derive(Default)]
pub struct Subsystems {
    pub player: Option<Player>,
    /// Multiple simultaneous recorders — one ongoing + one power-gated is the
    /// primary use case (see test/interface/projectrtprecord.js "dual
    /// recording"). Keyed informally by `recorder.file()`; same path replaces
    /// the existing recorder, different path coexists.
    pub recorders: Vec<Recorder>,
    /// Barge-in detector for `playrecord`. When the player is playing and
    /// inbound RMS crosses `power_threshold`, the player is interrupted and
    /// a `play/end reason=interrupted` event fires. Cleared when the player
    /// ends (naturally or via interrupt).
    pub bargein: Option<BargeInState>,
    /// JS-initiated dtmf via `channel.dtmf(...)` — targets state.remote_addr.
    pub dtmf_send: DtmfSender,
    /// Mix-relay dtmf — when a digit is detected on a peer's inbound while
    /// mixed, a full RFC 2833 burst is enqueued here and emitted to the
    /// local remote on the next tick. Matches the C++ mux DTMF behaviour.
    pub dtmf_relay: DtmfSender,
    pub dtmf_recv: DtmfReceiver,
}

pub struct BargeInState {
    pub power_threshold: i32,
    pub power_ma: crate::firfilter::MaFilter,
}

/// Top-level actor ownership. `Local` runs the per-channel ticker; `Mixed`
/// delegates ticks + most commands to the mix actor.
enum Mode {
    Local {
        state: Box<ChannelState>,
        subs: Subsystems,
    },
    Mixed {
        mix: MixHandle,
    },
}

async fn run(
    initial_state: Box<ChannelState>,
    mut cmds: mpsc::Receiver<Command>,
    events: Arc<dyn EventSink>,
    self_cmd: mpsc::Sender<Command>,
) {
    let id = initial_state.id;
    let mut mode = Mode::Local { state: initial_state, subs: Subsystems::default() };
    let mut ticker = interval(Duration::from_millis(TICK_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut closing: Option<String> = None;

    while closing.is_none() {
        match &mut mode {
            Mode::Local { state, subs } => {
                match step_local(state, subs, &mut cmds, &mut ticker, &events).await {
                    LocalStep::Continue => {}
                    LocalStep::Close(reason) => closing = Some(reason),
                    LocalStep::EnterMix { mix, ack } => {
                        // Migrate into the mixer. Need to move out of `mode`
                        // which requires taking ownership — swap to a temp
                        // Local with a dummy then rebuild the real Mixed.
                        let (state_out, mut subs_out) = take_local(&mut mode);
                        // Clear jitter so a remixed channel that restarts
                        // its SN counter doesn't get rejected as out-of-window.
                        state_out.jitter.lock().clear();
                        // Stop any active player — matches C++ mix2 behavior.
                        // Without this, a ringing player on an existing channel
                        // leaks into the mix output (e.g. blind transfer: b
                        // hears ringing while c rings, then c answers and the
                        // mix is established — the ringing must stop).
                        if subs_out.player.is_some() {
                            subs_out.player = None;
                            subs_out.bargein = None;
                            events.post(Event::Play {
                                state: PlayState::End,
                                reason: Some("mix".into()),
                            });
                        }
                        match mix.add(Box::new(Member::new(state_out, subs_out, events.clone()))).await {
                            Ok(()) => {
                                events.post(Event::Mix { state: "start".to_string() });
                                mode = Mode::Mixed { mix };
                                let _ = ack.send(());
                            }
                            Err(()) => {
                                // Mixer is gone (shouldn't happen in normal
                                // flow). Fall back to Local mode — state was
                                // consumed into the `Member` that got
                                // dropped. Repopulate a zero-state placeholder
                                // and close so the channel doesn't hang.
                                closing = Some("mix-add-failed".to_string());
                            }
                        }
                    }
                }
            }
            Mode::Mixed { mix } => {
                match cmds.recv().await {
                    None => closing = Some("handle-dropped".to_string()),
                    Some(Command::Close { reason }) => {
                        if let Some(member) = mix.remove(id).await {
                            restore_local(&mut mode, member);
                            events.post(Event::Mix { state: "finished".to_string() });
                        }
                        closing = Some(reason);
                    }
                    Some(Command::LeaveMix { ack }) => {
                        if let Some(member) = mix.remove(id).await {
                            restore_local(&mut mode, member);
                            events.post(Event::Mix { state: "finished".to_string() });
                        }
                        let _ = ack.send(());
                    }
                    Some(Command::EnterMix { mix: new_mix, ack }) => {
                        // Already in a mix. If it's the same one (facade
                        // re-asserts on same-group mix()), post a fresh
                        // `mix/start` event so JS sees one per mix() call.
                        // Different-group migration isn't supported yet.
                        if new_mix.id == mix.id {
                            events.post(Event::Mix { state: "start".to_string() });
                        }
                        let _ = ack.send(());
                    }
                    Some(cmd) => {
                        // All other commands get forwarded to the mixer for
                        // application against this member's state/subs.
                        mix.forward(id, cmd).await;
                    }
                }
            }
        }
    }

    let reason = closing.unwrap_or_default();

    // If we arrived here via a path that left us mixed (shouldn't happen
    // since close/leavemix above pull out first, but be defensive), still
    // try to reclaim state for the stats payload. Use a timeout so a dead
    // mixer doesn't strand the actor.
    if let Mode::Mixed { mix } = &mode {
        if let Ok(Some(member)) = tokio::time::timeout(Duration::from_millis(100), mix.remove(id)).await {
            restore_local(&mut mode, member);
            events.post(Event::Mix { state: "finished".to_string() });
        }
    }

    let (state, subs) = match &mut mode {
        Mode::Local { state, subs } => (state, subs),
        Mode::Mixed { .. } => {
            // Last-resort: emit a minimal Close event without stats.
            events.post(Event::Close { reason, stats: ChannelStats::default() });
            drop(self_cmd);
            return;
        }
    };

    // Player: if still active on close, emit `play/end reason=channelclosed`.
    if subs.player.is_some() {
        subs.player = None;
        subs.bargein = None;
        events.post(Event::Play {
            state: PlayState::End,
            reason: Some("channelclosed".into()),
        });
    }
    for mut rec in subs.recorders.drain(..) {
        let file_str = rec.file().to_string_lossy().into_owned();
        let size = rec.file_size();
        rec.close(FinishReason::ChannelClosed);
        events.post(Event::Record {
            state: RecordState::Finished,
            reason: Some("channelclosed".into()),
            file: Some(file_str),
            filesize: Some(size),
        });
    }
    // Cancel the recv_loop before collecting stats.
    if let Some(cancel) = state.recv_cancel.take() {
        cancel.cancel();
    }
    let stats = ChannelStats {
        in_count: state.in_count.load(Ordering::Relaxed),
        in_dropped: state.in_dropped + state.jitter.lock().dropped,
        in_skip: state.in_skip,
        out_count: state.out_count,
    };
    state.close_info = Some(CloseInfo { reason: reason.clone() });
    events.post(Event::Close { reason, stats });
    super::facade::unregister_channel(id);
    drop(self_cmd);
}

/// Replaces `mode` temporarily with a placeholder Mixed so we can move the
/// owned state + subs out. Caller must immediately rebuild `mode` on the
/// success path. On failure the placeholder Mixed is left in place and the
/// actor proceeds to close.
fn take_local(mode: &mut Mode) -> (Box<ChannelState>, Subsystems) {
    // Swap out with a temporary Mixed placeholder — we'll overwrite mode
    // right after. The placeholder MixHandle is never used because the
    // caller transitions mode before any further command handling runs.
    let placeholder = Mode::Mixed { mix: MixHandle { id: 0, cmd: mpsc::channel(1).0 } };
    let taken = std::mem::replace(mode, placeholder);
    match taken {
        Mode::Local { state, subs } => (state, subs),
        Mode::Mixed { .. } => unreachable!("take_local called from non-Local mode"),
    }
}

fn restore_local(mode: &mut Mode, member: Box<Member>) {
    let Member { state, subs, .. } = *member;
    *mode = Mode::Local { state, subs };
}

enum LocalStep {
    Continue,
    Close(String),
    EnterMix {
        mix: MixHandle,
        ack: tokio::sync::oneshot::Sender<()>,
    },
}

async fn step_local(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    cmds: &mut mpsc::Receiver<Command>,
    ticker: &mut Interval,
    events: &Arc<dyn EventSink>,
) -> LocalStep {
    tokio::select! {
        biased;

        cmd = cmds.recv() => {
            match cmd {
                None => LocalStep::Close("handle-dropped".to_string()),
                Some(cmd) => match handle_command_local(state, subs, cmd, events).await {
                    LocalOutcome::Continue => LocalStep::Continue,
                    LocalOutcome::Close(r) => LocalStep::Close(r),
                    LocalOutcome::EnterMix { mix, ack } => LocalStep::EnterMix { mix, ack },
                }
            }
        }

        _ = ticker.tick() => {
            let outcome = tick::run(state, subs).await;
            for ev in state.pending_events.drain(..) {
                events.post(ev);
            }
            if outcome == TickOutcome::Stop {
                LocalStep::Close("idle-timeout".to_string())
            } else {
                LocalStep::Continue
            }
        }
    }
}

enum LocalOutcome {
    Continue,
    Close(String),
    EnterMix {
        mix: MixHandle,
        ack: tokio::sync::oneshot::Sender<()>,
    },
}

async fn handle_command_local(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    cmd: Command,
    events: &Arc<dyn EventSink>,
) -> LocalOutcome {
    match cmd {
        Command::Close { reason } => LocalOutcome::Close(reason),

        Command::EnterMix { mix, ack } => LocalOutcome::EnterMix { mix, ack },

        Command::LeaveMix { ack } => {
            // Not in a mix — ack immediately. Matches C++ `unmix()` returning
            // true even when the channel wasn't mixed.
            let _ = ack.send(());
            LocalOutcome::Continue
        }

        Command::Direction(d) => {
            state.direction = d;
            LocalOutcome::Continue
        }

        Command::Echo { enabled } => {
            state.echo = enabled;
            LocalOutcome::Continue
        }

        Command::Remote { cfg, ack } => {
            state.set_remote_addr(cfg.addr);
            state.ticks_without_rtp = 0;
            state.remote_pt = cfg.payload_type;
            if let Some(pt) = cfg.rfc2833_payload_type {
                state.rfc2833_pt = pt;
            }
            if let Some(pt) = cfg.ilbc_payload_type {
                state.transcoder.set_local_ilbc_pt(pt);
            }
            if let Some(pwd) = &cfg.icepwd {
                state.remote_icepwd = pwd.clone();
            }

            // DTLS: if the remote config includes DTLS setup, start the handshake.
            if let Some(ref dtls) = cfg.dtls {
                let (dtls_tx, dtls_rx) = mpsc::channel::<Vec<u8>>(64);
                *state.dtls_inbound_tx.lock() = Some(dtls_tx);
                let cert = webrtc_dtls::crypto::Certificate::generate_self_signed(
                    vec!["projectrtp".into()],
                ).expect("generate dtls cert");
                state.dtls_result_rx = Some(super::dtls_session::spawn_handshake(
                    dtls.setup,
                    state.local_addr,
                    state.rtp_sock.clone(),
                    dtls_rx,
                    cert,
                ));
            }

            state.remote = Some(cfg);
            state.remote_confirmed = true;
            let _ = ack.send(());
            LocalOutcome::Continue
        }

        Command::Play { cfg, ack } => {
            if subs.player.is_some() {
                subs.player = None;
                events.post(Event::Play { state: PlayState::End, reason: Some("new".into()) });
            }
            subs.player = Some(Player::new(cfg));
            events.post(Event::Play { state: PlayState::Start, reason: Some("new".into()) });
            let _ = ack.send(());
            LocalOutcome::Continue
        }

        Command::Record { cfg, ack } => {
            let file_str = cfg.file.to_string_lossy().into_owned();
            let is_gated = cfg.start_above_power.is_some();
            // If an existing recorder at the same path is paused, resume it
            // instead of replacing it — preserves the WAV so both segments
            // end up in one file.
            if let Some(rec) = subs.recorders.iter_mut().find(|r| r.file() == cfg.file && r.state() == RecorderState::Paused) {
                rec.resume();
                events.post(Event::Record {
                    state: RecordState::Recording,
                    reason: None,
                    file: Some(file_str),
                    filesize: None,
                });
            } else {
            match Recorder::open(cfg.clone()).await {
                Ok(rec) => {
                    if let Some(idx) = subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = subs.recorders.remove(idx);
                        let size = old.file_size();
                        old.close(FinishReason::ChannelClosed);
                        events.post(Event::Record {
                            state: RecordState::Finished,
                            reason: Some("channelclosed".into()),
                            file: Some(file_str.clone()),
                            filesize: Some(size),
                        });
                    }
                    subs.recorders.push(rec);
                    if !is_gated {
                        events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                            filesize: None,
                        });
                    }
                }
                Err(e) => {
                    events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                        filesize: None,
                    });
                }
            }
            }
            let _ = ack.send(());
            LocalOutcome::Continue
        }

        Command::RecordFinish { file } => {
            if let Some(idx) = subs.recorders.iter().position(|r| r.file() == file) {
                let mut rec = subs.recorders.remove(idx);
                let file_str = rec.file().to_string_lossy().into_owned();
                let size = rec.file_size();
                rec.close(FinishReason::Requested);
                events.post(Event::Record {
                    state: RecordState::Finished,
                    reason: Some("requested".into()),
                    file: Some(file_str),
                    filesize: Some(size),
                });
            }
            LocalOutcome::Continue
        }

        Command::RecordSetPaused { file, paused } => {
            if let Some(rec) = subs.recorders.iter_mut().find(|r| r.file() == file) {
                if paused { rec.pause(); } else { rec.resume(); }
            }
            LocalOutcome::Continue
        }

        Command::PlayRecord { cfg, ack } => {
            if subs.player.is_some() {
                subs.player = None;
                events.post(Event::Play { state: PlayState::End, reason: Some("new".into()) });
            }
            subs.player = Some(Player::new(cfg.player));
            events.post(Event::Play { state: PlayState::Start, reason: Some("new".into()) });

            if cfg.interrupt {
                if let Some(threshold) = cfg.bargein_power {
                    let mut ma = crate::firfilter::MaFilter::new();
                    if let Some(n) = cfg.bargein_packets {
                        ma.reset(n as usize);
                    }
                    subs.bargein = Some(BargeInState {
                        power_threshold: threshold,
                        power_ma: ma,
                    });
                }
            }

            let file_str = cfg.recorder.file.to_string_lossy().into_owned();
            let is_gated = cfg.recorder.start_above_power.is_some();
            match Recorder::open(cfg.recorder).await {
                Ok(rec) => {
                    if let Some(idx) = subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = subs.recorders.remove(idx);
                        old.close(FinishReason::ChannelClosed);
                    }
                    subs.recorders.push(rec);
                    if !is_gated {
                        events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                            filesize: None,
                        });
                    }
                }
                Err(e) => {
                    events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                        filesize: None,
                    });
                }
            }
            let _ = ack.send(());
            LocalOutcome::Continue
        }

        Command::Dtmf { digits } => {
            subs.dtmf_send.enqueue(&digits);
            LocalOutcome::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::commands::Direction;
    use std::sync::Mutex;
    use tokio::sync::mpsc as tmpsc;

    struct TestSink { tx: tmpsc::UnboundedSender<Event> }
    impl EventSink for TestSink {
        fn post(&self, ev: Event) { let _ = self.tx.send(ev); }
    }

    struct CountingSink { count: Mutex<Vec<Event>> }
    impl EventSink for CountingSink {
        fn post(&self, ev: Event) { self.count.lock().unwrap().push(ev); }
    }

    #[tokio::test]
    async fn close_command_exits_loop_and_posts_close_event() {
        let (tx, mut rx) = tmpsc::unbounded_channel();
        let sink = Arc::new(TestSink { tx });

        let handle = spawn(SpawnConfig {
            id: 42,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: 1,
            events: sink,
            port_reservation: None,
            local_icepwd: String::new(),
        }).await.unwrap();

        handle.close("test").await;

        let ev = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await.expect("no close event").expect("stream ended");
        match ev {
            Event::Close { reason, .. } => assert_eq!(reason, "test"),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn direction_command_mutates_state_via_tick_observable() {
        let sink = Arc::new(CountingSink { count: Mutex::new(Vec::new()) });
        let handle = spawn(SpawnConfig {
            id: 1,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: 1,
            events: sink.clone(),
            port_reservation: None,
            local_icepwd: String::new(),
        }).await.unwrap();

        handle.direction(Direction { send: false, recv: false }).await;
        handle.echo(true).await;
        handle.dtmf("1#").await;

        tokio::time::sleep(Duration::from_millis(80)).await;

        handle.close("ok").await;
        tokio::time::sleep(Duration::from_millis(80)).await;

        let events = sink.count.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, Event::Close { .. })));
    }

    #[tokio::test]
    async fn play_command_emits_play_start_event_and_acks() {
        let (tx, mut rx) = tmpsc::unbounded_channel();
        let sink = Arc::new(TestSink { tx });

        let handle = spawn(SpawnConfig {
            id: 1,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ssrc: 1,
            events: sink,
            port_reservation: None,
            local_icepwd: String::new(),
        }).await.unwrap();

        handle.play(crate::channel::player::SoundSoupSpec {
            files: vec![],
            overall_loops: None,
            interrupt: false,
        }).await.unwrap();

        let mut saw_play = false;
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            if matches!(ev, Event::Play { state: PlayState::Start, .. }) {
                saw_play = true;
                break;
            }
        }
        assert!(saw_play, "expected Play::Start event");

        handle.close("done").await;
    }
}
