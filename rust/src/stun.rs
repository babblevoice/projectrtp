// STUN — port of projectrtpstun.cpp.
//
// Very-minimal STUN (RFC 5389) server used by the WebRTC ICE path: accepts a
// Binding Request, validates MESSAGE-INTEGRITY (HMAC-SHA1) and FINGERPRINT
// (CRC-32 XOR 0x5354554E), and emits a Binding Response with
// XOR-MAPPED-ADDRESS + MESSAGE-INTEGRITY + FINGERPRINT.
//
// Internal module only — no #[napi] exports; channel calls this directly.

use std::net::SocketAddr;

use hmac::{Hmac, Mac};
use sha1::Sha1;
type HmacSha1 = Hmac<Sha1>;

const MAGIC_COOKIE: u32 = 0x2112_A442;
const FINGERPRINT_XOR: u32 = 0x5354_554E;
const STUN_HEADER_LEN: usize = 20;

#[inline]
fn read_u16(pk: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([pk[off], pk[off + 1]])
}

#[inline]
fn read_u32(pk: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([pk[off], pk[off + 1], pk[off + 2], pk[off + 3]])
}

#[inline]
fn write_u16(pk: &mut [u8], off: usize, v: u16) {
    pk[off..off + 2].copy_from_slice(&v.to_be_bytes());
}

