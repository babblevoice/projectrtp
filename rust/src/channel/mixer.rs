// Mix group actor — port of projectrtpchannelmux.{cpp,h}.
//
// A mix group owns N channels for the duration of their mix. When a channel
// joins, its `ChannelState` and `Subsystems` move into this actor's map.
// When it leaves (unmix or close), they move back out via a oneshot.
//
// Per-tick flow (C++ mux::handletick, channelmux.cpp:260):
//   1. For each member: drain inbound socket, decode to i16 samples.
//   2. Build `summed[]` = Σ over members with direction.recv.
//   3. For each member with direction.send: out = summed - self, encode,
//      SRTP-protect, UDP send. Mirror of C++ mix2() / mixall().
//   4. Write each member's input samples to its active recorder.
//   5. Drain any completed DTMF events.
//
// The actual mix math (steps 1–3) depends on decode/encode buffer helpers in
// codec.rs (currently single-sample only) and on tick.rs actually decoding
// inbound packets. Those land in a follow-up pass; this module commits the
// actor shape, command wiring, and migration of state in/out.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};

use super::actor::{Event, EventSink, Subsystems, TICK_MS};
use super::commands::{ChannelId, Command};
use super::state::ChannelState;

// ---------- N-way mix shared state ----------
//
// Channel ticks deposit their latest decoded frame, then read out
// `summed - own` to encode and send. Holding a parking_lot Mutex for the
// few microseconds of a tick keeps cross-actor coordination cheap. The
// 2-channel byte relay is left in place as a fast path; the group is used
// only for 3+ members or when codecs differ in a way the relay can't.

pub const MIX_FRAME_SAMPLES: usize = 160; // 20 ms @ 8 kHz, matches G.711 ptime.

#[derive(Default)]
pub struct MixMember {
    pub frame: Vec<i16>,
    pub recv: bool,
    /// Group version at the moment this member last deposited a frame.
    pub last_deposit_version: u64,
}

pub struct MixGroupShared {
    pub members: Vec<MixMember>,
    /// Bumped each time any member deposits a new frame. Each channel tracks
    /// the version it last emitted on; if `version > last_emitted`, send.
    pub version: u64,
}

impl MixGroupShared {
    pub fn new(n: usize) -> Self {
        let members = (0..n)
            .map(|_| MixMember {
                frame: vec![0; MIX_FRAME_SAMPLES],
                recv: true,
                last_deposit_version: 0,
            })
            .collect();
        Self { members, version: 0 }
    }

    /// Highest `last_deposit_version` across members other than `own`.
    /// Used by tick to decide whether to emit (someone else has new audio
    /// since our last emit).
    pub fn max_other_deposit(&self, own: usize) -> u64 {
        self.members
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != own)
            .map(|(_, m)| m.last_deposit_version)
            .max()
            .unwrap_or(0)
    }

    /// Saturating sum of all `recv`-enabled members' frames.
    /// Caller passes `excluding` to drop the self-contribution in one pass.
    pub fn summed_minus(&self, excluding: usize) -> Vec<i16> {
        let mut out = vec![0i32; MIX_FRAME_SAMPLES];
        for (i, m) in self.members.iter().enumerate() {
            if i == excluding { continue; }
            if !m.recv { continue; }
            // Add into i32 accumulator; saturate at the end. Matches the
            // C++ approach (sum-then-clamp gives better headroom than
            // saturating-each-add).
            for (slot, &s) in out.iter_mut().zip(m.frame.iter()) {
                *slot += s as i32;
            }
        }
        out.iter().map(|&v| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16).collect()
    }
}

pub type MixGroup = Arc<Mutex<MixGroupShared>>;

pub struct Member {
    pub state: ChannelState,
    pub subs: Subsystems,
}

pub enum MixerCommand {
    /// Add a channel to the group. Its state has already moved here.
    Add { id: ChannelId, member: Box<Member>, ack: oneshot::Sender<()> },

    /// Remove a channel and hand its state back.
    Remove { id: ChannelId, ack: oneshot::Sender<Option<Box<Member>>> },

    /// Forward a per-channel command (play, dtmf, echo, direction, ...) for
    /// application to that channel's state while it's inside the mix.
    ForwardCommand { id: ChannelId, cmd: Command },

    /// Gracefully stop the mix group — members are left where they are; the
    /// owning channel actors will get their states back via a separate
    /// `Remove` before this fires.
    Stop,
}

#[derive(Clone)]
pub struct MixHandle {
    pub cmd: mpsc::Sender<MixerCommand>,
}

impl MixHandle {
    pub async fn add(&self, id: ChannelId, member: Box<Member>) {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd.send(MixerCommand::Add { id, member, ack: tx }).await;
        let _ = rx.await;
    }

    pub async fn remove(&self, id: ChannelId) -> Option<Box<Member>> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(MixerCommand::Remove { id, ack: tx }).await.ok()?;
        rx.await.ok().flatten()
    }

    pub async fn forward(&self, id: ChannelId, cmd: Command) {
        let _ = self.cmd.send(MixerCommand::ForwardCommand { id, cmd }).await;
    }

    pub async fn stop(&self) {
        let _ = self.cmd.send(MixerCommand::Stop).await;
    }
}

