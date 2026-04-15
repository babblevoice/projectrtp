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
use napi_derive::napi;

use super::actor::{self, Event, EventSink, SpawnConfig};
use super::commands::{Direction, Handle};

static NEXT_CHANNEL_ID: AtomicU64 = AtomicU64::new(1);

/// Stub event sink that drops events. Replaced by a ThreadsafeFunction-backed
/// sink in the integration-test pass (Task #10 follow-up).
struct NullSink;
impl EventSink for NullSink {
    fn post(&self, _ev: Event) {}
}

#[napi(object)]
pub struct ChannelLocal {
    pub port: u32,
    pub ssrc: u32,
}

#[napi]
pub struct ChannelObject {
    handle: Handle,
    channel_ssrc: u32,
    channel_port: u16,
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
    #[napi(getter)]
    pub fn local(&self) -> ChannelLocal {
        ChannelLocal { port: self.channel_port as u32, ssrc: self.channel_ssrc }
    }

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
#[napi(js_name = "openchannel")]
pub fn open_channel() -> Result<ChannelObject> {
    // (params, callback) from JS are silently dropped — napi-rs ignores
    // extra positional args. Param parsing + ThreadsafeFunction bridge are
    // a follow-up; events route to NullSink for now.
    let sink: Arc<dyn EventSink> = Arc::new(NullSink);
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

    Ok(ChannelObject { handle, channel_ssrc: ssrc, channel_port: port })
}

fn rand_ssrc() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    (t as u32) ^ (NEXT_CHANNEL_ID.load(Ordering::Relaxed) as u32).wrapping_mul(0x9E37_79B1)
}
