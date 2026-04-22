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
    /// ICE password of the *remote* agent. Per RFC 8445 §7.1.1, the remote
    /// uses our local icepwd to sign STUN Binding Requests it sends us; our
    /// Binding Responses are signed with the same key. This field is kept
    /// for completeness (and for future outgoing connectivity checks) even
    /// though the inbound request-integrity check uses `state.local_icepwd`.
    pub icepwd: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteDtls {
    #[allow(dead_code)]
    pub fingerprint: String,
    pub setup: DtlsSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DtlsSetup { Active, Passive }

/// The play config carried on `Command::Play`. Alias for the parsed spec in
/// `channel/player.rs` — commands used to carry a raw JSON string, but the
/// facade now parses at the JS boundary so actors never see unparsed input.
pub type SoundSoup = super::player::SoundSoupSpec;

/// Same struct the recorder subsystem uses — the facade does all JS→Rust
/// parsing, so Command payloads are already fully-typed.
pub type RecordConfig = super::recorder::RecorderConfig;

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
    /// `channel.record({ finish: true, file: "..." })` — finalize the named
    /// recorder (multiple can coexist, keyed by file path).
    RecordFinish { file: std::path::PathBuf },
    /// `channel.record({ pause: true, file: "..." })` — pause/resume toggle.
    /// `resume=true` flips from Paused to Active; `resume=false` pauses.
    RecordSetPaused { file: std::path::PathBuf, paused: bool },
    PlayRecord { cfg: PlayRecordConfig, ack: Ack },
    /// `channel.createReadStream({...})` — register a new audio reader.
    /// The id is generated facade-side (via `audio_reader::next_reader_id`)
    /// and passed in so JS can refer to this reader when it needs to
    /// destroy it. The mpsc sender is also facade-side; the forwarder task
    /// holding the matching receiver runs alongside.
    CreateReadStream {
        id: u64,
        cfg: super::audio_reader::ReaderConfig,
        sender: tokio::sync::mpsc::Sender<Vec<u8>>,
    },
    /// `readable.destroy()` from JS. Removes the reader with the given
    /// id — dropping it closes the mpsc and ends the forwarder task.
    DestroyReadStream { id: u64 },
    Dtmf { digits: String },
    Echo { enabled: bool },
    Direction(Direction),
    /// Migrate this channel's state + subs into a mix group actor. The actor
    /// transitions from Local to Mixed mode; subsequent commands are
    /// forwarded into the mixer. `ack` fires once the migration completes.
    EnterMix { mix: super::mixer::MixHandle, ack: Ack },
    /// Migrate state + subs back out of the mix group. The actor returns to
    /// Local mode and resumes its own ticker. Used for `channel.unmix()`.
    LeaveMix { ack: Ack },
    Close { reason: String },
}

// The public handle JS holds (indirectly, via a wrapper in channel/mod.rs).
// Cheap to clone — it's an mpsc Sender + a couple of ids.
#[derive(Clone)]
pub struct Handle {
    pub id: ChannelId,
    pub(crate) cmd: tokio::sync::mpsc::Sender<Command>,
}

#[allow(dead_code)]
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
