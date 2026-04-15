#![deny(clippy::all)]

use napi_derive::napi;

mod channel;
mod codec;
mod dtls;
mod rtpbuffer;
mod firfilter;
mod soundfile;
mod stun;
mod tone;

#[napi]
pub fn run() -> napi::Result<()> {
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
    // TODO: wire real counters from the channel registry once top-level
    // projectrtp state exists.
    Ok(Stats {
        channel: ChannelCounts {
            current: 0,
            available: 10000,
            totalcreated: 0,
            totalclosed: 0,
        },
    })
}
