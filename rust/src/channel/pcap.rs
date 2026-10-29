// Packet capture — writes a channel's RTP to a libpcap file.
//
// The sibling of `recorder.rs`: the recorder writes what the call *sounded*
// like, this writes what was actually *on the wire*. Same shape deliberately —
// armed by a command, owned by the channel actor, closed on finish or channel
// close, and announced to JS with the finished path so babble-rtp can ship it
// off the node exactly as it already ships a wav.
//
// Why this belongs in projectrtp rather than a tcpdump sidecar:
//
//   - projectrtp owns the socket, so capture is scoped to one channel by
//     construction. A port-filtered tcpdump has to guess which ports belong to
//     the call and races channel teardown/reuse.
//   - it holds the SRTP keys, so we can write *cleartext* media. A sidecar on
//     the wire captures ciphertext that nobody can open later.
//   - no NET_RAW capability, no extra container, nothing to co-schedule.
//
// We tap two points in the tick (see tick.rs): inbound immediately after
// `pop_and_decrypt_inbound`, and outbound at the top of `send_rtp` before
// `encrypt_rtp`. Both are the cleartext side of the crypto boundary.
//
// The captured bytes are the real RTP, but the IP/UDP headers around them are
// synthesised from the channel's own addresses — we never see the peer's
// framing. That is what a capture consumer needs (Wireshark decodes the RTP,
// "Play Streams" works, the flow reads correctly) without pretending to
// reproduce headers we do not have.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// libpcap file magic, microsecond timestamps, written little-endian.
const PCAP_MAGIC: u32 = 0xa1b2_c3d4;
const PCAP_VERSION_MAJOR: u16 = 2;
const PCAP_VERSION_MINOR: u16 = 4;
/// LINKTYPE_ETHERNET. We do have to fabricate MACs, but the alternative
/// (LINKTYPE_RAW, no link layer) is not readable by projectrtp's own pcap
/// decoder in test/interface/pcap.js, which keys everything off an ethertype.
/// Emitting ethernet means a capture taken from a live customer call can be
/// replayed straight into the existing DTMF and codec tests, which is worth
/// far more than the 14 bytes a frame it costs.
const LINKTYPE_ETHERNET: u32 = 1;
const ETHER_HEADER_LEN: usize = 14;
const ETHERTYPE_IPV4: u16 = 0x0800;
/// Synthetic MACs. The 0x02 leading octet marks these locally administered
/// and unicast, so they cannot be confused with a real card. Which end is
/// which follows the packet direction, so Wireshark's conversation view still
/// separates the two sides.
const MAC_LOCAL: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
const MAC_REMOTE: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
/// Largest packet we will store. RTP over UDP never approaches this.
const SNAPLEN: u32 = 65535;

const IPV4_HEADER_LEN: usize = 20;
const UDP_HEADER_LEN: usize = 8;
const IPPROTO_UDP: u8 = 17;
/// Arbitrary but sane: these packets never traverse a real network.
const IP_TTL: u8 = 64;

/// Which way a captured packet was travelling. Only used to decide which of
/// the channel's addresses is the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcapDirection {
    /// Received from the peer — written after SRTP decryption.
    In,
    /// Sent to the peer — written before SRTP encryption.
    Out,
}

#[derive(Debug, Clone)]
pub struct PcapConfig {
    pub file: PathBuf,
    /// Stop capturing once the file reaches this size. Capture is armed by an
    /// operator on a live platform, so an unbounded writer is a disk-space
    /// incident waiting to happen.
    pub max_bytes: Option<u64>,
    /// Address to record as our own end. The RTP sockets bind to 0.0.0.0, so
    /// without this every capture shows one side as "0.0.0.0", which reads as
    /// a broken capture. Callers pass the address they advertised in SDP —
    /// the one the far end is actually sending to.
    pub local_address: Option<Ipv4Addr>,
    /// Stop capturing after this many milliseconds of wall clock.
    pub max_duration_ms: Option<u64>,
}

/// Writes RTP packets to a libpcap file, wrapped in synthesised IPv4/UDP
/// headers.
pub struct PcapWriter {
    file: BufWriter<File>,
    path: PathBuf,
    max_bytes: Option<u64>,
    local_address: Option<Ipv4Addr>,
    max_duration_ms: Option<u64>,
    /// Bytes written to the file, including headers.
    written: u64,
    packets: u64,
    /// Packets dropped because the addresses were not IPv4 — surfaced so a
    /// silently short capture is explainable rather than mysterious.
    skipped: u64,
    started: SystemTime,
    /// IPv4 identification field, incremented per packet.
    ip_id: u16,
    finished: bool,
    /// Which limit ended the capture, if one did.
    stop_reason: Option<&'static str>,
}

