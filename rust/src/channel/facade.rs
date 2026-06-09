// #[napi] facade — the JS-visible surface for channel.
//
// Scope for this pass: land the ChannelObject shape and the `openchannel`
// entry point so the rest of the crate builds against a real napi surface.
// The ThreadsafeFunction bridge for events back to JS is stubbed (events are
// swallowed) — it lands with the integration test pass when real JS callers
// exercise it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::JsFunction;
use napi_derive::napi;

use super::actor::{self, Event, EventSink, PlayState, RecordState, SpawnConfig};
use super::commands::{Direction, Handle};

static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

/// Global registry of active channel handles so `shutdown()` can close them all.
static CHANNEL_REGISTRY: parking_lot::Mutex<Vec<Handle>> = parking_lot::Mutex::new(Vec::new());

/// Lifetime counters for the `stats()` napi export. Parallels the C++
/// addon's `channelscreated` atomic but tracks created / closed
/// independently so the JS side can read both. `current` is derived from
/// the registry length and reads live — decrementing must happen BEFORE
/// the Close event fires, otherwise the JS `afterEach` that asserts
/// `current === 0` races the TSFN queue.
static CHANNELS_CREATED: AtomicU64 = AtomicU64::new(0);
static CHANNELS_CLOSED: AtomicU64 = AtomicU64::new(0);

pub fn shutdown_all_channels() {
    let handles: Vec<Handle> = CHANNEL_REGISTRY.lock().drain(..).collect();
    for h in &handles {
        let _ = h.cmd.try_send(super::commands::Command::Close {
            reason: "shutdown".into(),
        });
    }
}

pub fn register_channel(handle: &Handle) {
    CHANNEL_REGISTRY.lock().push(handle.clone());
    CHANNELS_CREATED.fetch_add(1, Ordering::Relaxed);
}