#[inline]
fn write_u32(pk: &mut [u8], off: usize, v: u32) {
    pk[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

// Message type (method) decode per RFC 5389 §6. Result: method bits only.
fn message_method(pk: &[u8]) -> u16 {
    let mt = read_u16(pk, 0);
    ((mt & 0b0011_1110_0000_0000) >> 9)
        | ((mt & 0b0000_0000_1110_0000) >> 5)
        | (mt & 0b0000_0000_0000_1111)
}

fn message_class(pk: &[u8]) -> u8 {
    let mt = read_u16(pk, 0) & 0b0011_1111_1111_1111;
    let c0 = mt & 0b0000_0000_0001_0000;
    let c1 = mt & 0b0000_0001_0000_0000;
    ((c0 >> 4) as u8) | ((c1 >> 8) as u8)
}

fn message_length(pk: &[u8]) -> u16 {
    read_u16(pk, 2)
}
fn set_message_length(pk: &mut [u8], v: u16) {
    write_u16(pk, 2, v);
}
fn magic_cookie(pk: &[u8]) -> u32 {
    read_u32(pk, 4)
}

pub fn is_stun(pk: &[u8]) -> bool {
    if pk.len() < STUN_HEADER_LEN {
        return false;
    }
    if (pk[0] & 0b1100_0000) != 0 {
        return false;
    }
    if (pk.len() - STUN_HEADER_LEN) != message_length(pk) as usize {
        return false;
    }
    if magic_cookie(pk) != MAGIC_COOKIE {
        return false;
    }
    true
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn verify_integrity(pk: &mut [u8], attr_off: usize, key: &[u8]) -> bool {
    // Per RFC 5389 §15.4: set the message length to the value it would have if
    // MESSAGE-INTEGRITY were the last attribute, then HMAC over the header +
    // attributes up to (but not including) MESSAGE-INTEGRITY.
    let saved_len = message_length(pk);
    let new_len = (attr_off - STUN_HEADER_LEN + 24) as u16;
    set_message_length(pk, new_len);
    let hashed = &pk[..attr_off];
    let computed = hmac_sha1(key, hashed);
    let digest = &pk[attr_off + 4..attr_off + 24];
    let ok = computed.as_slice() == digest;
    set_message_length(pk, saved_len);
    ok
}

fn verify_fingerprint(pk: &[u8], attr_off: usize) -> bool {
    // FINGERPRINT covers everything up to (but not including) itself.
    let crc = crc32fast::hash(&pk[..attr_off]) ^ FINGERPRINT_XOR;
    let got = read_u32(pk, attr_off + 4);
    crc == got
}

fn walk_attributes(pk: &mut [u8], remote_key: &[u8]) -> bool {
    if remote_key.is_empty() {
        return false;
    }
    let total = STUN_HEADER_LEN + message_length(pk) as usize;
    if total > pk.len() {
        return false;
    }

    let mut off = STUN_HEADER_LEN;
    while off + 4 <= total {
        let attr_type = read_u16(pk, off);
        let attr_len = read_u16(pk, off + 2) as usize;
        if off + 4 + attr_len > total {
            return false;
        }

        match attr_type {
            0x0008 => {
                // MESSAGE-INTEGRITY
                if !verify_integrity(pk, off, remote_key) {
                    return false;
                }
            }
            0x8028 => {
                // FINGERPRINT
                if !verify_fingerprint(pk, off) {
                    return false;
                }
            }
            // USERNAME / PRIORITY / USE-CANDIDATE / ICE-CONTROLLING /
            // GOOG-NETWORK-INFO — accept without validation.
            _ => {}
        }

        let padding = (4 - (attr_len % 4)) % 4;
        off += 4 + attr_len + padding;
    }
    true
}

// Writes an XOR-MAPPED-ADDRESS attribute for `endpoint` at `attr_off` in `response`.
// Returns bytes written (including the 4-byte attribute header).
fn write_xor_mapped_address(response: &mut [u8], attr_off: usize, endpoint: SocketAddr) -> usize {
    let xor_port = endpoint.port() ^ ((MAGIC_COOKIE >> 16) as u16);
    match endpoint {
        SocketAddr::V4(v4) => {
            write_u16(response, attr_off, 0x0020);
            write_u16(response, attr_off + 2, 8);
            response[attr_off + 4] = 0; // reserved
            response[attr_off + 5] = 0x01; // IPv4
            write_u16(response, attr_off + 6, xor_port);
            let ip = u32::from(*v4.ip());
            write_u32(response, attr_off + 8, ip ^ MAGIC_COOKIE);
            4 + 8
        }
        SocketAddr::V6(v6) => {
            write_u16(response, attr_off, 0x0020);
            write_u16(response, attr_off + 2, 20);
            response[attr_off + 4] = 0;
            response[attr_off + 5] = 0x02; // IPv6
            write_u16(response, attr_off + 6, xor_port);
            let addr = v6.ip().octets();
            let cookie = MAGIC_COOKIE.to_be_bytes();
            let transid = &response[8..20];
            let xor_key: [u8; 16] = {
                let mut k = [0u8; 16];
                k[..4].copy_from_slice(&cookie);
                k[4..].copy_from_slice(transid);
                k
            };
            for i in 0..16 {
                response[attr_off + 8 + i] = addr[i] ^ xor_key[i];
            }
            4 + 20
        }
    }
}

fn add_message_integrity(response: &mut [u8], attr_off: usize, key: &[u8]) -> usize {
    write_u16(response, attr_off, 0x0008);
    write_u16(response, attr_off + 2, 20);
    let new_len = (attr_off - STUN_HEADER_LEN + 24) as u16;
    set_message_length(response, new_len);
    let digest = hmac_sha1(key, &response[..attr_off]);
    response[attr_off + 4..attr_off + 24].copy_from_slice(&digest);
    24
}

fn add_fingerprint(response: &mut [u8], attr_off: usize) -> usize {
    write_u16(response, attr_off, 0x8028);
    write_u16(response, attr_off + 2, 4);
    let new_len = (attr_off - STUN_HEADER_LEN + 8) as u16;
    set_message_length(response, new_len);
    let crc = crc32fast::hash(&response[..attr_off]) ^ FINGERPRINT_XOR;
    write_u32(response, attr_off + 4, crc);
    8
}

fn create_binding_response(
    request: &[u8],
    response: &mut [u8],
    endpoint: SocketAddr,
    local_key: &[u8],
) -> usize {
    if response.len() < STUN_HEADER_LEN || local_key.is_empty() {
        return 0;
    }
    for b in response.iter_mut().take(STUN_HEADER_LEN) {
        *b = 0;
    }

    write_u16(response, 0, 0x0101); // Binding Response
    write_u32(response, 4, MAGIC_COOKIE);
    // Copy 12-byte transaction id from request.
    response[8..20].copy_from_slice(&request[8..20]);

    let mut off = STUN_HEADER_LEN;
    off += write_xor_mapped_address(response, off, endpoint);
    off += add_message_integrity(response, off, local_key);
    off += add_fingerprint(response, off);
    off
}

/// Handle an inbound packet. Returns the number of bytes written to `response`
/// (0 if no response should be sent or validation failed).
pub fn handle(
    pk: &mut [u8],
    response: &mut [u8],
    endpoint: SocketAddr,
    local_key: &[u8],
    remote_key: &[u8],
) -> usize {
    if !is_stun(pk) {
        return 0;
    }
    if message_method(pk) != 0x01 {
        return 0;
    }

    if message_class(pk) == 0 {
        // Binding Request
        if !walk_attributes(pk, remote_key) {
            return 0;
        }
        create_binding_response(pk, response, endpoint, local_key)
    } else {
        // Binding Success response from peer — validate but don't reply.
        let _ = walk_attributes(pk, local_key);
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    fn build_request(key: &[u8]) -> Vec<u8> {
        // Minimal binding request: header + MESSAGE-INTEGRITY + FINGERPRINT.
        let mut pk = vec![0u8; STUN_HEADER_LEN];
        write_u16(&mut pk, 0, 0x0001); // Binding Request
        write_u32(&mut pk, 4, MAGIC_COOKIE);
        // transaction id = [1..13) fixed
        for i in 0..12 {
            pk[8 + i] = (i as u8) + 1;
        }

        // MESSAGE-INTEGRITY attribute at offset 20
        let mi_off = pk.len();
        pk.extend_from_slice(&[0u8; 24]);
        write_u16(&mut pk, mi_off, 0x0008);
        write_u16(&mut pk, mi_off + 2, 20);

        // Set message length to cover up to end of MI.
        let mi_msg_len = (mi_off - STUN_HEADER_LEN + 24) as u16;
        set_message_length(&mut pk, mi_msg_len);
        let digest = hmac_sha1(key, &pk[..mi_off]);
        pk[mi_off + 4..mi_off + 24].copy_from_slice(&digest);

        // FINGERPRINT attribute
        let fp_off = pk.len();
        pk.extend_from_slice(&[0u8; 8]);
        write_u16(&mut pk, fp_off, 0x8028);
        write_u16(&mut pk, fp_off + 2, 4);
        let fp_msg_len = (fp_off - STUN_HEADER_LEN + 8) as u16;
        set_message_length(&mut pk, fp_msg_len);
        let crc = crc32fast::hash(&pk[..fp_off]) ^ FINGERPRINT_XOR;
        write_u32(&mut pk, fp_off + 4, crc);

        pk
    }

    #[test]
    fn is_stun_detects_valid_header() {
        let pk = build_request(b"remote");
        assert!(is_stun(&pk));
    }

    #[test]
    fn is_stun_rejects_non_stun() {
        let junk = [0xFFu8; 60];
        assert!(!is_stun(&junk));
    }

    #[test]
    fn handle_roundtrip_produces_valid_response() {
        let mut req = build_request(b"remote-key");
        let mut resp = [0u8; 256];
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 4242));
        let n = handle(&mut req, &mut resp, addr, b"local-key", b"remote-key");
        assert!(n > 0, "expected response");
        assert!(is_stun(&resp[..n]));
        // Response should be a Binding Response (class = success, method = binding).
        assert_eq!(read_u16(&resp, 0), 0x0101);
        // Transaction id must match request.
        assert_eq!(&resp[8..20], &req[8..20]);
        // Fingerprint self-check.
        let fp_off = n - 8;
        assert!(verify_fingerprint(&resp[..n], fp_off));
    }

    #[test]
    fn handle_rejects_bad_integrity() {
        let mut req = build_request(b"correct-key");
        let mut resp = [0u8; 256];
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 4242));
        // Use wrong remote key — integrity check must fail, no response.
        let n = handle(&mut req, &mut resp, addr, b"local", b"wrong-key");
        assert_eq!(n, 0);
    }
}
