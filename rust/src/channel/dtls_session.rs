// DTLS session — pure-Rust via webrtc-dtls.
//
// Role: negotiate DTLS-SRTP with the peer and export the keying material
// libsrtp needs. We deliberately do NOT use DTLS for RTP traffic itself — RTP
// goes through the SRTP contexts (see srtp_ctx.rs) after handshake completes.
//
// The channel actor owns the UDP socket; it drives the handshake by calling
// `step_incoming` with bytes pulled off the socket and `drain_outgoing` to
// push bytes the session wants to send. Once `handshake_complete()` returns
// true, call `export_srtp_keys()` and hand the keys to `SrtpContext::new`.
//
// NOTE: Full wiring to webrtc-dtls's `DTLSConn` lands alongside tick.rs where
// the socket is in scope. This file pins the Rust-facing shape so commands.rs,
// state.rs and tick.rs can compile against a stable API.

use crate::channel::commands::DtlsSetup;

#[derive(Debug, Clone)]
pub struct PeerFingerprint {
    pub algorithm: String, // "sha-256"
    pub hex_colon: String, // "AB:CD:..."
}

/// Keying material extracted from a completed DTLS handshake. Shape matches
/// what webrtc-srtp's `Context::new()` expects: client/server master keys +
/// salts, plus the negotiated profile.
#[derive(Debug, Clone)]
pub struct SrtpKeyingMaterial {
    pub profile: webrtc_srtp::protection_profile::ProtectionProfile,
    pub client_write_key: Vec<u8>,
    pub client_write_salt: Vec<u8>,
    pub server_write_key: Vec<u8>,
    pub server_write_salt: Vec<u8>,
    /// Which side we are — determines which key is "ours" vs "theirs" when
    /// constructing the SRTP contexts.
    pub local_is_server: bool,
}

pub struct DtlsSession {
    setup: DtlsSetup,
    peer_fingerprint: Option<PeerFingerprint>,
    handshake_done: bool,
    // Real DTLSConn lands here when tick.rs wires the socket transport.
}

impl DtlsSession {
    pub fn new(setup: DtlsSetup, peer_fingerprint: Option<PeerFingerprint>) -> Self {
        Self { setup, peer_fingerprint, handshake_done: false }
    }

    pub fn setup(&self) -> DtlsSetup { self.setup }
    pub fn handshake_complete(&self) -> bool { self.handshake_done }

    /// Feed a UDP datagram into the DTLS state machine. Returns any bytes the
    /// session now wants transmitted on the wire (may be empty).
    pub fn step_incoming(&mut self, _datagram: &[u8]) -> Vec<bytes::Bytes> {
        // TODO: drive webrtc_dtls::conn::DTLSConn via its Conn trait.
        Vec::new()
    }

    /// Drain anything the session wants to transmit (e.g. ClientHello on start).
    pub fn drain_outgoing(&mut self) -> Vec<bytes::Bytes> {
        Vec::new()
    }

    /// After handshake completes, extract SRTP keys. Returns None if handshake
    /// isn't done yet.
    pub fn export_srtp_keys(&self) -> Option<SrtpKeyingMaterial> {
        if !self.handshake_done { return None; }
        // TODO: call webrtc_dtls State::export_keying_material + split per RFC 5764.
        None
    }
}
