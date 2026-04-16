// Command surface — the messages the channel actor accepts from outside.
//
// Mirrors the JS-facing API in index.js: close / remote / mix / unmix / dtmf /
// echo / play / record / playrecord / direction. Each mutating command that
// JS awaits includes a oneshot ack so the caller's Promise can resolve.

use std::net::SocketAddr;

use tokio::sync::oneshot;

#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub addr: SocketAddr,
    pub payload_type: u8,
    pub ilbc_payload_type: Option<u8>,
    pub rfc2833_payload_type: Option<u8>,
    pub dtls: Option<RemoteDtls>,
}

#[derive(Debug, Clone)]
pub struct RemoteDtls {
    pub fingerprint: String,
    pub setup: DtlsSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsSetup { Active, Passive }

#[derive(Debug, Clone)]
pub struct SoundSoup {
    // soundsoup is a JSON description of a playlist (files + loops + regions).
    // Full structure will land in player.rs; keep opaque here so callers can
    // compile against the Command surface before player.rs exists.
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct RecordConfig {
    pub filename: String,
    pub num_channels: u16,
    pub max_duration_ms: Option<u64>,
    pub power_threshold: Option<i32>,
    pub power_packets: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct PlayRecordConfig {
    pub player: SoundSoup,
    pub recorder: RecordConfig,
    pub interrupt: bool,
    pub bargein_power: Option<i32>,
    pub bargein_packets: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Direction {
    pub send: bool,
    pub recv: bool,
}

impl Default for Direction {
    fn default() -> Self { Self { send: true, recv: true } }
}

// Ack types —
// `()` for fire-and-forget completion; richer types introduced as needed.
pub type Ack<T = ()> = oneshot::Sender<T>;

// ChannelId is a u64 rather than Uuid to keep the command channel cheap; the
// public UUID lives on the JS-facing handle, not on the internal wire.
pub type ChannelId = u64;

pub enum Command {
    Remote { cfg: RemoteConfig, ack: Ack },
    Play { cfg: SoundSoup, ack: Ack },
    Record { cfg: RecordConfig, ack: Ack },
    PlayRecord { cfg: PlayRecordConfig, ack: Ack },
    Dtmf { digits: String },
    Echo { enabled: bool },
    Direction(Direction),
    Mix { other_id: ChannelId, other_sender: tokio::sync::mpsc::Sender<Command>, ack: Ack },
    Unmix,
    /// Attach a mix-relay peer. `peer_handle` lets us push remote-updates to
    /// the peer when our own `Remote` changes. `peer_remote/pt/rfc2833_pt` are
    /// the peer's outbound targets at bind time — may still be None if the
    /// peer hasn't been configured yet, in which case the peer will push an
    /// update to us via `SetPeerRemote` once its `Remote` lands.
    BindMixPeer {
        peer_handle: tokio::sync::mpsc::Sender<Command>,
        peer_remote: Option<SocketAddr>,
        peer_pt: u8,
        peer_rfc2833_pt: u8,
    },
    /// Detach the mix-relay peer. Mirrors `Unmix` but scoped to the 2-chan
    /// relay (clears peer handle + outbound targets + emits mix/finished).
    UnbindMixPeer,
    /// Peer-to-peer update: the peer's own `Remote` changed; refresh our
    /// outbound targets accordingly. No mix start/finished event — the bind
    /// state hasn't changed, only the target.
    SetPeerRemote { remote: Option<SocketAddr>, pt: u8, rfc2833_pt: u8 },
    Close { reason: String },
}

// The public handle JS holds (indirectly, via a wrapper in channel/mod.rs).
// Cheap to clone — it's an mpsc Sender + a couple of ids.
#[derive(Clone)]
pub struct Handle {
    pub id: ChannelId,
    pub(crate) cmd: tokio::sync::mpsc::Sender<Command>,
}

impl Handle {
    pub async fn close(&self, reason: impl Into<String>) {
        let _ = self.cmd.send(Command::Close { reason: reason.into() }).await;
    }

    pub async fn dtmf(&self, digits: impl Into<String>) {
        let _ = self.cmd.send(Command::Dtmf { digits: digits.into() }).await;
    }

    pub async fn echo(&self, on: bool) {
        let _ = self.cmd.send(Command::Echo { enabled: on }).await;
    }

    pub async fn direction(&self, d: Direction) {
        let _ = self.cmd.send(Command::Direction(d)).await;
    }

    pub async fn remote(&self, cfg: RemoteConfig) -> Result<(), ()> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Remote { cfg, ack: tx }).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())
    }

    pub async fn play(&self, cfg: SoundSoup) -> Result<(), ()> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Play { cfg, ack: tx }).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())
    }

    pub async fn record(&self, cfg: RecordConfig) -> Result<(), ()> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Record { cfg, ack: tx }).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())
    }

    pub async fn play_record(&self, cfg: PlayRecordConfig) -> Result<(), ()> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::PlayRecord { cfg, ack: tx }).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())
    }
}