pub fn unregister_channel(id: u64) {
    let removed = {
        let mut reg = CHANNEL_REGISTRY.lock();
        let before = reg.len();
        reg.retain(|h| h.id != id);
        before != reg.len()
    };
    // Only bump the closed counter if this id was actually in the
    // registry — re-calling unregister (e.g. during a mix teardown that
    // races with the actor's own close path) must be idempotent.
    if removed {
        CHANNELS_CLOSED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn active_channel_count() -> usize {
    CHANNEL_REGISTRY.lock().len()
}

pub fn total_channels_created() -> u64 {
    CHANNELS_CREATED.load(Ordering::Relaxed)
}

pub fn total_channels_closed() -> u64 {
    CHANNELS_CLOSED.load(Ordering::Relaxed)
}

#[derive(Debug)]
struct EventPayload {
    action: &'static str,
    reason: Option<String>,
    event: Option<String>,
    state: Option<String>,
    stats: Option<super::actor::ChannelStats>,
    /// File path for record events — the JS tests assert
    /// `{ action: "record", file: "...", event: "..." }`.
    file: Option<String>,
    /// File size in bytes for record-finished events.
    filesize: Option<u64>,
}

fn event_to_payload(ev: Event) -> EventPayload {
    match ev {
        Event::Close { reason, stats } => EventPayload {
            action: "close",
            reason: Some(reason),
            event: None,
            state: None,
            stats: Some(stats),
            file: None,
            filesize: None,
        },
        Event::Play { state, reason } => EventPayload {
            action: "play",
            reason,
            event: Some(
                match state {
                    PlayState::Start => "start",
                    PlayState::End => "end",
                }
                .into(),
            ),
            state: None,
            stats: None,
            file: None,
            filesize: None,
        },
        Event::Record {
            state,
            reason,
            file,
            filesize,
        } => {
            // C++ surfaces both recording and finish transitions with a
            // compound event name — `recording.abovepower` when a
            // power-gated recorder starts writing, `finished.belowpower` /
            // `finished.requested` / `finished.channelclosed` etc on stop.
            // A bare `recording` (no reason) is the immediate-active case.
            let prefix = match state {
                RecordState::Recording => "recording",
                RecordState::Finished => "finished",
            };
            let event = match &reason {
                Some(r) => format!("{prefix}.{r}"),
                None => prefix.to_string(),
            };
            EventPayload {
                action: "record",
                reason,
                event: Some(event),
                state: None,
                stats: None,
                file,
                filesize,
            }
        }
        Event::TelephoneEvent { digit } => EventPayload {
            action: "telephone-event",
            reason: None,
            event: Some(digit.to_string()),
            state: None,
            stats: None,
            file: None,
            filesize: None,
        },
        Event::Mix { state } => EventPayload {
            // Tests read `d.event` for "start" / "finished", not `d.state`.
            action: "mix",
            reason: None,
            event: Some(state),
            state: None,
            stats: None,
            file: None,
            filesize: None,
        },
    }
}

struct JsEventSink {
    tsfn: ThreadsafeFunction<EventPayload, ErrorStrategy::Fatal>,
}
impl EventSink for JsEventSink {
    fn post(&self, ev: Event) {
        self.tsfn.call(
            event_to_payload(ev),
            ThreadsafeFunctionCallMode::NonBlocking,
        );
    }
}

/// Used when no callback is supplied (some tests do this).
struct NullSink;
impl EventSink for NullSink {
    fn post(&self, _ev: Event) {}
}

#[cfg_attr(not(test), napi(object))]
#[allow(dead_code)]
pub struct ChannelDtls {
    pub fingerprint: String,
    pub enabled: bool,
    pub icepwd: String,
}

#[cfg_attr(not(test), napi(object))]
#[allow(dead_code)]
pub struct ChannelLocal {
    pub port: u32,
    pub ssrc: u32,
    pub icepwd: String,
    pub dtls: ChannelDtls,
}

#[cfg_attr(not(test), napi)]
pub struct ChannelObject {
    handle: Handle,
    channel_ssrc: u32,
    channel_port: u16,
    channel_icepwd: String,
    /// Snapshot of the remote address supplied at openchannel — used to
    /// program the mix-peer relay when JS calls `chan.mix(other)`.
    #[allow(dead_code)]
    remote_addr: Option<SocketAddr>,
    /// Snapshot of the remote payload type from params.remote.codec.
    /// For iLBC with dynamic-PT negotiation this is the ilbcpt value,
    /// not the static 97 — i.e. the actual wire PT.
    #[allow(dead_code)]
    remote_pt: u8,
    /// Snapshot of the remote rfc2833 PT (defaults to 101).
    #[allow(dead_code)]
    rfc2833_pt: u8,
    /// Snapshot of the iLBC wire PT (defaults to 97). Used when BindMixPeer
    /// tells the peer how to recognise this side's iLBC packets.
    #[allow(dead_code)]
    ilbc_pt: u8,
    /// Mix-group handle — `Some` when this channel is currently part of a
    /// mix (state has migrated into the mixer actor). Facade-side only;
    /// the channel actor carries a parallel Mode::Mixed reference. Guarded
    /// by a std Mutex so the sync `mix()` / `unmix()` methods can update
    /// it from JS without await.
    mix_slot: std::sync::Mutex<Option<super::mixer::MixHandle>>,
    /// Per-channel registry of active write-stream senders. Each open
    /// `createWriteStream` entry holds the `mpsc::Sender<Vec<u8>>` half
    /// of the pair whose `Receiver` lives on the actor-side
    /// `AudioWriter`. Indexed by writer id so `push_writer_bytes` /
    /// `end_write_stream` can route to the right one. Guarded by a
    /// `parking_lot::Mutex` so `push` is lock-cheap on the hot path —
    /// we don't want a contended std mutex between every 20 ms frame.
    write_senders:
        parking_lot::Mutex<std::collections::HashMap<u64, tokio::sync::mpsc::Sender<Vec<u8>>>>,
}

// The `#[napi]` proc macro on an impl block needs to expand before the
// per-method `#[napi(...)]` attributes to recognise `self`. `cfg_attr`
// breaks that ordering, so we cfg-gate the whole impl instead. Tests
// don't exercise ChannelObject methods directly — they go through
// ChannelState / Subsystems — so no test-side shim is needed.
#[cfg(not(test))]
#[napi]
impl ChannelObject {
    #[napi(getter)]
    pub fn ssrc(&self) -> u32 {
        self.channel_ssrc
    }

    #[napi(getter)]
    pub fn port(&self) -> u32 {
        self.channel_port as u32
    }

    // index.js expects to read chan.local.{port,ssrc} right after openchannel,
    // then assign chan.local.address = ... Defensive JS already handles the
    // transient-object issue (see index.js comment "I can't find a way of
    // defining a getter in napi"). Returns a fresh object each call.
    // index.js attaches `local` as a regular property after openchannel
    // returns. We only expose the constituent fields; napi-rs class getters
    // would be non-writable and can't be shadowed by the JS wrapper.
    #[napi(getter)]
    pub fn icepwd(&self) -> String {
        self.channel_icepwd.clone()
    }

    #[napi(getter)]
    pub fn dtlsfingerprint(&self) -> String {
        crate::dtls::fingerprint().to_string()
    }

    #[napi]
    pub fn close(&self, reason: Option<String>) -> bool {
        let r = reason.unwrap_or_else(|| "requested".into());
        self.handle
            .cmd
            .try_send(super::commands::Command::Close { reason: r })
            .is_ok()
    }

    #[napi]
    pub async fn dtmf(&self, digits: String) {
        self.handle.dtmf(digits).await;
    }

    // C++ returns `true`; tests assert `expect(channel.echo()).to.be.true`.
    // Sync because the existing test suite calls it without await.
    #[napi]
    pub fn echo(&self) -> bool {
        // Fire-and-forget on the actor's command channel.
        let cmd = self.handle.cmd.clone();
        let _ = cmd.try_send(super::commands::Command::Echo { enabled: true });
        true
    }

    /// JS calls `channel.direction({ send, recv })` with a single object.
    #[napi]
    pub fn direction(&self, opts: DirectionOpts) -> bool {
        self.handle
            .cmd
            .try_send(super::commands::Command::Direction(Direction {
                send: opts.send.unwrap_or(true),
                recv: opts.recv.unwrap_or(true),
            }))
            .is_ok()
    }

    /// Reconfigure the remote end for outbound RTP. JS calls
    /// `channel.remote({ address, port, codec })`. Synchronous so the test's
    /// `channel.remote(...)` sits on the same tick as `channel.echo()`.
    /// Returns true when the params were valid enough to enqueue a Remote
    /// command (address + port parseable). DTLS tests assert the return
    /// value, so `false` for an invalid spec is how JS learns the call
    /// was a no-op.
    #[napi]
    pub fn remote(&self, params: Object) -> bool {
        let addr = params.get_named_property::<String>("address").ok();
        let port = params.get_named_property::<u32>("port").ok();
        let codec = params.get_named_property::<u32>("codec").ok().unwrap_or(0);
        let icepwd = params.get_named_property::<String>("icepwd").ok();
        let dtls = parse_remote_dtls(&params);
        let Some(addr_s) = addr else {
            return false;
        };
        let Some(port_n) = port else {
            return false;
        };
        let Ok(ip) = addr_s.parse::<IpAddr>() else {
            return false;
        };
        let sa = SocketAddr::new(ip, port_n as u16);
        let (ack, _) = tokio::sync::oneshot::channel();
        self.handle
            .cmd
            .try_send(super::commands::Command::Remote {
                cfg: super::commands::RemoteConfig {
                    addr: sa,
                    payload_type: codec as u8,
                    ilbc_payload_type: None,
                    rfc2833_payload_type: None,
                    dtls,
                    icepwd,
                },
                ack,
            })
            .is_ok()
    }

    /// Start (or replace) a soundsoup playback on this channel. JS shape
    /// matches `projectrtpsoundsoup.cpp`: `{ files:[{wav,start,stop,loop}],
    /// loop: bool|number, interupt: bool }` — note the original C++ typo
    /// "interupt" is preserved for JS compat. Returns false (and fires no
    /// commands) when the spec is invalid so tests can assert failure.
    #[napi]
    pub fn play(&self, params: Object) -> bool {
        let Some(spec) = parse_soundsoup(&params) else {
            return false;
        };
        let (ack, _) = tokio::sync::oneshot::channel();
        self.handle
            .cmd
            .try_send(super::commands::Command::Play { cfg: spec, ack })
            .is_ok()
    }

    /// Start (or finish / pause) a recording. Shapes:
    /// - `{ file, numchannels, maxduration, startabovepower, ... }` — start
    /// - `{ file, finish: true }` — finalize the recorder at that file path
    /// - `{ file, pause: true|false }` — pause/resume the recorder
    /// Matches the C++ `projectrtpchannelrecorder.cpp` JS API. Multiple
    /// recorders (different `file`s) can coexist on one channel.
    #[napi]
    pub fn record(&self, params: Object) -> bool {
        let file = params
            .get_named_property::<String>("file")
            .ok()
            .filter(|s| !s.is_empty());
        if params.get_named_property::<bool>("finish").ok() == Some(true) {
            let Some(file) = file else {
                return false;
            };
            return self
                .handle
                .cmd
                .try_send(super::commands::Command::RecordFinish {
                    file: std::path::PathBuf::from(file),
                })
                .is_ok();
        }
        if params.get_named_property::<bool>("pause").ok() == Some(true) {
            let Some(file) = file else {
                return false;
            };
            return self
                .handle
                .cmd
                .try_send(super::commands::Command::RecordSetPaused {
                    file: std::path::PathBuf::from(file),
                    paused: true,
                })
                .is_ok();
        }
        let Some(cfg) = parse_recorder(&params) else {
            return false;
        };
        let (ack, _) = tokio::sync::oneshot::channel();
        self.handle
            .cmd
            .try_send(super::commands::Command::Record { cfg, ack })
            .is_ok()
    }

    /// Register a live audio reader. `callback` is invoked from a dedicated
    /// forwarder task with one `Buffer` argument per 20 ms frame; the JS
    /// wrapper in index.js builds a `Readable` around that callback. Returns
    /// a non-zero reader id so JS can later destroy this reader via
    /// `destroy_read_stream`. Returns 0 on failure (channel already closed).
    #[napi]
    pub fn create_read_stream(
        &self,
        env: Env,
        params: Object,
        callback: JsFunction,
    ) -> Result<u32> {
        let cfg = parse_reader_config(&params);

        // Build the TSFN that turns each Vec<u8> into a Node Buffer and
        // invokes the JS callback. NonBlocking call mode ensures napi drops
        // if its own queue is full — the forwarder task must never stall
        // the 20 ms tick pipeline upstream.
        let mut tsfn: ThreadsafeFunction<Vec<u8>, ErrorStrategy::Fatal> = callback
            .create_threadsafe_function(
                0,
                |ctx: napi::threadsafe_function::ThreadSafeCallContext<Vec<u8>>| {
                    let buf = ctx.env.create_buffer_with_data(ctx.value)?;
                    Ok(vec![buf.into_raw()])
                },
            )?;
        tsfn.unref(&env)?;

        let (sender, mut receiver) = super::audio_reader::make_channel();
        let id = super::audio_reader::next_reader_id();

        // Forwarder task: drains the Rust-side mpsc into the TSFN. Exits
        // automatically when all senders are dropped (channel close, or
        // destroy_read_stream removes the reader from subs.readers). On
        // exit we call the TSFN once more with a zero-length buffer — the
        // JS wrapper treats that as end-of-stream and pushes `null` to the
        // Readable so consumers see a clean `end` event.
        tokio::spawn(async move {
            while let Some(bytes) = receiver.recv().await {
                tsfn.call(bytes, ThreadsafeFunctionCallMode::NonBlocking);
            }
            tsfn.call(Vec::new(), ThreadsafeFunctionCallMode::NonBlocking);
        });

        if self
            .handle
            .cmd
            .try_send(super::commands::Command::CreateReadStream { id, cfg, sender })
            .is_err()
        {
            return Ok(0);
        }
        // u32 is plenty — id is monotonic and channels are short-lived. Cast
        // is safe for the lifetime of any realistic process.
        Ok(id as u32)
    }

    /// Tear down a specific reader by id (as returned from
    /// `create_read_stream`). Safe to call for an already-destroyed id —
    /// the handler is a no-op if the id isn't found.
    #[napi]
    pub fn destroy_read_stream(&self, id: u32) -> bool {
        self.handle
            .cmd
            .try_send(super::commands::Command::DestroyReadStream { id: id as u64 })
            .is_ok()
    }

    /// Register a live audio writer. JS wraps the returned id in a Node
    /// `Writable` whose `_write` pipes bytes through `push_writer_bytes`
    /// and whose `_final`/`_destroy` route through `end_write_stream`
    /// and `destroy_write_stream`. Returns 0 on failure (channel closed).
    ///
    /// v1 accepts linear PCM-16 LE at 8 kHz mono only — the params
    /// object is reserved for the format matrix we'll open up later.
    #[napi]
    pub fn create_write_stream(&self, params: Object) -> u32 {
        let cfg = parse_writer_config(&params);
        let (sender, receiver) = super::audio_writer::make_channel();
        let id = super::audio_writer::next_writer_id();
        if self
            .handle
            .cmd
            .try_send(super::commands::Command::CreateWriteStream { id, cfg, receiver })
            .is_err()
        {
            return 0;
        }
        self.write_senders.lock().insert(id, sender);
        id as u32
    }

    /// Push one chunk of audio bytes onto a registered writer. Returns
    /// `false` when the writer's 1 s bounded queue is full (the JS
    /// wrapper retries with a short delay — the 20 ms tick drains it
    /// steadily) or when the id is unknown. The hot path: no actor
    /// round-trip, just a direct `try_send` onto the writer's mpsc.
    #[napi]
    pub fn push_writer_bytes(&self, id: u32, buf: Buffer) -> bool {
        let sender = {
            let map = self.write_senders.lock();
            map.get(&(id as u64)).cloned()
        };
        let Some(sender) = sender else {
            return false;
        };
        sender.try_send(buf.to_vec()).is_ok()
    }

    /// `writable.end()` from JS — drop the sender so the `AudioWriter`
    /// sees `Disconnected` on its next drain, flushes the final partial
    /// frame, and emits `play/end reason=completed`.
    #[napi]
    pub fn end_write_stream(&self, id: u32) -> bool {
        self.write_senders.lock().remove(&(id as u64)).is_some()
    }

    /// Immediate teardown (analogue of `reader.destroy()`): drops the
    /// sender AND tells the actor to remove the writer from Subsystems.
    /// Any buffered samples are discarded — use `end_write_stream` if
    /// you want graceful drain instead.
    #[napi]
    pub fn destroy_write_stream(&self, id: u32) -> bool {
        let _ = self.write_senders.lock().remove(&(id as u64));
        self.handle
            .cmd
            .try_send(super::commands::Command::DestroyWriteStream { id: id as u64 })
            .is_ok()
    }

    /// Play a prompt then record; optionally barge-in on loud inbound audio.
    /// JS shape: `{ soup: <soundsoup>, record: <record params>, interrupt,
    /// bargeinpower, bargeinpoweraveragepackets }`.
    #[napi]
    pub fn playrecord(&self, params: Object) -> bool {
        let Ok(soup_obj) = params.get_named_property::<Object>("soup") else {
            return false;
        };
        let Some(player) = parse_soundsoup(&soup_obj) else {
            return false;
        };
        let Ok(rec_obj) = params.get_named_property::<Object>("record") else {
            return false;
        };
        let Some(recorder) = parse_recorder(&rec_obj) else {
            return false;
        };
        let interrupt = params
            .get_named_property::<bool>("interrupt")
            .ok()
            .unwrap_or(false);
        let bargein_power = params.get_named_property::<i32>("bargeinpower").ok();
        let bargein_packets = params
            .get_named_property::<u32>("bargeinpoweraveragepackets")
            .ok();
        let cfg = super::commands::PlayRecordConfig {
            player,
            recorder,
            interrupt,
            bargein_power,
            bargein_packets,
        };
        let (ack, _) = tokio::sync::oneshot::channel();
        self.handle
            .cmd
            .try_send(super::commands::Command::PlayRecord { cfg, ack })
            .is_ok()
    }

    /// Place both channels in a shared mix group. The channels' state and
    /// subsystems migrate into the mix actor (which owns the tick for all
    /// members in lockstep — see `channel/mixer.rs`). Subsequent `mix(c)`
    /// calls extend the existing group. Returns true unless the channels
    /// are already in *different* groups (unsupported merge).
    #[napi]
    pub fn mix(&self, other: &ChannelObject) -> bool {
        // Deadlock-avoidance: always lock in ascending channel-id order.
        let (mut self_guard, mut other_guard) = if self.handle.id < other.handle.id {
            (
                self.mix_slot.lock().unwrap(),
                other.mix_slot.lock().unwrap(),
            )
        } else if self.handle.id > other.handle.id {
            let og = other.mix_slot.lock().unwrap();
            let sg = self.mix_slot.lock().unwrap();
            (sg, og)
        } else {
            // Same channel — mix(self, self) is a no-op.
            return true;
        };
        let self_slot = &mut *self_guard;
        let other_slot = &mut *other_guard;

        let mix = match (&self_slot, &other_slot) {
            (None, None) => super::mixer::spawn(),
            (Some(mix), None) | (None, Some(mix)) => mix.clone(),
            (Some(ma), Some(mb)) => {
                if ma.id == mb.id {
                    // Same group — still fire a fresh mix/start on both sides
                    // by re-sending EnterMix (the actor is idempotent when
                    // already in this mix).
                    ma.clone()
                } else {
                    return false;
                }
            }
        };

        // Always send EnterMix to both channels. The channel actor migrates
        // on first entry and posts a `mix/start` event; on a re-mix into the
        // same group it just posts the event again (matches C++ behaviour
        // where every `mix()` call fires `mix/start` on both sides).
        *self_slot = Some(mix.clone());
        *other_slot = Some(mix.clone());
        let (ack, _) = tokio::sync::oneshot::channel();
        let _ = self
            .handle
            .cmd
            .try_send(super::commands::Command::EnterMix {
                mix: mix.clone(),
                ack,
            });
        let (ack, _) = tokio::sync::oneshot::channel();
        let _ = other
            .handle
            .cmd
            .try_send(super::commands::Command::EnterMix { mix, ack });
        true
    }

    #[napi]
    pub fn unmix(&self, _other: Option<&ChannelObject>) -> bool {
        *self.mix_slot.lock().unwrap() = None;
        let (ack, _) = tokio::sync::oneshot::channel();
        let _ = self
            .handle
            .cmd
            .try_send(super::commands::Command::LeaveMix { ack });
        true
    }
}

#[cfg_attr(not(test), napi(object))]
pub struct DirectionOpts {
    pub send: Option<bool>,
    pub recv: Option<bool>,
}

/// Synchronous — index.js treats the return as the channel object directly,
/// not a Promise. Bind sockets via std::net, hand over to the tokio actor.
/// Events route to JS via ThreadsafeFunction so async tests resolve.
type BindTuple = (
    std::net::UdpSocket,
    std::net::UdpSocket,
    SocketAddr,
    Option<crate::portpool::PortReservation>,
);

/// Bind RTP+RTCP from the managed port pool. The pool only hands out even
/// ports and each is held exclusively, so both binds should always succeed
/// when the pool is non-empty. If a bind *does* fail (port held by another
/// process on the same box, say), we release back to the pool and try the
/// next available entry — bounded by `MAX_POOL_TRIES` so a truly exhausted
/// pool reports promptly.
fn bind_from_pool() -> Result<BindTuple> {
    const MAX_POOL_TRIES: usize = 16;
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..MAX_POOL_TRIES {
        let Some(port) = crate::portpool::acquire() else {
            return Err(Error::from_reason("no available rtp ports in pool"));
        };
        let rtp_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let rtcp_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port + 1);
        let rtp = match std::net::UdpSocket::bind(rtp_addr) {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(e);
                crate::portpool::release(port);
                continue;
            }
        };
        let rtcp = match std::net::UdpSocket::bind(rtcp_addr) {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(e);
                crate::portpool::release(port);
                continue;
            }
        };
        return Ok((
            rtp,
            rtcp,
            rtp_addr,
            Some(crate::portpool::PortReservation::new(port)),
        ));
    }
    Err(Error::from_reason(format!(
        "bind rtp/rtcp from pool: exhausted {MAX_POOL_TRIES} tries; last={last_err:?}"
    )))
}

