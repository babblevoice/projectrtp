// SRTP context — pure-Rust via webrtc-srtp.
//
// Two streams per channel: one for outbound (protect) and one for inbound
// (unprotect). The webrtc_srtp::Context type is async, so protect/unprotect
// are async methods here — they take a mutable buffer and encrypt/decrypt in
// place (or return a new buffer; final shape chosen alongside tick.rs).
//
// Drop semantics are automatic: webrtc_srtp::Context holds only Rust-owned
// state (no libsrtp C context), so going out of scope cleans everything up.
// That's the whole point of preferring pure Rust here.

use crate::channel::dtls_session::SrtpKeyingMaterial;

pub struct SrtpContext {
    _km: SrtpKeyingMaterial,
    // Real webrtc_srtp::Context instances are created here once tick.rs wires
    // them in. The Context API requires an async runtime to drive internal
    // replay detection, so construction is deferred until the actor task is
    // running.
}

impl SrtpContext {
    pub fn new(km: SrtpKeyingMaterial) -> Self {
        Self { _km: km }
    }

    // Protect an outbound RTP packet in place. Returns the new post-encryption
    // length. TODO: wire to webrtc_srtp::Context::protect_rtp.
    pub async fn protect_rtp(&mut self, _buf: &mut Vec<u8>) -> Result<(), SrtpError> {
        Err(SrtpError::NotYetImplemented)
    }

    pub async fn unprotect_rtp(&mut self, _buf: &mut Vec<u8>) -> Result<(), SrtpError> {
        Err(SrtpError::NotYetImplemented)
    }
}

#[derive(Debug)]
pub enum SrtpError {
    NotYetImplemented,
    Crypto(String),
}

impl std::fmt::Display for SrtpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotYetImplemented => write!(f, "SRTP: not yet implemented"),
            Self::Crypto(s) => write!(f, "SRTP crypto: {s}"),
        }
    }
}

impl std::error::Error for SrtpError {}
