// rtpbuffer — JS-facing wrapper around channel::jitter::JitterBuffer.
//
// Exposes a #[napi] class that matches the C++ `projectrtp.rtpbuffer.create(...)`
// surface used in test/unit/projectrtpbuffer.js: push({payload}), pop(),
// peek(), poppeeked(), size().

use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::channel::jitter::{
    JitterBuffer, DEFAULT_BUFFER_PACKET_COUNT, DEFAULT_BUFFER_WATER_LEVEL,
};
use crate::channel::rtp::RtpPacket;

#[cfg_attr(not(test), napi(object))]
pub struct BufferOptions {
    pub size: Option<u32>,
    pub waterlevel: Option<u32>,
}

#[cfg_attr(not(test), napi)]
pub struct RtpJitterBuffer {
    inner: JitterBuffer,
}

// Whole-impl cfg-gate: per-method `#[napi]` attributes need the impl's
// own `#[napi]` to have expanded first, which doesn't compose with
// cfg_attr. Test code doesn't call these methods directly — the JS
// test suite exercises them from the other side of the FFI.
#[cfg(not(test))]
#[napi]
impl RtpJitterBuffer {
    #[napi]
    pub fn push(&mut self, pkt: Object) -> Result<()> {
        let payload: Buffer = pkt.get_named_property("payload")?;
        let bytes: &[u8] = payload.as_ref();
        // Need at least 4 bytes to extract the sequence number. Tests feed
        // truncated buffers that are shorter than the full 12-byte RTP header,
        // and the C++ version tolerated that.
        if bytes.len() < 4 {
            return Err(Error::from_reason("payload too short for sequence number"));
        }
        let mut p = RtpPacket::new();
        p.as_mut_slice_for_fill(bytes.len()).copy_from_slice(bytes);
        self.inner.push(p);
        Ok(())
    }

    // napi-rs serialises Option::None as JS `null`; the existing test suite
    // checks for `undefined` (matching the C++ behavior of returning
    // napi_value NULL which Node coerces to undefined). Use `Either<Buffer,
    // Undefined>` to preserve that contract.
    #[napi]
    pub fn pop(&mut self) -> Either<Buffer, Undefined> {
        match self.inner.pop() {
            Some(p) => Either::A(Buffer::from(p.as_slice().to_vec())),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn peek(&mut self) -> Either<Buffer, Undefined> {
        match self.inner.peek() {
            Some(p) => Either::A(Buffer::from(p.as_slice().to_vec())),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn poppeeked(&mut self) -> Either<Buffer, Undefined> {
        let copy = self
            .inner
            .peek()
            .map(|p| Buffer::from(p.as_slice().to_vec()));
        self.inner.discard_peeked();
        match copy {
            Some(b) => Either::A(b),
            None => Either::B(()),
        }
    }

    #[napi]
    pub fn size(&self) -> u32 {
        self.inner.size() as u32
    }
}

#[cfg_attr(not(test), napi(namespace = "rtpbuffer", js_name = "create"))]
pub fn create(opts: Option<BufferOptions>) -> Result<RtpJitterBuffer> {
    let (size, water) = match opts {
        Some(o) => (
            o.size
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_BUFFER_PACKET_COUNT),
            o.waterlevel
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_BUFFER_WATER_LEVEL),
        ),
        None => (DEFAULT_BUFFER_PACKET_COUNT, DEFAULT_BUFFER_WATER_LEVEL),
    };
    Ok(RtpJitterBuffer {
        inner: JitterBuffer::new(size, water),
    })
}