impl PcapWriter {
    /// Create the file and write the libpcap global header.
    pub fn create(cfg: &PcapConfig) -> io::Result<Self> {
        if let Some(parent) = cfg.file.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut file = BufWriter::new(File::create(&cfg.file)?);
        let mut header = Vec::with_capacity(24);
        header.extend_from_slice(&PCAP_MAGIC.to_le_bytes());
        header.extend_from_slice(&PCAP_VERSION_MAJOR.to_le_bytes());
        header.extend_from_slice(&PCAP_VERSION_MINOR.to_le_bytes());
        header.extend_from_slice(&0i32.to_le_bytes()); // thiszone — UTC
        header.extend_from_slice(&0u32.to_le_bytes()); // sigfigs
        header.extend_from_slice(&SNAPLEN.to_le_bytes());
        header.extend_from_slice(&LINKTYPE_ETHERNET.to_le_bytes());
        file.write_all(&header)?;

        Ok(Self {
            file,
            path: cfg.file.clone(),
            max_bytes: cfg.max_bytes,
            local_address: cfg.local_address,
            max_duration_ms: cfg.max_duration_ms,
            written: header.len() as u64,
            packets: 0,
            skipped: 0,
            started: SystemTime::now(),
            ip_id: 0,
            finished: false,
            stop_reason: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn packets(&self) -> u64 {
        self.packets
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    pub fn bytes(&self) -> u64 {
        self.written
    }

    /// Which limit ended the capture - `maxsize` or `maxduration` - if one
    /// did. Reported as the finished event's reason.
    pub fn stop_reason(&self) -> Option<&'static str> {
        self.stop_reason
    }

    /// Have we hit a configured limit, and which? Checked before each write so
    /// the file never exceeds `max_bytes`.
    fn limit_reached(&self, next_len: u64) -> Option<&'static str> {
        if let Some(max) = self.max_bytes {
            if self.written + next_len > max {
                return Some("maxsize");
            }
        }
        if let Some(max_ms) = self.max_duration_ms {
            let elapsed = self.started.elapsed().unwrap_or_default();
            if elapsed.as_millis() as u64 >= max_ms {
                return Some("maxduration");
            }
        }
        None
    }

    /// Write one RTP packet. `local` and `remote` are the channel's own
    /// addresses; `direction` picks which is the source.
    ///
    /// Returns `true` while the capture is still open. A `false` return means
    /// a limit was reached and the caller should finish the capture — it is
    /// not an error.
    pub fn write_packet(
        &mut self,
        direction: PcapDirection,
        local: SocketAddr,
        remote: SocketAddr,
        payload: &[u8],
    ) -> io::Result<bool> {
        if self.finished {
            return Ok(false);
        }

        let (src, dst) = match direction {
            PcapDirection::In => (remote, local),
            PcapDirection::Out => (local, remote),
        };

        // IPv6 legs are not captured. Writing an IPv4 header around them would
        // produce a file that decodes as nonsense, which is worse than a short
        // file plus a count that says so.
        let (src4, dst4) = match (src.ip(), dst.ip()) {
            (std::net::IpAddr::V4(s), std::net::IpAddr::V4(d)) => (s, d),
            _ => {
                self.skipped += 1;
                return Ok(true);
            }
        };

        // The sockets bind to 0.0.0.0. Swap in the advertised address so the
        // capture names both ends; leave a genuinely bound address alone.
        let (src4, dst4) = match (self.local_address, direction) {
            (Some(a), PcapDirection::In) if dst4.is_unspecified() => (src4, a),
            (Some(a), PcapDirection::Out) if src4.is_unspecified() => (a, dst4),
            _ => (src4, dst4),
        };

        let total =
            (16 + ETHER_HEADER_LEN + IPV4_HEADER_LEN + UDP_HEADER_LEN + payload.len()) as u64;
        if let Some(reason) = self.limit_reached(total) {
            self.stop_reason = Some(reason);
            self.finish()?;
            return Ok(false);
        }

        let (src_mac, dst_mac) = match direction {
            PcapDirection::In => (MAC_REMOTE, MAC_LOCAL),
            PcapDirection::Out => (MAC_LOCAL, MAC_REMOTE),
        };
        let frame = build_frame(
            src4,
            src.port(),
            dst4,
            dst.port(),
            payload,
            self.ip_id,
            src_mac,
            dst_mac,
        );
        self.ip_id = self.ip_id.wrapping_add(1);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut record = Vec::with_capacity(16 + frame.len());
        record.extend_from_slice(&(now.as_secs() as u32).to_le_bytes());
        record.extend_from_slice(&now.subsec_micros().to_le_bytes());
        record.extend_from_slice(&(frame.len() as u32).to_le_bytes()); // incl_len
        record.extend_from_slice(&(frame.len() as u32).to_le_bytes()); // orig_len
        record.extend_from_slice(&frame);

        self.file.write_all(&record)?;
        self.written += record.len() as u64;
        self.packets += 1;
        Ok(true)
    }

    /// Flush and close. Idempotent — the actor closes captures on both an
    /// explicit finish and on channel close, and those can race.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.file.flush()
    }
}

impl Drop for PcapWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// Build one ethernet frame: MAC header + IPv4 header + UDP header + payload.
#[allow(clippy::too_many_arguments)]
fn build_frame(
    src: Ipv4Addr,
    src_port: u16,
    dst: Ipv4Addr,
    dst_port: u16,
    payload: &[u8],
    ip_id: u16,
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
) -> Vec<u8> {
    let udp_len = (UDP_HEADER_LEN + payload.len()) as u16;
    let total_len = (IPV4_HEADER_LEN as u16).saturating_add(udp_len);

    let mut frame = Vec::with_capacity(ETHER_HEADER_LEN + total_len as usize);

    // ---- Ethernet II ----
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&src_mac);
    frame.extend_from_slice(&ETHERTYPE_IPV4.to_be_bytes());

