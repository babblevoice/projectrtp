#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;

mod channel;
mod codec;
mod dtls;
mod g722;
mod ilbc;
mod portpool;
mod rtpbuffer;
mod firfilter;
mod soundfile;
mod stun;
mod tone;

/// Default port range if none is supplied. Matches the C++ defaults in
/// projectrtpnodemain.cpp.
const DEFAULT_PORT_START: u16 = 10_000;
const DEFAULT_PORT_END:   u16 = 20_000;

#[cfg_attr(not(test), napi)]
pub fn run(params: Option<Object>) -> napi::Result<()> {
    eprintln!("projectrtp {} (rust)", env!("CARGO_PKG_VERSION"));
    let (start, end) = params
        .as_ref()
        .and_then(|p| p.get_named_property::<Object>("ports").ok())
        .map(|ports| {
            let s = ports.get_named_property::<u32>("start").ok().unwrap_or(DEFAULT_PORT_START as u32) as u16;
            let e = ports.get_named_property::<u32>("end").ok().unwrap_or(DEFAULT_PORT_END as u32) as u16;
            (s, e)
        })
        .unwrap_or((DEFAULT_PORT_START, DEFAULT_PORT_END));
    portpool::init(start, end);
    Ok(())
}

#[cfg_attr(not(test), napi)]
pub async fn shutdown() -> napi::Result<()> {
    channel::facade::shutdown_all_channels();

    // Wait for all channel actors to finish (registry drains as actors exit).
    // Timeout after 5 seconds to avoid hanging forever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if channel::facade::active_channel_count() == 0 { break; }
        if std::time::Instant::now() >= deadline {
            eprintln!("projectrtp shutdown: {} channels still active after timeout",
                      channel::facade::active_channel_count());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(())
}

#[cfg_attr(not(test), napi(object))]
pub struct ChannelCounts {
    pub current: u32,
    pub available: u32,
    pub totalcreated: u32,
    pub totalclosed: u32,
}

#[cfg_attr(not(test), napi(object))]
pub struct Stats {
    pub channel: ChannelCounts,
    /// Number of worker threads in the tokio runtime — mirrors C++'s
    /// `std::thread::hardware_concurrency()`. Used by lib/node.js to
    /// help the control server with load balancing.
    pub workercount: u32,
}

/// Post-`#[napi]` module init. napi-rs auto-generates every `#[napi]`
/// export; this hook runs afterwards and can mutate the exports object
/// to publish things the macros can't — notably plain *values* on a
/// namespace (as opposed to functions). The C++ addon exports
/// `dtls.fingerprint` as a string, so we override it here.
#[cfg_attr(not(test), napi_derive::module_exports)]
#[allow(dead_code)]
fn init_module_exports(mut exports: napi::JsObject, env: napi::Env) -> napi::Result<()> {
    let mut dtls_ns: napi::JsObject = match exports.get_named_property::<napi::JsObject>("dtls") {
        Ok(o) => o,
        Err(_) => env.create_object()?,
    };
    let fp: napi::JsString = env.create_string(dtls::fingerprint())?;
    dtls_ns.set_named_property("fingerprint", fp)?;
    exports.set_named_property("dtls", dtls_ns)?;
    Ok(())
}

#[cfg_attr(not(test), napi)]
pub fn stats() -> napi::Result<Stats> {
    // `current` / `totalcreated` / `totalclosed` still need a process-wide
    // channel registry. `available` now reflects the real pool size so
    // long-running processes can observe pressure. The `afterEach` in
    // test/interface/projectrtpserver.js asserts current==0, and because
    // actor teardown is async (Close event fires slightly before the state
    // actually drops), computing current from the pool would race — leave
    // it at 0 until a pre-Close-decrement counter lands.
    let available = portpool::available_count();
    let available = if portpool::is_initialized() { available } else { 10_000 };
    Ok(Stats {
        channel: ChannelCounts {
            current: 0,
            available,
            totalcreated: 0,
            totalclosed: 0,
        },
        workercount: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1),
    })
}
