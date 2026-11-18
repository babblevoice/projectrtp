// Packet capture — writes what a channel's sockets put on and take off the
// wire to a libpcap file.
//
// The sibling of `recorder.rs`: the recorder writes what the call *sounded*
// like, this writes what was actually *on the wire*. Same shape deliberately —
// armed by a command, closed on finish, a limit or channel close, and
// announced to JS with the finished path so babble-rtp can ship it off the
// node exactly as it already ships a wav.
//
// The taps sit at the socket boundary (`PcapTap`), on every socket a channel
// owns:
//
//   - inbound, in `recv_loop` and `rtcp_loop`, the moment `recv_from` returns:
//     stamped with its arrival time and the peer's real source address, before
//     the jitter buffer reorders or drops anything;
//   - outbound, at every `send_to`, after SRTP/SRTCP protection: RTP from the
//     tick, the mixer and the relay, RTCP reports and BYE, relay NACK and
//     PLI, STUN responses and the DTLS handshake.
//
// So the file is a record of the wire: RTP, RTCP, STUN and DTLS, audio and
// relay (video) legs alike, with the arrival jitter and bursts left in. On a
// secure leg the payload is ciphertext, so a capture holds no call audio and
// no DTMF digits. That costs network diagnosis nothing: SRTP encrypts only the
// payload, and the sequence number, timestamp, SSRC and payload type Wireshark
// analyses stay in the clear. Listening to a call is what `record()` is for.
//
// Why this belongs in projectrtp rather than a tcpdump sidecar:
//
//   - projectrtp owns the sockets, so capture is scoped to one channel by
//     construction. A port-filtered tcpdump has to guess which ports belong to
//     the call and races channel teardown/reuse.
//   - no NET_RAW capability, no extra container, nothing to co-schedule.
//
// The taps run on the media path, so they only copy the datagram into a
// bounded queue. A thread of the capture's own does the file I/O and owns the
// limits, so a slow disk never stalls a channel, and `maxduration` is kept
// even on a channel that has gone silent. A full queue drops the packet and
// counts it (`dropped`) rather than block.
//
// The captured bytes are the real datagrams, but the IP/UDP headers around
// them are synthesised from the addresses the socket reported — we never see
// the kernel's framing. That is what a capture consumer needs (Wireshark
// decodes the RTP, the flow reads correctly) without pretending to reproduce
// headers we do not have.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self as stdmpsc, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;

use super::actor::{Event, EventSink};

/// `maxsize` when the caller gives none. A capture is armed by an operator on
/// a live platform, so it is never unbounded. Both legs of a G.711 call, with
/// RTCP, come to about 100 MB an hour; this is the 50 MB babble-sip already
/// asks for.
pub const DEFAULT_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// `maxduration` when the caller gives none: an hour, as babble-sip asks for.
pub const DEFAULT_MAX_DURATION_MS: u64 = 60 * 60 * 1000;
/// Datagrams a capture may have queued for its writer. A channel moves about
/// 100 a second, so this is many seconds of a stalled disk before a packet is
/// dropped.
const QUEUE_DEPTH: usize = 4096;
/// The file holds whole datagrams - ICE usernames, RTCP CNAMEs, and on a leg
/// that is not encrypted the call audio itself - so only its owner may read it.
const FILE_MODE: u32 = 0o600;

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
/// the two addresses is the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcapDirection {
    /// Received from the peer, as it arrived.
    In,
    /// Sent to the peer, as it left - after SRTP/SRTCP protection.
    Out,
}

#[derive(Debug, Clone)]
pub struct PcapConfig {
    pub file: PathBuf,
    /// Stop capturing once the file reaches this size. The facade defaults it
    /// to `DEFAULT_MAX_BYTES`; `None` (unlimited) is for this module's tests.
    pub max_bytes: Option<u64>,
    /// Address to record as our own end. The RTP sockets bind to 0.0.0.0, so
    /// without this every capture shows one side as "0.0.0.0", which reads as
    /// a broken capture. Callers pass the address they advertised in SDP —
    /// the one the far end is actually sending to.
    pub local_address: Option<Ipv4Addr>,
    /// Stop capturing after this many milliseconds of wall clock, whether or
    /// not packets are still arriving. The facade defaults it to
    /// `DEFAULT_MAX_DURATION_MS`.
    pub max_duration_ms: Option<u64>,
}