    // ---- IPv4 (RFC 791) ----
    frame.push(0x45); // version 4, IHL 5 (no options)
    frame.push(0x00); // DSCP / ECN
    frame.extend_from_slice(&total_len.to_be_bytes());
    frame.extend_from_slice(&ip_id.to_be_bytes());
    frame.extend_from_slice(&0x4000u16.to_be_bytes()); // don't fragment
    frame.push(IP_TTL);
    frame.push(IPPROTO_UDP);
    frame.extend_from_slice(&[0, 0]); // checksum placeholder
    frame.extend_from_slice(&src.octets());
    frame.extend_from_slice(&dst.octets());

    // Fill the header checksum in place. Wireshark flags a bad one, and a
    // capture people distrust is worthless.
    let ip = ETHER_HEADER_LEN;
    let checksum = ipv4_checksum(&frame[ip..ip + IPV4_HEADER_LEN]);
    frame[ip + 10..ip + 12].copy_from_slice(&checksum.to_be_bytes());

    // ---- UDP (RFC 768) ----
    frame.extend_from_slice(&src_port.to_be_bytes());
    frame.extend_from_slice(&dst_port.to_be_bytes());
    frame.extend_from_slice(&udp_len.to_be_bytes());
    // Checksum 0 = "not computed", explicitly allowed over IPv4. Wireshark
    // accepts it without complaint, unlike a wrong value.
    frame.extend_from_slice(&[0, 0]);

    frame.extend_from_slice(payload);
    frame
}

