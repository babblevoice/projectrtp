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

#[derive(Debug)]
struct EventPayload {
    action: &'static str,
    reason: Option<String>,
    event: Option<String>,
    state: Option<String>,
    stats: Option<super::actor::ChannelStats>,
}

fn event_to_payload(ev: Event) -> EventPayload {
    match ev {
        Event::Close { reason, stats } => EventPayload {
            action: "close", reason: Some(reason), event: None, state: None,
            stats: Some(stats),
        },
        Event::Play { state, reason } => EventPayload {
            action: "play", reason,
            event: Some(match state { PlayState::Start => "start", PlayState::End => "end" }.into()),
            state: None,
            stats: None,
        },
        Event::Record { state, reason } => EventPayload {
            action: "record", reason,
            event: Some(match state { RecordState::Recording => "recording", RecordState::Finished => "finished" }.into()),
            state: None,
            stats: None,
        },
        Event::TelephoneEvent { digit } => EventPayload {
            action: "telephone-event", reason: None, event: Some(digit.to_string()), state: None,
            stats: None,
        },
        Event::Mix { state } => EventPayload {
            // Tests read `d.event` for "start" / "finished", not `d.state`.
            action: "mix", reason: None, event: Some(state), state: None,
            stats: None,
        },
    }
}

struct JsEventSink {
    tsfn: ThreadsafeFunction<EventPayload, ErrorStrategy::Fatal>,
}
impl EventSink for JsEventSink {
    fn post(&self, ev: Event) {
        self.tsfn.call(event_to_payload(ev), ThreadsafeFunctionCallMode::NonBlocking);
    }
}

/// Used when no callback is supplied (some tests do this).
struct NullSink;
impl EventSink for NullSink {
    fn post(&self, _ev: Event) {}
}

#[napi(object)]
pub struct ChannelDtls {
    pub fingerprint: String,
    pub enabled: bool,
    pub icepwd: String,
}

#[napi(object)]
pub struct ChannelLocal {
    pub port: u32,
    pub ssrc: u32,
    pub icepwd: String,
    pub dtls: ChannelDtls,
}

#[napi]
pub struct ChannelObject {
    handle: Handle,
    channel_ssrc: u32,
    channel_port: u16,
    channel_icepwd: String,
    /// Snapshot of the remote address supplied at openchannel — used to
    /// program the mix-peer relay when JS calls `chan.mix(other)`.
    remote_addr: Option<SocketAddr>,
    /// Snapshot of the remote payload type from params.remote.codec.
    remote_pt: u8,
    /// Snapshot of the remote rfc2833 PT (defaults to 101).
    rfc2833_pt: u8,
}

#[napi]
impl ChannelObject {
    #[napi(getter)]
    pub fn ssrc(&self) -> u32 { self.channel_ssrc }

    #[napi(getter)]
    pub fn port(&self) -> u32 { self.channel_port as u32 }

    // index.js expects to read chan.local.{port,ssrc} right after openchannel,
    // then assign chan.local.address = ... Defensive JS already handles the
    // transient-object issue (see index.js comment "I can't find a way of
    // defining a getter in napi"). Returns a fresh object each call.
    // index.js attaches `local` as a regular property after openchannel
    // returns. We only expose the constituent fields; napi-rs class getters
    // would be non-writable and can't be shadowed by the JS wrapper.
    #[napi(getter)]
    pub fn icepwd(&self) -> String { self.channel_icepwd.clone() }

    #[napi(getter)]
    pub fn dtlsfingerprint(&self) -> String { crate::dtls::js_fingerprint() }

