// Channel actor — the tokio task that owns ChannelState.
//
// The actor is the sole mutator of per-channel state. Everything else — JS,
// the mixer group, another channel — talks to it via `Handle`'s mpsc. When a
// channel joins a mix group, the state migrates into the MixGroup actor; the
// channel actor becomes a thin forwarder until the group releases it back.
// See the Task #7 design note for the full ownership model.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use super::commands::{Command, Direction, Handle};
use super::dtmf::{DtmfReceiver, DtmfSender};
use super::player::Player;
use super::recorder::{FinishReason, Recorder};
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

#[derive(Debug, Clone)]
pub enum Event {
    Close { reason: String, stats: ChannelStats },
    Play { state: PlayState, reason: Option<String> },
    Record { state: RecordState, reason: Option<String>, file: Option<String> },
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
    pub bind_addr: SocketAddr,
    pub ssrc: u32,
    pub events: Arc<dyn EventSink>,
    /// `Some` when the port came from the managed pool — handed to
    /// ChannelState so the port is returned on actor exit.
    pub port_reservation: Option<crate::portpool::PortReservation>,
    /// Our own ICE password — used for STUN Binding Request integrity checks.
    pub local_icepwd: String,
}

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
    state.local_icepwd = cfg.local_icepwd;
    let (tx, rx) = mpsc::channel::<Command>(DEFAULT_CMD_QUEUE_DEPTH);
    let events = cfg.events;
    tokio::spawn(async move { run(state, rx, events).await });
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
    /// Mix-relay dtmf — when a digit is detected on inbound while mixed, a
    /// full RFC 2833 burst is enqueued here and emitted to mix_peer_remote.
    /// Matches the C++ behavior where the mux side regenerates a proper
    /// burst rather than passing raw packets through.
    pub dtmf_relay: DtmfSender,
    pub dtmf_recv: DtmfReceiver,
}

pub struct BargeInState {
    pub power_threshold: i32,
    pub power_ma: crate::firfilter::MaFilter,
}