/// Writes datagrams to a libpcap file, wrapped in synthesised IPv4/UDP
/// headers. Used only on a capture's writer thread (see `PcapTap`): its file
/// I/O blocks.
pub struct PcapWriter {
    file: BufWriter<File>,
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
    /// Create the file, readable by its owner only, and write the libpcap
    /// global header.
    pub fn create(cfg: &PcapConfig) -> io::Result<Self> {
        if let Some(parent) = cfg.file.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&cfg.file)?;
        // `mode` only applies to a file this call creates; one left over at the
        // same path keeps its old permissions unless they are set again.
        f.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))?;
        let mut file = BufWriter::new(f);
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

    /// Write one datagram, stamped now. See `write_packet_at`.
    #[cfg(test)]
    pub fn write_packet(
        &mut self,
        direction: PcapDirection,
        local: SocketAddr,
        remote: SocketAddr,
        payload: &[u8],
    ) -> io::Result<bool> {
        self.write_packet_at(direction, local, remote, payload, SystemTime::now())
    }

    /// Write one datagram, stamped `at` - when it arrived or was sent, not
    /// when the writer got to it. `local` is our socket's address and
    /// `remote` the peer's; `direction` picks which is the source.
    ///
    /// Returns `true` while the capture is still open. A `false` return means
    /// a limit was reached and the caller should finish the capture — it is
    /// not an error.
    pub fn write_packet_at(
        &mut self,
        direction: PcapDirection,
        local: SocketAddr,
        remote: SocketAddr,
        payload: &[u8],
        at: SystemTime,
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

        let now = at.duration_since(UNIX_EPOCH).unwrap_or_default();
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

/// One datagram on its way to a capture's writer thread.
struct Captured {
    at: SystemTime,
    direction: PcapDirection,
    local: SocketAddr,
    remote: SocketAddr,
    data: Vec<u8>,
}

/// Why a capture was finished from outside, and who is waiting for its event.
type FinishRequest = (&'static str, Option<tokio::sync::oneshot::Sender<()>>);

/// What a running capture shares with its writer thread besides the queue.
#[derive(Default)]
struct Control {
    /// Datagrams the taps dropped because the queue was full.
    dropped: AtomicU64,
    /// Set by `PcapTap::finish` before it drops the queue's sender: why the
    /// capture is closing, and who is waiting for its event to be posted.
    finish: PLMutex<Option<FinishRequest>>,
}

/// The capture currently armed on a channel.
struct Armed {
    /// Tells this capture apart from one that replaced it, so a writer that
    /// stops on its own disarms only itself.
    id: u64,
    tx: SyncSender<Captured>,
    control: Arc<Control>,
}

#[derive(Default)]
struct TapInner {
    /// Read without the lock on every datagram, so an unarmed channel pays
    /// one relaxed load per packet and nothing else.
    armed: AtomicBool,
    current: PLMutex<Option<Armed>>,
    next_id: AtomicU64,
}

/// A channel's capture point. Every task that owns one of the channel's
/// sockets - the tick and mixer through `ChannelState`, `recv_loop`,
/// `rtcp_loop`, the relay send task, the DTLS transport - holds a clone, and
/// calls `received` for each datagram it reads and `send_to` for each it
/// sends. Cheap to clone; does nothing while no capture is armed.
#[derive(Clone, Default)]
pub struct PcapTap(Arc<TapInner>);

impl std::fmt::Debug for PcapTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcapTap")
            .field("armed", &self.is_armed())
            .finish()
    }
}

impl PcapTap {
    pub fn is_armed(&self) -> bool {
        self.0.armed.load(Ordering::Relaxed)
    }

    /// Record a datagram `sock` just received from `from`. Call it the moment
    /// `recv_from` returns, before anything looks at the packet.
    pub fn received(&self, sock: &UdpSocket, from: SocketAddr, data: &[u8]) {
        if !self.is_armed() {
            return;
        }
        let at = SystemTime::now();
        if let Ok(local) = sock.local_addr() {
            self.push(at, PcapDirection::In, local, from, data);
        }
    }

    /// Send `data` to `to` on `sock`, and record it if it went. Used in place
    /// of `sock.send_to` everywhere a channel sends, with the bytes exactly
    /// as they go on the wire.
    pub async fn send_to(
        &self,
        sock: &UdpSocket,
        data: &[u8],
        to: SocketAddr,
    ) -> io::Result<usize> {
        let sent = sock.send_to(data, to).await;
        if sent.is_ok() && self.is_armed() {
            let at = SystemTime::now();
            if let Ok(local) = sock.local_addr() {
                self.push(at, PcapDirection::Out, local, to, data);
            }
        }
        sent
    }