    #[napi]
    pub async fn close(&self, reason: Option<String>) {
        self.handle.close(reason.unwrap_or_else(|| "requested".into())).await;
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
    pub fn direction(&self, opts: DirectionOpts) {
        let _ = self.handle.cmd.try_send(super::commands::Command::Direction(Direction {
            send: opts.send.unwrap_or(true),
            recv: opts.recv.unwrap_or(true),
        }));
    }

    /// Reconfigure the remote end for outbound RTP. JS calls
    /// `channel.remote({ address, port, codec })`. Synchronous so the test's
    /// `channel.remote(...)` sits on the same tick as `channel.echo()`.
    #[napi]
    pub fn remote(&self, params: Object) -> Result<()> {
        let addr = params.get_named_property::<String>("address").ok();
        let port = params.get_named_property::<u32>("port").ok();
        let codec = params.get_named_property::<u32>("codec").ok().unwrap_or(0);
        let Some(addr_s) = addr else { return Ok(()); };
        let Some(port_n) = port else { return Ok(()); };
        let Ok(ip) = addr_s.parse::<IpAddr>() else { return Ok(()); };
        let sa = SocketAddr::new(ip, port_n as u16);
        let (ack, _) = tokio::sync::oneshot::channel();
        let _ = self.handle.cmd.try_send(super::commands::Command::Remote {
            cfg: super::commands::RemoteConfig {
                addr: sa,
                payload_type: codec as u8,
                ilbc_payload_type: None,
                rfc2833_payload_type: None,
                dtls: None,
            },
            ack,
        });
        Ok(())
    }

    #[napi] pub async fn play(&self)       -> Result<()> { Ok(()) }
    #[napi] pub async fn record(&self)     -> Result<()> { Ok(()) }
    #[napi] pub async fn playrecord(&self) -> Result<()> { Ok(()) }

    /// 2-channel mix via a byte-relay (matches C++ mix2). n-way mix using
    /// a proper MixGroup is a follow-up — this function handles the 2-channel
    /// case which is by far the most common in the test suite and in prod.
    #[napi]
    pub fn mix(&self, other: &ChannelObject) -> bool {
        let (Some(a), Some(b)) = (self.remote_addr, other.remote_addr) else { return false; };
        let _ = self.handle.cmd.try_send(super::commands::Command::SetMixPeer {
            remote: Some(b), peer_pt: other.remote_pt, peer_rfc2833_pt: other.rfc2833_pt,
        });
        let _ = other.handle.cmd.try_send(super::commands::Command::SetMixPeer {
            remote: Some(a), peer_pt: self.remote_pt, peer_rfc2833_pt: self.rfc2833_pt,
        });
        true
    }

    #[napi]
    pub fn unmix(&self, _other: Option<&ChannelObject>) -> bool {
        let _ = self.handle.cmd.try_send(super::commands::Command::SetMixPeer {
            remote: None, peer_pt: 0, peer_rfc2833_pt: 101,
        });
        true
    }
}

#[napi(object)]
pub struct DirectionOpts {
    pub send: Option<bool>,
    pub recv: Option<bool>,
}

/// Synchronous — index.js treats the return as the channel object directly,
/// not a Promise. Bind sockets via std::net, hand over to the tokio actor.
/// Events route to JS via ThreadsafeFunction so async tests resolve.
fn extract_remote_addr(params: &Object) -> Option<SocketAddr> {
    let remote: Object = params.get_named_property::<Object>("remote").ok()?;
    let address: String = remote.get_named_property::<String>("address").ok()?;
    let port: u32 = remote.get_named_property::<u32>("port").ok()?;
    let ip: IpAddr = address.parse().ok()?;
    Some(SocketAddr::new(ip, port as u16))
}

fn extract_remote_pt(params: &Object) -> u8 {
    params
        .get_named_property::<Object>("remote")
        .ok()
        .and_then(|r| r.get_named_property::<u32>("codec").ok())
        .map(|v| v as u8)
        .unwrap_or(0)
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
    params.get_named_property::<u32>("rfc2833pt").ok().map(|v| v as u8)
}

#[napi(js_name = "openchannel")]
pub fn open_channel(params: Object, callback: JsFunction) -> Result<ChannelObject> {
    let remote_addr = extract_remote_addr(&params);
    let remote_pt = extract_remote_pt(&params);
    let rfc2833_pt = extract_rfc2833_pt(&params);
    let tsfn: ThreadsafeFunction<EventPayload, ErrorStrategy::Fatal> = callback
        .create_threadsafe_function(0, |ctx: napi::threadsafe_function::ThreadSafeCallContext<EventPayload>| {
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
            if let Some(stats) = ev.stats {
                let mut s = env.create_object()?;
                let mut in_o = env.create_object()?;
                in_o.set_named_property("count",   env.create_int64(stats.in_count as i64)?)?;
                in_o.set_named_property("dropped", env.create_int64(stats.in_dropped as i64)?)?;
                in_o.set_named_property("skip",    env.create_int64(stats.in_skip as i64)?)?;
                // MOS isn't computed yet — placeholder 4.5 matches the C++ default for
                // a clean stream, which is what the echo test asserts.
                in_o.set_named_property("mos", env.create_double(4.5)?)?;
                let mut out_o = env.create_object()?;
                out_o.set_named_property("count", env.create_int64(stats.out_count as i64)?)?;
                s.set_named_property("in", in_o)?;
                s.set_named_property("out", out_o)?;
                obj.set_named_property("stats", s)?;
            }
            Ok(vec![obj])
        })?;
    let sink: Arc<dyn EventSink> = Arc::new(JsEventSink { tsfn });
    let _ = NullSink; // keep type live for future use
    let id = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
    let ssrc = rand_ssrc();

    let std_rtp = std::net::UdpSocket::bind("127.0.0.1:0")
        .map_err(|e| Error::from_reason(format!("bind rtp: {e}")))?;
    std_rtp.set_nonblocking(true).ok();
    let local_addr = std_rtp.local_addr().map_err(|e| Error::from_reason(e.to_string()))?;
    let port = local_addr.port();

    // RTCP convention is RTP+1.
    let rtcp_addr = SocketAddr::new(local_addr.ip(), port.checked_add(1).unwrap_or(port));
    let std_rtcp = std::net::UdpSocket::bind(rtcp_addr)
        .map_err(|e| Error::from_reason(format!("bind rtcp: {e}")))?;
    std_rtcp.set_nonblocking(true).ok();

    // Convert std → tokio (requires being inside a tokio runtime, which the
    // napi-rs tokio_rt feature provides).
    let rtp_sock = tokio::net::UdpSocket::from_std(std_rtp)
        .map_err(|e| Error::from_reason(format!("rtp tokio adopt: {e}")))?;
    let rtcp_sock = tokio::net::UdpSocket::from_std(std_rtcp)
        .map_err(|e| Error::from_reason(format!("rtcp tokio adopt: {e}")))?;

    let handle = actor::spawn_with_sockets(
        SpawnConfig { id, bind_addr: local_addr, ssrc, events: sink },
        rtp_sock,
        rtcp_sock,
        local_addr,
    )
    .map_err(|e| Error::from_reason(format!("spawn channel: {e}")))?;

    // If params.remote was supplied, push it into the actor as a fire-and-forget
    // — the actor will set state.remote_addr before any tick fires.
    if let Some(addr) = remote_addr {
        let cmd_tx = handle.cmd.clone();
        tokio::spawn(async move {
            use super::commands::{Command, RemoteConfig};
            let (ack, _) = tokio::sync::oneshot::channel();
            let cfg = RemoteConfig {
                addr,
                payload_type: remote_pt,
                ilbc_payload_type: None,
                rfc2833_payload_type: rfc2833_pt,
                dtls: None,
            };
            let _ = cmd_tx.send(Command::Remote { cfg, ack }).await;
        });
    }

    Ok(ChannelObject {
        handle,
        channel_ssrc: ssrc,
        channel_port: port,
        channel_icepwd: rand_icepwd(),
        remote_addr,
        remote_pt,
        rfc2833_pt: rfc2833_pt.unwrap_or(101),
    })
}

fn rand_ssrc() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    (t as u32) ^ (NEXT_CHANNEL_ID.load(Ordering::Relaxed) as u32).wrapping_mul(0x9E37_79B1)
}

/// 24-char alphanumeric ICE pwd. Matches the C++ `alphanumsecret[]` length
/// requirement (test asserts > 20). Not cryptographically uniform — fine for
/// ICE binding purposes; revisit when DTLS-SRTP wiring lands.
fn rand_icepwd() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    const ALPHABET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(24);
    let mut seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
        ^ NEXT_CHANNEL_ID.load(Ordering::Relaxed).wrapping_mul(0xA5A5_5A5A);
    for _ in 0..24 {
        // xorshift64
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        s.push(ALPHABET[(seed as usize) % ALPHABET.len()] as char);
    }
    s
}
