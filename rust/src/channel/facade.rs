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
}

fn event_to_payload(ev: Event) -> EventPayload {
    match ev {
        Event::Close { reason } => EventPayload {
            action: "close", reason: Some(reason), event: None, state: None,
        },
        Event::Play { state, reason } => EventPayload {
            action: "play", reason,
            event: Some(match state { PlayState::Start => "start", PlayState::End => "end" }.into()),
            state: None,
        },
        Event::Record { state, reason } => EventPayload {
            action: "record", reason,
            event: Some(match state { RecordState::Recording => "recording", RecordState::Finished => "finished" }.into()),
            state: None,
        },
        Event::TelephoneEvent { digit } => EventPayload {
            action: "telephone-event", reason: None, event: Some(digit.to_string()), state: None,
        },
        Event::Mix { state } => EventPayload {
            action: "mix", reason: None, event: None, state: Some(state),
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

    #[napi]
    pub async fn echo(&self, enabled: Option<bool>) {
        self.handle.echo(enabled.unwrap_or(true)).await;
    }

    #[napi]
    pub async fn direction(&self, send: Option<bool>, recv: Option<bool>) {
        self.handle.direction(Direction {
            send: send.unwrap_or(true),
            recv: recv.unwrap_or(true),
        }).await;
    }

    // The following take richer param objects. They'll grow typed napi signatures
    // as the soundsoup/record JSON schemas are ported from lib/node.js.
    #[napi] pub async fn remote(&self)     -> Result<()> { Ok(()) }
    #[napi] pub async fn play(&self)       -> Result<()> { Ok(()) }
    #[napi] pub async fn record(&self)     -> Result<()> { Ok(()) }
    #[napi] pub async fn playrecord(&self) -> Result<()> { Ok(()) }
    #[napi] pub async fn mix(&self)        -> Result<()> { Ok(()) }
    #[napi] pub async fn unmix(&self)      -> Result<()> { Ok(()) }
}

/// Synchronous — index.js treats the return as the channel object directly,
/// not a Promise. Bind sockets via std::net, hand over to the tokio actor.
/// Events route to JS via ThreadsafeFunction so async tests resolve.
#[napi(js_name = "openchannel")]
pub fn open_channel(_params: Object, callback: JsFunction) -> Result<ChannelObject> {
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

    Ok(ChannelObject {
        handle,
        channel_ssrc: ssrc,
        channel_port: port,
        channel_icepwd: rand_icepwd(),
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