/// Standard one's-complement header checksum (RFC 1071).
fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < header.len() {
        sum += u32::from(u16::from_be_bytes([header[i], header[i + 1]]));
        i += 2;
    }
    if i < header.len() {
        sum += u32::from(header[i]) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn tmpfile(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "projectrtp-pcap-test-{}-{}.pcap",
            name,
            std::process::id()
        ));
        p
    }

    fn read(path: &Path) -> Vec<u8> {
        let mut v = Vec::new();
        File::open(path).unwrap().read_to_end(&mut v).unwrap();
        v
    }

    fn cfg(path: &Path) -> PcapConfig {
        PcapConfig {
            file: path.to_path_buf(),
            max_bytes: None,
            max_duration_ms: None,
            local_address: None,
        }
    }

    fn addrs() -> (SocketAddr, SocketAddr) {
        (
            "10.0.0.1:12000".parse().unwrap(),
            "10.0.0.2:40000".parse().unwrap(),
        )
    }

    #[test]
    fn writes_a_valid_global_header() {
        let path = tmpfile("global");
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        w.finish().unwrap();
        let bytes = read(&path);

        assert_eq!(bytes.len(), 24, "global header is 24 bytes");
        assert_eq!(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            PCAP_MAGIC
        );
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 4);
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            SNAPLEN
        );
        // A wrong link type makes every packet decode as garbage.
        assert_eq!(
            u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
            LINKTYPE_ETHERNET
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn writes_packet_with_ip_and_udp_headers() {
        let path = tmpfile("packet");
        let (local, remote) = addrs();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        let payload = [0x80u8, 0x00, 0x00, 0x01, 0xde, 0xad, 0xbe, 0xef];
        w.write_packet(PcapDirection::In, local, remote, &payload)
            .unwrap();
        w.finish().unwrap();

        let bytes = read(&path);
        let rec = &bytes[24..];
        let incl = u32::from_le_bytes(rec[8..12].try_into().unwrap()) as usize;
        assert_eq!(
            incl,
            ETHER_HEADER_LEN + IPV4_HEADER_LEN + UDP_HEADER_LEN + payload.len()
        );
        assert_eq!(
            u32::from_le_bytes(rec[12..16].try_into().unwrap()) as usize,
            incl
        );

        let eth = &rec[16..16 + incl];
        // Inbound: the peer is the source, at both layers.
        assert_eq!(&eth[0..6], &MAC_LOCAL, "dst mac is local for inbound");
        assert_eq!(&eth[6..12], &MAC_REMOTE, "src mac is remote for inbound");
        assert_eq!(u16::from_be_bytes([eth[12], eth[13]]), ETHERTYPE_IPV4);

        let frame = &eth[ETHER_HEADER_LEN..];
        assert_eq!(frame[0], 0x45, "IPv4, IHL 5");
        assert_eq!(frame[9], IPPROTO_UDP);
        assert_eq!(&frame[12..16], &[10, 0, 0, 2], "src is remote for inbound");
        assert_eq!(&frame[16..20], &[10, 0, 0, 1], "dst is local for inbound");
        assert_eq!(
            u16::from_be_bytes([frame[20], frame[21]]),
            40000,
            "src port"
        );
        assert_eq!(
            u16::from_be_bytes([frame[22], frame[23]]),
            12000,
            "dst port"
        );
        assert_eq!(&frame[28..], &payload, "rtp payload written verbatim");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn direction_selects_the_source() {
        let path = tmpfile("direction");
        let (local, remote) = addrs();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        w.write_packet(PcapDirection::Out, local, remote, &[1, 2, 3, 4])
            .unwrap();
        w.finish().unwrap();

        let bytes = read(&path);
        let eth = &bytes[40..];
        assert_eq!(&eth[0..6], &MAC_REMOTE, "dst mac is remote for outbound");
        assert_eq!(&eth[6..12], &MAC_LOCAL, "src mac is local for outbound");
        let frame = &eth[ETHER_HEADER_LEN..];
        assert_eq!(&frame[12..16], &[10, 0, 0, 1], "src is local for outbound");
        assert_eq!(&frame[16..20], &[10, 0, 0, 2], "dst is remote for outbound");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn substitutes_the_advertised_address_for_an_unspecified_bind() {
        // The RTP sockets always bind 0.0.0.0, so without this every capture
        // names one end "0.0.0.0" and reads as broken.
        let path = tmpfile("localaddr");
        let mut c = cfg(&path);
        c.local_address = Some("1.2.3.4".parse().unwrap());
        let local: SocketAddr = "0.0.0.0:12000".parse().unwrap();
        let remote: SocketAddr = "10.0.0.2:40000".parse().unwrap();

        let mut w = PcapWriter::create(&c).unwrap();
        w.write_packet(PcapDirection::In, local, remote, &[1, 2, 3, 4])
            .unwrap();
        w.write_packet(PcapDirection::Out, local, remote, &[1, 2, 3, 4])
            .unwrap();
        w.finish().unwrap();

        let bytes = read(&path);
        let ip = 24 + 16 + ETHER_HEADER_LEN;
        assert_eq!(
            &bytes[ip + 16..ip + 20],
            &[1, 2, 3, 4],
            "inbound dst is the advertised address"
        );

        let second = ip + IPV4_HEADER_LEN + UDP_HEADER_LEN + 4 + 16 + ETHER_HEADER_LEN;
        assert_eq!(
            &bytes[second + 12..second + 16],
            &[1, 2, 3, 4],
            "outbound src is the advertised address"
        );
        assert_eq!(
            &bytes[second + 16..second + 20],
            &[10, 0, 0, 2],
            "peer address is untouched"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keeps_a_genuinely_bound_address() {
        let path = tmpfile("boundaddr");
        let mut c = cfg(&path);
        c.local_address = Some("1.2.3.4".parse().unwrap());
        let (local, remote) = addrs();

        let mut w = PcapWriter::create(&c).unwrap();
        w.write_packet(PcapDirection::In, local, remote, &[1, 2, 3, 4])
            .unwrap();
        w.finish().unwrap();

        let bytes = read(&path);
        let ip = 24 + 16 + ETHER_HEADER_LEN;
        assert_eq!(
            &bytes[ip + 16..ip + 20],
            &[10, 0, 0, 1],
            "a real bind address is not overridden"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ip_header_checksum_is_correct() {
        // A correct one's-complement checksum makes the sum over the header
        // (including the checksum field) come to zero.
        let frame = build_frame(
            "192.168.1.1".parse().unwrap(),
            5060,
            "192.168.1.2".parse().unwrap(),
            5062,
            &[0xaa; 20],
            42,
            MAC_LOCAL,
            MAC_REMOTE,
        );
        let ip = &frame[ETHER_HEADER_LEN..ETHER_HEADER_LEN + IPV4_HEADER_LEN];
        assert_eq!(ipv4_checksum(ip), 0);
    }

    #[test]
    fn stops_at_max_bytes() {
        let path = tmpfile("maxbytes");
        let (local, remote) = addrs();
        let mut c = cfg(&path);
        c.max_bytes = Some(120);
        let mut w = PcapWriter::create(&c).unwrap();

        let payload = [0u8; 60];
        let mut accepted = 0;
        for _ in 0..10 {
            if w.write_packet(PcapDirection::In, local, remote, &payload)
                .unwrap()
            {
                accepted += 1;
            } else {
                break;
            }
        }
        assert!(accepted < 10, "capture should stop at the cap");
        assert!(
            w.bytes() <= 120,
            "must never exceed max_bytes, got {}",
            w.bytes()
        );
        // Hitting the cap finishes the capture, so further writes are refused.
        assert!(!w
            .write_packet(PcapDirection::In, local, remote, &payload)
            .unwrap());
        assert_eq!(w.stop_reason(), Some("maxsize"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stops_at_max_duration() {
        let path = tmpfile("maxduration");
        let (local, remote) = addrs();
        let mut c = cfg(&path);
        c.max_duration_ms = Some(0);
        let mut w = PcapWriter::create(&c).unwrap();

        // a zero duration is already over by the first packet
        assert!(!w
            .write_packet(PcapDirection::In, local, remote, &[0u8; 60])
            .unwrap());
        assert_eq!(w.stop_reason(), Some("maxduration"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_stop_reason_while_capturing() {
        let path = tmpfile("noreason");
        let (local, remote) = addrs();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        assert!(w
            .write_packet(PcapDirection::In, local, remote, &[0u8; 60])
            .unwrap());
        assert_eq!(w.stop_reason(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn counts_but_does_not_write_ipv6() {
        let path = tmpfile("ipv6");
        let local: SocketAddr = "[::1]:12000".parse().unwrap();
        let remote: SocketAddr = "[::2]:40000".parse().unwrap();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();

        assert!(w
            .write_packet(PcapDirection::In, local, remote, &[1, 2, 3])
            .unwrap());
        w.finish().unwrap();

        assert_eq!(w.packets(), 0);
        assert_eq!(
            w.skipped(),
            1,
            "skips are counted so a short file is explainable"
        );
        assert_eq!(
            read(&path).len(),
            24,
            "nothing written past the global header"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn finish_is_idempotent() {
        let path = tmpfile("idempotent");
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        w.finish().unwrap();
        w.finish().unwrap();
        // Writing after finish is a no-op, not an error — close and an
        // explicit finish can race on the actor.
        let (local, remote) = addrs();
        assert!(!w
            .write_packet(PcapDirection::In, local, remote, &[1])
            .unwrap());
        let _ = std::fs::remove_file(&path);
    }
}
