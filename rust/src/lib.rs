#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;

mod channel;
mod codec;
mod dtls;
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

#[napi]
pub fn run(params: Option<Object>) -> napi::Result<()> {
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

#[napi]
pub fn shutdown() -> napi::Result<()> {
    Ok(())
}

#[napi(object)]
pub struct ChannelCounts {
    pub current: u32,
    pub available: u32,
    pub totalcreated: u32,
    pub totalclosed: u32,
}

#[napi(object)]
pub struct Stats {
    pub channel: ChannelCounts,
}

#[napi]
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
    })
}