    fn push(
        &self,
        at: SystemTime,
        direction: PcapDirection,
        local: SocketAddr,
        remote: SocketAddr,
        data: &[u8],
    ) {
        let current = self.0.current.lock();
        let Some(armed) = current.as_ref() else {
            return;
        };
        let pkt = Captured {
            at,
            direction,
            local,
            remote,
            data: data.to_vec(),
        };
        match armed.tx.try_send(pkt) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                armed.control.dropped.fetch_add(1, Ordering::Relaxed);
            }
            // The writer stopped on a limit or an error and is disarming.
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Arm a capture, replacing any already running (which finishes with
    /// `replaced`). The file is opened on the capture's own thread, so a
    /// path that cannot be opened is reported by an `open-failed` event,
    /// never by failing the call.
    pub fn start(&self, cfg: PcapConfig, events: Arc<dyn EventSink>) {
        self.finish("replaced");

        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = stdmpsc::sync_channel(QUEUE_DEPTH);
        let control = Arc::new(Control::default());
        *self.0.current.lock() = Some(Armed {
            id,
            tx,
            control: control.clone(),
        });
        self.0.armed.store(true, Ordering::Relaxed);

        let tap = Arc::downgrade(&self.0);
        let file = cfg.file.to_string_lossy().into_owned();
        let thread_events = events.clone();
        // A thread rather than spawn_blocking: it lives as long as the
        // capture, which would hold a blocking-pool thread just the same.
        let spawned = std::thread::Builder::new()
            .name("pcap-writer".into())
            .spawn(move || writer_thread(cfg, rx, control, thread_events, tap, id));
        if let Err(e) = spawned {
            disarm(&Arc::downgrade(&self.0), id);
            events.post(Event::Pcap {
                reason: "open-failed".into(),
                error: Some(e.to_string()),
                file,
                filesize: 0,
                packets: 0,
                skipped: 0,
                dropped: 0,
            });
        }
    }

    /// Close the running capture, if there is one, with `reason`. Its writer
    /// writes out whatever is still queued, then posts the `pcap` event; the
    /// receiver resolves once it has. `None` when nothing was armed - or the
    /// capture had already stopped on its own, and said so.
    pub fn finish(&self, reason: &'static str) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let armed = {
            let mut current = self.0.current.lock();
            self.0.armed.store(false, Ordering::Relaxed);
            current.take()
        }?;
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        *armed.control.finish.lock() = Some((reason, Some(done_tx)));
        // Dropping the sender is the signal: the writer drains the queue, then
        // sees the disconnect and reads why from `control`.
        drop(armed);
        Some(done_rx)
    }
}

/// Disarm capture `id`, if it is still the one armed - a writer that stopped
/// on a limit must not disarm a capture that has since replaced it.
fn disarm(tap: &Weak<TapInner>, id: u64) {
    let Some(tap) = tap.upgrade() else {
        return;
    };
    let mut current = tap.current.lock();
    if current.as_ref().is_some_and(|a| a.id == id) {
        tap.armed.store(false, Ordering::Relaxed);
        *current = None;
    }
}

/// A capture's own thread: opens the file, writes what the taps queue, keeps
/// the limits, and posts the one `pcap` event the capture ends with.
fn writer_thread(
    cfg: PcapConfig,
    rx: stdmpsc::Receiver<Captured>,
    control: Arc<Control>,
    events: Arc<dyn EventSink>,
    tap: Weak<TapInner>,
    id: u64,
) {
    let file = cfg.file.to_string_lossy().into_owned();
    let deadline = cfg
        .max_duration_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));

    let (stopped, error, mut writer) = match PcapWriter::create(&cfg) {
        Ok(mut w) => {
            let (stopped, error) = write_until_stopped(&mut w, &rx, deadline);
            (stopped, error, Some(w))
        }
        Err(e) => (Some("open-failed"), Some(e.to_string()), None),
    };
    // Stopped on its own (a limit, or an error): stop the taps queueing for
    // it now, before the file is flushed.
    if stopped.is_some() {
        disarm(&tap, id);
    }
    drop(rx);

    let mut stopped = stopped;
    let mut error = error;
    if let Some(w) = writer.as_mut() {
        if let Err(e) = w.finish() {
            if error.is_none() {
                stopped = Some("write-failed");
                error = Some(e.to_string());
            }
        }
    }

    let requested = control.finish.lock().take();
    let reason = stopped
        .or(requested.as_ref().map(|(r, _)| *r))
        .unwrap_or("channelclosed");
    events.post(Event::Pcap {
        reason: reason.into(),
        error,
        file,
        filesize: writer.as_ref().map_or(0, |w| w.bytes()),
        packets: writer.as_ref().map_or(0, |w| w.packets()),
        skipped: writer.as_ref().map_or(0, |w| w.skipped()),
        dropped: control.dropped.load(Ordering::Relaxed),
    });
    if let Some((_, Some(done))) = requested {
        let _ = done.send(());
    }
}