pub fn spawn(events: Arc<dyn EventSink>) -> MixHandle {
    let (tx, rx) = mpsc::channel(32);
    tokio::spawn(run(rx, events));
    MixHandle { cmd: tx }
}

async fn run(mut cmds: mpsc::Receiver<MixerCommand>, events: Arc<dyn EventSink>) {
    let mut members: HashMap<ChannelId, Member> = HashMap::new();
    let mut ticker = interval(Duration::from_millis(TICK_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stopped = false;

    while !stopped {
        tokio::select! {
            biased;

            cmd = cmds.recv() => match cmd {
                None => stopped = true,
                Some(MixerCommand::Stop) => stopped = true,
                Some(MixerCommand::Add { id, member, ack }) => {
                    members.insert(id, *member);
                    events.post(Event::Mix { state: "start".to_string() });
                    let _ = ack.send(());
                }
                Some(MixerCommand::Remove { id, ack }) => {
                    let m = members.remove(&id).map(Box::new);
                    if members.len() < 2 {
                        events.post(Event::Mix { state: "finished".to_string() });
                    }
                    let _ = ack.send(m);
                    if members.is_empty() { stopped = true; }
                }
                Some(MixerCommand::ForwardCommand { id, cmd }) => {
                    if let Some(m) = members.get_mut(&id) {
                        apply_forwarded(m, cmd);
                    }
                }
            },

            _ = ticker.tick() => {
                if let Err(()) = mix_tick(&mut members).await {
                    stopped = true;
                }
            }
        }
    }
}

/// Mix-tick. Placeholder that advances each member's tick counter and drains
/// their inbound sockets into jitter. The actual per-tick sum / subtract /
/// encode pipeline lands when codec.rs gains buffer-level encode/decode and
/// tick.rs decodes jitter output — see the module docstring.
async fn mix_tick(members: &mut HashMap<ChannelId, Member>) -> Result<(), ()> {
    for (_id, m) in members.iter_mut() {
        m.state.tick_count += 1;
        // TODO: drain inbound, decode, sum.
    }
    // TODO: for each member with direction.send, compute (summed - self),
    // encode, send.
    Ok(())
}

/// Apply a command that targeted a channel currently inside this mix group.
fn apply_forwarded(m: &mut Member, cmd: Command) {
    match cmd {
        Command::Direction(d) => m.state.direction = d,
        Command::Echo { enabled } => m.state.echo = enabled,
        Command::Dtmf { digits } => m.subs.dtmf_send.enqueue(&digits),
        // Remote / Play / Record / PlayRecord / Close / Mix / Unmix are
        // complex enough to need dedicated handling. Left as TODO — the
        // forwarder in actor.rs will grow to dispatch these explicitly as
        // the mixer impl fills in.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::actor::EventSink;
    use std::sync::Mutex;

    struct Capture(Mutex<Vec<Event>>);
    impl EventSink for Capture {
        fn post(&self, ev: Event) { self.0.lock().unwrap().push(ev); }
    }

    #[tokio::test]
    async fn add_and_remove_round_trip() {
        let sink = Arc::new(Capture(Mutex::new(Vec::new())));
        let mix = spawn(sink.clone());

        // Build two dummy members with bound sockets.
        let make_member = |id: ChannelId| async move {
            use tokio::net::UdpSocket;
            let rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let rtcp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let addr = rtp.local_addr().unwrap();
            let state = ChannelState::new(id, addr, rtp, rtcp, 1);
            Box::new(Member { state, subs: Subsystems::default() })
        };

        let a = make_member(1).await;
        let b = make_member(2).await;
        mix.add(1, a).await;
        mix.add(2, b).await;

        // Mixer should have emitted "start" on the second add (members.len >= 2).
        {
            let events = sink.0.lock().unwrap();
            assert!(events.iter().any(|e| matches!(e, Event::Mix { state } if state == "start")));
        }

        let removed = mix.remove(1).await.expect("removed");
        assert_eq!(removed.state.id, 1);

        mix.stop().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn forwarded_direction_command_applies() {
        let sink = Arc::new(Capture(Mutex::new(Vec::new())));
        let mix = spawn(sink.clone());

        use tokio::net::UdpSocket;
        let rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rtcp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = rtp.local_addr().unwrap();
        let state = ChannelState::new(7, addr, rtp, rtcp, 1);
        let member = Box::new(Member { state, subs: Subsystems::default() });
        mix.add(7, member).await;

        mix.forward(7, Command::Direction(
            crate::channel::commands::Direction { send: false, recv: false }
        )).await;

        // Retrieve to assert direction applied.
        let got = mix.remove(7).await.expect("should be present");
        assert!(!got.state.direction.send);
        assert!(!got.state.direction.recv);

        mix.stop().await;
    }
}