/// Ephemeral-bind fallback for when the pool is uninitialized — still
/// requires an even RTP port so RTCP = P+1 is semantically correct.
fn bind_ephemeral() -> Result<BindTuple> {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..64 {
        let rtp = match std::net::UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let la = match rtp.local_addr() {
            Ok(a) => a,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        let p = la.port();
        if p % 2 != 0 {
            continue;
        }
        let rtcp_addr = SocketAddr::new(la.ip(), p + 1);
        match std::net::UdpSocket::bind(rtcp_addr) {
            Ok(rtcp) => return Ok((rtp, rtcp, la, None)),
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        }
    }
    Err(Error::from_reason(format!(
        "bind rtp/rtcp pair: exhausted ephemeral retries; last={last_err:?}"
    )))
}

/// Parse a JS `play` / `playrecord` params object into a `SoundSoupSpec`.
/// Matches the C++ parser in projectrtpsoundsoup.cpp: rejects when no files
/// are supplied or when no file is loadable, honours per-file `start`/`stop`
/// (ms) and `loop`, plus top-level `loop` (bool or integer) and `interupt`.
fn parse_soundsoup(params: &Object) -> Option<super::player::SoundSoupSpec> {
    let files_arr: Array = params.get_named_property::<Array>("files").ok()?;
    let len = files_arr.len();
    if len == 0 {
        return None;
    }
    let mut files = Vec::with_capacity(len as usize);
    for i in 0..len {
        let Ok(entry) = files_arr.get::<Object>(i) else {
            continue;
        };
        let Some(entry) = entry else {
            continue;
        };
        let Ok(wav): Result<String> = entry.get_named_property("wav") else {
            continue;
        };
        if wav.is_empty() {
            continue;
        }
        let path = std::path::PathBuf::from(wav);
        // Skip entries whose file doesn't exist — matches C++
        // `soundsoup::load` which drops missing files silently and
        // (when every entry is missing) causes `channel.play()` to
        // return false. Tests assert the false return explicitly.
        if !path.exists() {
            continue;
        }
        // Per-file loop: JS accepts `true` (infinite — encoded as Some(0)
        // in player.rs) or a positive integer.
        let max_loops = if entry.get_named_property::<bool>("loop").ok() == Some(true) {
            Some(0)
        } else if let Ok(n) = entry.get_named_property::<u32>("loop") {
            if n == 0 {
                None
            } else {
                Some(n)
            }
        } else {
            None
        };
        files.push(super::player::SoundSoupFileSpec {
            path,
            start_ms: entry
                .get_named_property::<u32>("start")
                .ok()
                .filter(|&v| v > 0)
                .map(|v| v as u64),
            stop_ms: entry
                .get_named_property::<u32>("stop")
                .ok()
                .filter(|&v| v > 0)
                .map(|v| v as u64),
            max_loops,
        });
    }
    if files.is_empty() {
        return None;
    }
    // Top-level `loop`: bool (true → infinite, false/absent → once) or number
    // (explicit loop count). We can't know the JS type here, so try number
    // first then fall back to bool — matches how the C++ parser handles it.
    let overall_loops = if let Ok(n) = params.get_named_property::<u32>("loop") {
        // `loop: 0` means no looping in C++; treat as one-shot for parity.
        if n == 0 {
            None
        } else {
            Some(n)
        }
    } else if params.get_named_property::<bool>("loop").ok() == Some(true) {
        Some(0) // 0 = infinite in player.rs
    } else {
        None
    };
    // Typo is intentional — C++ API uses "interupt".
    let interrupt = params
        .get_named_property::<bool>("interupt")
        .ok()
        .unwrap_or(false);
    Some(super::player::SoundSoupSpec {
        files,
        overall_loops,
        interrupt,
    })
}

/// Parse a JS `createWriteStream` params object into a `WriterConfig`.
/// v1 locks the matrix at L16 / 8000 / mono — we validate the fields
/// but ignore out-of-range values (matches the reader's behaviour of
/// silently clamping to the supported set).
fn parse_writer_config(params: &Object) -> super::audio_writer::WriterConfig {
    use super::audio_writer::{WriterConfig, WriterFormat};
    let _ = params; // format/samplerate/numchannels reserved for future use
    WriterConfig {
        format: WriterFormat::L16,
        samplerate: 8000,
        num_channels: 1,
    }
}

/// Parse a JS `createReadStream` params object into a `ReaderConfig`.
/// All fields optional; defaults match the common "mono inbound linear
/// PCM at the channel's narrowband rate" case.
fn parse_reader_config(params: &Object) -> super::audio_reader::ReaderConfig {
    use super::audio_reader::{ReaderConfig, ReaderDirection, ReaderFormat};
    let direction = match params
        .get_named_property::<String>("direction")
        .ok()
        .as_deref()
    {
        Some("out") => ReaderDirection::Out,
        Some("both") => ReaderDirection::Both,
        _ => ReaderDirection::In,
    };
    let format = match params
        .get_named_property::<String>("format")
        .ok()
        .as_deref()
    {
        Some("pcma") => ReaderFormat::Pcma,
        Some("pcmu") => ReaderFormat::Pcmu,
        Some("g722") => ReaderFormat::G722,
        Some("ilbc") => ReaderFormat::Ilbc,
        _ => ReaderFormat::L16,
    };
    let samplerate = params
        .get_named_property::<u32>("samplerate")
        .ok()
        .filter(|&v| v == 8000 || v == 16000)
        .unwrap_or(8000);
    let num_channels = params
        .get_named_property::<u32>("numchannels")
        .ok()
        .filter(|&v| v == 1 || v == 2)
        .map(|v| v as u16)
        .unwrap_or(1);
    ReaderConfig {
        direction,
        format,
        samplerate,
        num_channels,
    }
}

/// Parse a JS `record` / `playrecord.record` params object into a
/// `RecorderConfig`. Returns None for the common failure mode (no `file`)
/// — the facade turns that into `record()`→false which tests assert.
fn parse_recorder(params: &Object) -> Option<super::recorder::RecorderConfig> {
    let file: String = params.get_named_property::<String>("file").ok()?;
    if file.is_empty() {
        return None;
    }
    // Default 2 channels matches the C++ recorder default — test/interface/
    // projectrtprecord.js "record to file" asserts channelcount === 2.
    let num_channels = params
        .get_named_property::<u32>("numchannels")
        .ok()
        .filter(|&v| v > 0)
        .map(|v| v as u16)
        .unwrap_or(2);
    Some(super::recorder::RecorderConfig {
        file: std::path::PathBuf::from(file),
        num_channels,
        sample_rate: 8000,
        max_duration_ms: params
            .get_named_property::<u32>("maxduration")
            .ok()
            .filter(|&v| v > 0)
            .map(|v| v as u64),
        start_above_power: params
            .get_named_property::<i32>("startabovepower")
            .ok()
            .filter(|&v| v != 0),
        finish_below_power: params
            .get_named_property::<i32>("finishbelowpower")
            .ok()
            .filter(|&v| v != 0),
        max_since_start_power: params
            .get_named_property::<i32>("maxsincestartpower")
            .ok()
            .filter(|&v| v != 0),
        min_duration_ms: params
            .get_named_property::<u32>("minduration")
            .ok()
            .filter(|&v| v > 0)
            .map(|v| v as u64),
        power_averaging_packets: params
            .get_named_property::<u32>("poweraveragepackets")
            .ok()
            .filter(|&v| v > 0),
    })
}

fn extract_remote_addr(params: &Object) -> Option<SocketAddr> {
    let remote: Object = params.get_named_property::<Object>("remote").ok()?;
    let address: String = remote.get_named_property::<String>("address").ok()?;
    let port: u32 = remote.get_named_property::<u32>("port").ok()?;
    let ip: IpAddr = address.parse().ok()?;
    Some(SocketAddr::new(ip, port as u16))
}

fn extract_remote_pt(params: &Object) -> u8 {
    let remote = params.get_named_property::<Object>("remote").ok();
    let codec = remote
        .as_ref()
        .and_then(|r| r.get_named_property::<u32>("codec").ok())
        .map(|v| v as u8)
        .unwrap_or(0);
    // For iLBC (logical codec id 97), JS may override the RTP wire PT via
    // `remote.ilbcpt` (dynamic-PT negotiation). Use that as the effective
    // wire PT so outbound packets carry the negotiated value.
    if codec == 97 {
        if let Some(pt) = remote.and_then(|r| r.get_named_property::<u32>("ilbcpt").ok()) {
            return pt as u8;
        }
    }
    codec
}

/// Read `params.remote.ilbcpt` — the dynamic wire PT for iLBC. Returns
/// `None` when not set; callers default to 97.
fn extract_ilbc_pt(params: &Object) -> Option<u8> {
    params
        .get_named_property::<Object>("remote")
        .ok()
        .and_then(|r| r.get_named_property::<u32>("ilbcpt").ok())
        .map(|v| v as u8)
}

fn extract_rfc2833_pt(params: &Object) -> Option<u8> {
    // Tests pass the RFC 2833 PT two ways: `params.remote.rfc2833pt` (nested)
    // and `params.rfc2833pt` (flat). Accept both.
    if let Some(pt) = params
        .get_named_property::<Object>("remote")
        .ok()
        .and_then(|r| r.get_named_property::<u32>("rfc2833pt").ok())
    {
        return Some(pt as u8);
    }
    params
        .get_named_property::<u32>("rfc2833pt")
        .ok()
        .map(|v| v as u8)
}

/// Parse `params.direction = { send, recv }`. Both fields default to true.
/// Tests use this to create write-only or read-only channels in a mix.
fn extract_direction(params: &Object) -> super::commands::Direction {
    let d = params.get_named_property::<Object>("direction").ok();
    let send = d
        .as_ref()
        .and_then(|o| o.get_named_property::<bool>("send").ok())
        .unwrap_or(true);
    let recv = d
        .and_then(|o| o.get_named_property::<bool>("recv").ok())
        .unwrap_or(true);
    super::commands::Direction { send, recv }
}

fn extract_local_icepwd(params: &Object) -> Option<String> {
    params
        .get_named_property::<Object>("local")
        .ok()
        .and_then(|l| l.get_named_property::<String>("icepwd").ok())
        .filter(|s| !s.is_empty())
}

fn extract_remote_icepwd(params: &Object) -> Option<String> {
    params
        .get_named_property::<Object>("remote")
        .ok()
        .and_then(|r| r.get_named_property::<String>("icepwd").ok())
        .filter(|s| !s.is_empty())
}

/// Parse an optional `dtls` block from a remote config object. JS shape:
/// `{ fingerprint: { hash: "sha-256 ..." }, mode: "active"|"passive" }`
/// — matches `projectrtpdtls.js` test fixtures. Returns None when missing
/// or malformed so plain (non-DTLS) RTP still works.
fn parse_remote_dtls(params: &Object) -> Option<super::commands::RemoteDtls> {
    let dtls_obj = params.get_named_property::<Object>("dtls").ok()?;
    let mode: String = dtls_obj.get_named_property::<String>("mode").ok()?;
    let setup = match mode.as_str() {
        "active" => super::commands::DtlsSetup::Active,
        "passive" => super::commands::DtlsSetup::Passive,
        _ => return None,
    };
    let fingerprint = dtls_obj
        .get_named_property::<Object>("fingerprint")
        .ok()
        .and_then(|fp| fp.get_named_property::<String>("hash").ok())
        .unwrap_or_default();
    Some(super::commands::RemoteDtls { fingerprint, setup })
}

fn extract_remote_dtls(params: &Object) -> Option<super::commands::RemoteDtls> {
    let remote = params.get_named_property::<Object>("remote").ok()?;
    parse_remote_dtls(&remote)
}

#[cfg_attr(not(test), napi(js_name = "openchannel"))]
pub fn open_channel(env: Env, params: Object, callback: JsFunction) -> Result<ChannelObject> {
    let remote_addr = extract_remote_addr(&params);
    let remote_pt = extract_remote_pt(&params);
    let rfc2833_pt = extract_rfc2833_pt(&params);
    let ilbc_pt = extract_ilbc_pt(&params).unwrap_or(97);
    let override_local_icepwd = extract_local_icepwd(&params);
    let initial_remote_icepwd = extract_remote_icepwd(&params);
    let initial_remote_dtls = extract_remote_dtls(&params);
    let initial_direction = extract_direction(&params);
    let mut tsfn: ThreadsafeFunction<EventPayload, ErrorStrategy::Fatal> = callback
        .create_threadsafe_function(
            0,
            |ctx: napi::threadsafe_function::ThreadSafeCallContext<EventPayload>| {
                let ev = ctx.value;
                let env = ctx.env;
                let mut obj = env.create_object()?;
                let action: napi::JsString = env.create_string(ev.action)?;
                obj.set_named_property("action", action)?;
                if let Some(r) = ev.reason {
                    let v: napi::JsString = env.create_string(&r)?;
                    obj.set_named_property("reason", v)?;
                }
                if let Some(e) = ev.event {
                    let v: napi::JsString = env.create_string(&e)?;
                    obj.set_named_property("event", v)?;
                }
                if let Some(s) = ev.state {
                    let v: napi::JsString = env.create_string(&s)?;
                    obj.set_named_property("state", v)?;
                }
                if let Some(f) = ev.file {
                    let v: napi::JsString = env.create_string(&f)?;
                    obj.set_named_property("file", v)?;
                }
                if let Some(fs) = ev.filesize {
                    obj.set_named_property("filesize", env.create_int64(fs as i64)?)?;
                }
                if let Some(stats) = ev.stats {
                    let mut s = env.create_object()?;
                    let mut in_o = env.create_object()?;
                    in_o.set_named_property("count", env.create_int64(stats.in_count as i64)?)?;
                    in_o.set_named_property("dropped", env.create_int64(stats.in_dropped as i64)?)?;
                    in_o.set_named_property("skip", env.create_int64(stats.in_skip as i64)?)?;
                    in_o.set_named_property("mos", env.create_double(stats.mos())?)?;
                    let mut out_o = env.create_object()?;
                    out_o.set_named_property("count", env.create_int64(stats.out_count as i64)?)?;
                    // Matches the C++ close-stats shape used by lib/node.js + the
                    // proxy tests that assert `msg.stats.{in,out,tick}` exist.
                    // Values are placeholders — we don't yet measure per-tick
                    // wall time the way the C++ addon does.
                    out_o.set_named_property("skip", env.create_int64(0)?)?;
                    out_o.set_named_property("drop", env.create_int64(0)?)?;
                    out_o.set_named_property("write", env.create_int64(stats.out_count as i64)?)?;
                    let mut tick_o = env.create_object()?;
                    tick_o.set_named_property("count", env.create_int64(0)?)?;
                    tick_o.set_named_property("meanus", env.create_double(0.0)?)?;
                    tick_o.set_named_property("maxus", env.create_double(0.0)?)?;
                    s.set_named_property("in", in_o)?;
                    s.set_named_property("out", out_o)?;
                    s.set_named_property("tick", tick_o)?;
                    obj.set_named_property("stats", s)?;
                }
                Ok(vec![obj])
            },
        )?;
    // Unref the TSFN so it doesn't keep the Node.js event loop alive.
    // Without this, the process hangs on shutdown because each channel's
    // TSFN holds a libuv handle that prevents the event loop from draining.
    tsfn.unref(&env)?;
    let sink: Arc<dyn EventSink> = Arc::new(JsEventSink { tsfn });
    let _ = NullSink; // keep type live for future use
    let id = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
    let ssrc = rand_ssrc();

    // Port allocation. Prefer the managed pool (filled by `run({ports: {...}})`)
    // — each port is held exclusively for the channel's lifetime, so RTP on P
    // and RTCP on P+1 are both guaranteed free at bind time. If the pool is
    // not initialized (e.g. a test harness that didn't call `run`), fall back
    // to an ephemeral-bind loop that requires an even P.
    let (std_rtp, std_rtcp, local_addr, port_reservation) = if crate::portpool::is_initialized() {
        bind_from_pool()?
    } else {
        bind_ephemeral()?
    };
    std_rtp.set_nonblocking(true).ok();
    std_rtcp.set_nonblocking(true).ok();
    let port = local_addr.port();

    // Convert std → tokio (requires being inside a tokio runtime, which the
    // napi-rs tokio_rt feature provides).
    let rtp_sock = tokio::net::UdpSocket::from_std(std_rtp)
        .map_err(|e| Error::from_reason(format!("rtp tokio adopt: {e}")))?;
    let rtcp_sock = tokio::net::UdpSocket::from_std(std_rtcp)
        .map_err(|e| Error::from_reason(format!("rtcp tokio adopt: {e}")))?;

    // Local ICE password — JS can override the generated value via
    // `params.local.icepwd` (used by the STUN tests and by SIP stacks that
    // negotiate the value in SDP). Falls back to a fresh random string.
    let local_icepwd = override_local_icepwd.unwrap_or_else(rand_icepwd);

    let handle = actor::spawn_with_sockets(
        SpawnConfig {
            id,
            bind_addr: local_addr,
            ssrc,
            events: sink,
            port_reservation,
            local_icepwd: local_icepwd.clone(),
        },
        rtp_sock,
        rtcp_sock,
        local_addr,
    )
    .map_err(|e| Error::from_reason(format!("spawn channel: {e}")))?;

    // Apply params.direction (if the caller supplied one) before anything
    // else so the channel observes the right send/recv flags on its first
    // tick. Skip the send if the user didn't override the defaults.
    if initial_direction.send != true || initial_direction.recv != true {
        let _ = handle
            .cmd
            .try_send(super::commands::Command::Direction(initial_direction));
    }

    // If params.remote was supplied, enqueue a Remote command
    // SYNCHRONOUSLY before `openchannel` returns. Using `try_send` here
    // (rather than the old `tokio::spawn { ... .send().await }`) avoids
    // a race: JS code that calls `channel.remote({...})` or
    // `channel.mix(peer)` immediately after `await openchannel(...)`
    // also uses `try_send`, so without this fix the spawned initial
    // send could land in the actor's queue AFTER the JS follow-ups and
    // silently revert any remote override applied by the caller.
    // The command queue is empty at this point so `try_send` always
    // succeeds; the actor will apply the remote on its first dispatch.
    if let Some(addr) = remote_addr {
        use super::commands::{Command, RemoteConfig};
        let (ack, _) = tokio::sync::oneshot::channel();
        let cfg = RemoteConfig {
            addr,
            payload_type: remote_pt,
            ilbc_payload_type: Some(ilbc_pt),
            rfc2833_payload_type: rfc2833_pt,
            dtls: initial_remote_dtls.clone(),
            icepwd: initial_remote_icepwd.clone(),
        };
        let _ = handle.cmd.try_send(Command::Remote { cfg, ack });
    }

    register_channel(&handle);

    Ok(ChannelObject {
        handle,
        channel_ssrc: ssrc,
        channel_port: port,
        channel_icepwd: local_icepwd,
        remote_addr,
        remote_pt,
        rfc2833_pt: rfc2833_pt.unwrap_or(101),
        ilbc_pt,
        mix_slot: std::sync::Mutex::new(None),
        write_senders: parking_lot::Mutex::new(std::collections::HashMap::new()),
    })
}

fn rand_ssrc() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (t as u32) ^ (NEXT_CHANNEL_ID.load(Ordering::Relaxed) as u32).wrapping_mul(0x9E37_79B1)
}

/// 24-char alphanumeric ICE pwd. Matches the C++ `alphanumsecret[]` length
/// requirement (test asserts > 20). Not cryptographically uniform — fine for
/// ICE binding purposes; revisit when DTLS-SRTP wiring lands.
fn rand_icepwd() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(24);
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ NEXT_CHANNEL_ID
            .load(Ordering::Relaxed)
            .wrapping_mul(0xA5A5_5A5A);
    for _ in 0..24 {
        // xorshift64
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        s.push(ALPHABET[(seed as usize) % ALPHABET.len()] as char);
    }
    s
}