/// Write queued datagrams until the capture is finished from outside (the
/// queue disconnects: `None`), or stops itself on a limit or an error.
fn write_until_stopped(
    w: &mut PcapWriter,
    rx: &stdmpsc::Receiver<Captured>,
    deadline: Option<Instant>,
) -> (Option<&'static str>, Option<String>) {
    loop {
        let next = match deadline {
            Some(d) => {
                let now = Instant::now();
                if now >= d {
                    return (Some("maxduration"), None);
                }
                rx.recv_timeout(d - now)
            }
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(p) => match w.write_packet_at(p.direction, p.local, p.remote, &p.data, p.at) {
                Ok(true) => {}
                Ok(false) => return (Some(w.stop_reason().unwrap_or("maxsize")), None),
                Err(e) => return (Some("write-failed"), Some(e.to_string())),
            },
            // The deadline passed with nothing arriving: a silent channel
            // still stops on time.
            Err(RecvTimeoutError::Timeout) => return (Some("maxduration"), None),
            Err(RecvTimeoutError::Disconnected) => return (None, None),
        }
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
    use std::path::Path;

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

    // ---- the tap and its writer thread ----

    struct Sink(tokio::sync::mpsc::UnboundedSender<Event>);
    impl EventSink for Sink {
        fn post(&self, ev: Event) {
            let _ = self.0.send(ev);
        }
    }

    fn sink() -> (
        Arc<dyn EventSink>,
        tokio::sync::mpsc::UnboundedReceiver<Event>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Arc::new(Sink(tx)), rx)
    }

    async fn next_event(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Event {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("a pcap event within 3s")
            .expect("sink open")
    }

    async fn pair() -> (UdpSocket, UdpSocket) {
        let a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        (a, b)
    }

    /// Every record in a capture file: (secs, usecs, src ip:port, dst ip:port, udp payload).
    type Record = (u32, u32, SocketAddr, SocketAddr, Vec<u8>);
    fn records(path: &Path) -> Vec<Record> {
        let bytes = read(path);
        let mut out = Vec::new();
        let mut at = 24;
        while at + 16 <= bytes.len() {
            let secs = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let usecs = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            let incl = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
            let frame = &bytes[at + 16..at + 16 + incl];
            let ip = &frame[ETHER_HEADER_LEN..];
            let udp = &ip[IPV4_HEADER_LEN..];
            let addr = |o: &[u8], p: &[u8]| {
                SocketAddr::new(
                    Ipv4Addr::new(o[0], o[1], o[2], o[3]).into(),
                    u16::from_be_bytes([p[0], p[1]]),
                )
            };
            out.push((
                secs,
                usecs,
                addr(&ip[12..16], &udp[0..2]),
                addr(&ip[16..20], &udp[2..4]),
                udp[UDP_HEADER_LEN..].to_vec(),
            ));
            at += 16 + incl;
        }
        out
    }

    #[tokio::test]
    async fn an_unarmed_tap_sends_and_records_nothing() {
        let (a, b) = pair().await;
        let tap = PcapTap::default();
        assert!(!tap.is_armed());
        tap.send_to(&a, &[1, 2, 3], b.local_addr().unwrap())
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        let (n, from) = b.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], &[1, 2, 3], "the datagram still goes");
        tap.received(&b, from, &buf[..n]);
        assert!(tap.finish("requested").is_none(), "nothing was armed");
    }

    #[tokio::test]
    async fn records_both_directions_with_the_real_addresses() {
        let path = tmpfile("tap");
        let (ours, theirs) = pair().await;
        let (events, mut rx) = sink();
        let tap = PcapTap::default();
        tap.start(cfg(&path), events);
        assert!(tap.is_armed());

        // what we send, as it leaves
        tap.send_to(&ours, b"out", theirs.local_addr().unwrap())
            .await
            .unwrap();
        // what arrives, from wherever it really came from
        theirs
            .send_to(b"in", ours.local_addr().unwrap())
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        let (n, from) = ours.recv_from(&mut buf).await.unwrap();
        tap.received(&ours, from, &buf[..n]);

        let done = tap.finish("requested").expect("armed");
        tokio::time::timeout(Duration::from_secs(3), done)
            .await
            .unwrap()
            .unwrap();
        assert!(!tap.is_armed());
        match next_event(&mut rx).await {
            Event::Pcap {
                reason,
                error,
                packets,
                dropped,
                ..
            } => {
                assert_eq!(reason, "requested");
                assert_eq!(error, None);
                assert_eq!(packets, 2);
                assert_eq!(dropped, 0);
            }
            other => panic!("unexpected event {other:?}"),
        }

        let recs = records(&path);
        assert_eq!(recs.len(), 2);
        let (o, t) = (ours.local_addr().unwrap(), theirs.local_addr().unwrap());
        assert_eq!(
            (recs[0].2, recs[0].3, recs[0].4.as_slice()),
            (o, t, &b"out"[..])
        );
        assert_eq!(
            (recs[1].2, recs[1].3, recs[1].4.as_slice()),
            (t, o, &b"in"[..])
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stamps_a_record_with_when_it_happened_not_when_it_was_written() {
        let path = tmpfile("stamp");
        let (local, remote) = addrs();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        let at = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_000);
        w.write_packet_at(PcapDirection::In, local, remote, &[1], at)
            .unwrap();
        w.finish().unwrap();
        let recs = records(&path);
        assert_eq!((recs[0].0, recs[0].1), (1_700_000_000, 123_456));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_silent_capture_still_stops_at_maxduration() {
        let path = tmpfile("silent");
        let (events, mut rx) = sink();
        let tap = PcapTap::default();
        let mut c = cfg(&path);
        c.max_duration_ms = Some(100);
        tap.start(c, events);

        // no packet ever arrives
        match next_event(&mut rx).await {
            Event::Pcap {
                reason, packets, ..
            } => {
                assert_eq!(reason, "maxduration");
                assert_eq!(packets, 0);
            }
            other => panic!("unexpected event {other:?}"),
        }
        assert!(!tap.is_armed(), "a capture that stopped disarms itself");
        assert!(
            tap.finish("requested").is_none(),
            "and is not reported twice"
        );
        assert_eq!(read(&path).len(), 24, "a valid, empty capture");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn stopping_at_maxsize_disarms_and_reports_once() {
        let path = tmpfile("tapmax");
        let (ours, theirs) = pair().await;
        let (events, mut rx) = sink();
        let tap = PcapTap::default();
        let mut c = cfg(&path);
        c.max_bytes = Some(200);
        tap.start(c, events);

        for _ in 0..20 {
            let _ = tap
                .send_to(&ours, &[0u8; 60], theirs.local_addr().unwrap())
                .await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        match next_event(&mut rx).await {
            Event::Pcap {
                reason,
                filesize,
                packets,
                ..
            } => {
                assert_eq!(reason, "maxsize");
                assert!(filesize <= 200);
                assert!(packets > 0);
            }
            other => panic!("unexpected event {other:?}"),
        }
        assert!(!tap.is_armed());
        assert!(tap.finish("requested").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_opened_is_a_fixed_reason_and_an_error() {
        let (events, mut rx) = sink();
        let tap = PcapTap::default();
        tap.start(cfg(Path::new("/proc/nope/cannot.pcap")), events);
        match next_event(&mut rx).await {
            Event::Pcap {
                reason,
                error,
                packets,
                ..
            } => {
                assert_eq!(reason, "open-failed", "no OS text in the reason");
                assert!(error.is_some_and(|e| !e.is_empty()));
                assert_eq!(packets, 0);
            }
            other => panic!("unexpected event {other:?}"),
        }
        assert!(!tap.is_armed());
    }

    #[tokio::test]
    async fn arming_again_replaces_the_running_capture() {
        let (first, second) = (tmpfile("first"), tmpfile("second"));
        let (events, mut rx) = sink();
        let tap = PcapTap::default();
        tap.start(cfg(&first), events.clone());
        tap.start(cfg(&second), events);
        match next_event(&mut rx).await {
            Event::Pcap { reason, file, .. } => {
                assert_eq!(reason, "replaced");
                assert_eq!(file, first.to_string_lossy());
            }
            other => panic!("unexpected event {other:?}"),
        }
        assert!(tap.is_armed(), "the new capture is running");
        let done = tap.finish("channelclosed").unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(3), done).await;
        match next_event(&mut rx).await {
            Event::Pcap { reason, file, .. } => {
                assert_eq!(reason, "channelclosed");
                assert_eq!(file, second.to_string_lossy());
            }
            other => panic!("unexpected event {other:?}"),
        }
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        let path = tmpfile("mode");
        // even when a file with looser permissions is already there
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut w = PcapWriter::create(&cfg(&path)).unwrap();
        w.finish().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }
}