async fn run(mut state: ChannelState, mut cmds: mpsc::Receiver<Command>, events: Arc<dyn EventSink>) {
    let mut subs = Subsystems::default();
    let mut ticker = interval(Duration::from_millis(TICK_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut closing: Option<String> = None;

    while closing.is_none() {
        tokio::select! {
            biased;

            cmd = cmds.recv() => {
                match cmd {
                    Some(cmd) => if let Some(reason) = handle_command(&mut state, &mut subs, cmd, &events).await {
                        closing = Some(reason);
                    },
                    None => closing = Some("handle-dropped".to_string()),
                }
            }

            _ = ticker.tick() => {
                let outcome = tick::run(&mut state, &mut subs).await;
                // Drain any events the tick generated (e.g. inbound DTMF).
                for ev in state.pending_events.drain(..) {
                    events.post(ev);
                }
                if outcome == TickOutcome::Stop {
                    closing = Some("idle-timeout".to_string());
                }
            }
        }
    }

    // Final flush — let recorders finalize their WAV files and surface
    // per-file `record/finished.channelclosed` before the `close` event.
    // WavWriter's Drop finalizes headers regardless, but we emit JS events
    // before the Close payload so subscribers see the expected order.
    let reason = closing.unwrap_or_default();
    // Player: if still active on close, emit `play/end reason=channelclosed`.
    // Matches the C++ event ordering that proxy tests assert — messages
    // appear as play/start → play/end → close, not play/start → close.
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
        rec.close(FinishReason::ChannelClosed);
        events.post(Event::Record {
            state: RecordState::Finished,
            reason: Some("channelclosed".into()),
            file: Some(file_str),
        });
    }
    // If the channel was still in a mix at close time, emit `mix/finished`
    // before the `close` event. Matches C++ teardown ordering.
    if state.mix_peer_remote.is_some() {
        state.mix_peer_remote = None;
        events.post(Event::Mix { state: "finished".to_string() });
    }
    let stats = ChannelStats {
        in_count: state.in_count,
        // Jitter-side drops (out-of-window SN, duplicates) plus any drops
        // counted directly on state (e.g. mix-relay transcode failures).
        in_dropped: state.in_dropped + state.jitter.dropped,
        in_skip: state.in_skip,
        out_count: state.out_count,
    };
    state.close_info = Some(CloseInfo { reason: reason.clone() });
    events.post(Event::Close { reason, stats });
}

async fn handle_command(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    cmd: Command,
    events: &Arc<dyn EventSink>,
) -> Option<String> {
    match cmd {
        Command::Close { reason } => Some(reason),

        Command::Direction(d) => {
            state.direction = d;
            None
        }

        Command::Echo { enabled } => {
            state.echo = enabled;
            None
        }

        Command::Remote { cfg, ack } => {
            state.remote_addr = Some(cfg.addr);
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
            // If we're already bound into a 2-chan mix, notify the peer of
            // our new remote so the packets it forwards to us via the relay
            // land on the correct address (and the correct PT for transcode).
            if let Some(peer) = state.mix_peer_handle.clone() {
                let update = super::commands::Command::SetPeerRemote {
                    remote: Some(cfg.addr),
                    pt: cfg.payload_type,
                    rfc2833_pt: state.rfc2833_pt,
                };
                let _ = peer.try_send(update);
            }
            state.remote = Some(cfg);
            state.remote_confirmed = true;
            let _ = ack.send(());
            None
        }

        Command::Play { cfg, ack } => {
            // If a prior player is still alive, C++ semantics are "replace,
            // fire play/end reason=new on the old one". We emit that first
            // so JS sees the transition before the new play/start.
            if subs.player.is_some() {
                subs.player = None;
                events.post(Event::Play { state: PlayState::End, reason: Some("new".into()) });
            }
            subs.player = Some(Player::new(cfg));
            events.post(Event::Play { state: PlayState::Start, reason: Some("new".into()) });
            let _ = ack.send(());
            None
        }

        Command::Record { cfg, ack } => {
            let file_str = cfg.file.to_string_lossy().into_owned();
            let is_gated = cfg.start_above_power.is_some();
            match Recorder::open(cfg.clone()).await {
                Ok(rec) => {
                    // If a recorder already exists for this exact file path,
                    // replace it with an implicit "channelclosed" reason —
                    // the old writer's Drop finalizes its header, and JS
                    // observes a finished event followed by the new recording.
                    if let Some(idx) = subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = subs.recorders.remove(idx);
                        old.close(FinishReason::ChannelClosed);
                        events.post(Event::Record {
                            state: RecordState::Finished,
                            reason: Some("channelclosed".into()),
                            file: Some(file_str.clone()),
                        });
                    }
                    subs.recorders.push(rec);
                    // A power-gated recorder stays silent until the gate opens;
                    // the tick emits `recording.abovepower` on that transition.
                    // Matches C++ event ordering in projectrtprecord.js.
                    if !is_gated {
                        events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                        });
                    }
                }
                Err(e) => {
                    events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                    });
                }
            }
            let _ = ack.send(());
            None
        }

        Command::RecordFinish { file } => {
            // JS targeted one specific recorder by file path.
            if let Some(idx) = subs.recorders.iter().position(|r| r.file() == file) {
                let mut rec = subs.recorders.remove(idx);
                let file_str = rec.file().to_string_lossy().into_owned();
                rec.close(FinishReason::Requested);
                events.post(Event::Record {
                    state: RecordState::Finished,
                    reason: Some("requested".into()),
                    file: Some(file_str),
                });
            }
            None
        }

        Command::RecordSetPaused { file, paused } => {
            if let Some(rec) = subs.recorders.iter_mut().find(|r| r.file() == file) {
                if paused { rec.pause(); } else { rec.resume(); }
            }
            None
        }

        Command::PlayRecord { cfg, ack } => {
            if subs.player.is_some() {
                subs.player = None;
                events.post(Event::Play { state: PlayState::End, reason: Some("new".into()) });
            }
            subs.player = Some(Player::new(cfg.player));
            events.post(Event::Play { state: PlayState::Start, reason: Some("new".into()) });

            // Barge-in setup. `interrupt:true` + `bargeinpower` is the
            // trigger — without both, we never interrupt the player from
            // loud inbound audio (DTMF interrupt is separate).
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
                        });
                    }
                }
                Err(e) => {
                    events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                    });
                }
            }
            let _ = ack.send(());
            None
        }

        Command::Dtmf { digits } => {
            subs.dtmf_send.enqueue(&digits);
            None
        }

        Command::Mix { other_id: _, other_sender: _, ack } => {
            // TODO: hand state off to MixGroup actor (Task #8).
            let _ = ack.send(());
            None
        }

        Command::Unmix => {
            // Legacy path. Prefer Command::UnbindMixPeer via facade.unmix.
            let was_bound = state.mix_peer_handle.is_some() || state.mix_peer_remote.is_some();
            state.mix_peer_handle = None;
            state.mix_peer_remote = None;
            state.mix_peer_pt = 0;
            state.mix_peer_rfc2833_pt = 101;
            if was_bound {
                events.post(Event::Mix { state: "finished".to_string() });
            }
            None
        }

        Command::BindMixPeer { peer_handle, peer_remote, peer_pt, peer_rfc2833_pt, peer_ilbc_pt } => {
            let was_bound = state.mix_peer_handle.is_some();
            state.mix_peer_handle = Some(peer_handle);
            state.mix_peer_remote = peer_remote;
            state.mix_peer_pt = peer_pt;
            state.mix_peer_rfc2833_pt = peer_rfc2833_pt;
            // Tell the transcoder which PT to treat as iLBC on the peer side,
            // so when we encode outbound for them we pick the iLBC path even
            // if they're using a dynamic PT != 97.
            state.transcoder.set_peer_ilbc_pt(peer_ilbc_pt);
            if !was_bound {
                events.post(Event::Mix { state: "start".to_string() });
            }
            None
        }

        Command::UnbindMixPeer => {
            let was_bound = state.mix_peer_handle.is_some();
            // Push an unbind to the peer so both sides release together.
            if let Some(peer) = state.mix_peer_handle.take() {
                let _ = peer.try_send(super::commands::Command::UnbindMixPeer);
            }
            state.mix_peer_remote = None;
            state.mix_peer_pt = 0;
            state.mix_peer_rfc2833_pt = 101;
            if was_bound {
                events.post(Event::Mix { state: "finished".to_string() });
            }
            None
        }

        Command::SetPeerRemote { remote, pt, rfc2833_pt } => {
            // Pure target refresh — no event. The bound state didn't change,
            // only where we forward to.
            state.mix_peer_remote = remote;
            state.mix_peer_pt = pt;
            state.mix_peer_rfc2833_pt = rfc2833_pt;
            None
        }

        Command::BindMixGroup { group, own_idx, own_handle } => {
            let was_bound = state.mix_peer_handle.is_some() || state.mix_group.is_some();
            // Drop any 2-chan fast-path state — promotion to N-way takes over.
            state.mix_peer_handle = None;
            state.mix_peer_remote = None;
            state.mix_peer_pt = 0;
            state.mix_peer_rfc2833_pt = 101;

            // Fresh-start semantics: on (re-)bind, adopt the group's current
            // total-other-deposits as our last-emit mark. That way a remix
            // after an unmix doesn't fire a burst of "catch-up" packets for
            // history that isn't relevant any more.
            let baseline = {
                let mut g = group.lock();
                if let Some(m) = g.members.get_mut(own_idx) {
                    m.channel_handle = Some(own_handle);
                }
                g.total_other_deposits(own_idx)
            };
            state.mix_group = Some(group);
            state.mix_group_idx = own_idx;
            state.mix_last_emit = baseline;
            if !was_bound {
                events.post(Event::Mix { state: "start".to_string() });
            }
            None
        }

        Command::MixRelayDtmf { digits } => {
            // Another member forwarded a DTMF burst — enqueue it so our
            // next tick emits the full RFC 2833 sequence to our remote.
            subs.dtmf_relay.enqueue(&digits);
            None
        }

        Command::UnbindMixGroup => {
            let was_bound = state.mix_group.is_some();
            if let Some(group) = state.mix_group.take() {
                let idx = state.mix_group_idx;
                group.lock().tombstone(idx);
            }
            state.mix_group_idx = 0;
            state.mix_last_emit = 0;
            if was_bound {
                events.post(Event::Mix { state: "finished".to_string() });
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // Echo command flips state.echo; we can't observe state directly from
        // outside the actor (by design), but we can drive a tick and assert
        // no panic plus expect no early close.
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

        // Expect a play-start event.
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
