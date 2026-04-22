// Channel — per-channel actor + mixer group actor.
//
// See /home/nick/.claude/plans/lively-whistling-honey.md and the design note
// in the task-7 conversation for the overall shape.
//
// Port-order within Task #7:
//   1. rtp            — packet data type (done)
//   2. jitter         — reorder buffer
//   3. state          — ChannelState struct
//   4. commands       — Command enum + Handle
//   5. dtls_session   — gnutls wrapper
//   6. srtp_ctx       — libsrtp2 wrapper
//   7. tick           — the per-tick pipeline
//   8. player / recorder / dtmf — media subsystems
//   9. actor          — the tokio task
//  10. mixer          — mix group actor
//  11. #[napi] facade — openchannel()

pub mod actor;
pub mod audio_reader;
pub mod commands;
pub mod dtls_session;
pub mod dtmf;
pub mod facade;
pub mod jitter;
pub mod mixer;
pub mod player;
pub mod recorder;
pub mod recv_loop;
pub mod rtp;
pub mod srtp_ctx;
pub mod state;
pub mod tick;
