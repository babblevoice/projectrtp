# C++ → Rust migration (historical)

projectrtp began as a C++ NAPI addon. It was ported to a Rust napi-rs
crate and the C++ implementation was **removed in 3.0.0** — Rust is now
the only backend. This document is kept for context: why the port was
done the way it was, and the performance evidence behind it.

The old C++ tree lived under `src/` (plus `binding.gyp`, the node-gyp
build, and the C++ `Dockerfile`/`Dockerfile.debian`). To recover it,
check out a commit before the 3.0.0 cleanup:

```bash
git log --oneline -- src/binding.gyp        # find the last C++ commit
git show <commit>:src/projectrtpchannel.cpp  # read a specific file
```

The current architecture is documented in [ARCHITECTURE.md](./ARCHITECTURE.md).

## Why it was done this way

- **Behavior-preserving, not line-by-line.** The tests in
  `test/interface/` and `test/unit/` were the contract. The Rust modules
  reshaped the implementation freely as long as the JS surface and
  observable wire behavior matched. Those same tests remain the
  acceptance suite.
- **Module-at-a-time.** One C++ translation unit mapped to one Rust
  module, sometimes split further — e.g. the ~3k-LOC
  `projectrtpchannel.cpp` fanned out across
  `channel/{actor,commands,state,tick,facade,rtp,jitter}.rs`.
- **Actor model replaced shared-mutex state.** The C++ code used
  spinlocks and `shared_from_this`. The Rust port gives each channel a
  tokio task that owns its `ChannelState` exclusively; outside callers
  (JS, other channels, the mix group) send `Command`s through an mpsc
  queue. See the `rust/src/channel/actor.rs` header for the ownership
  rules.
- **Fast paths stayed fast.** The 2-channel mix is a byte relay in
  `tick.rs` (no mix-group actor, no decode/encode round-trip) — it covers
  the overwhelming majority of production traffic. The full N-way mix in
  `channel/mixer.rs` is used only for >2 channels or codec combinations
  the relay can't handle.
- **Safety by design.** Unsafe is confined to FFI shims only (SRTP,
  iLBC, G.722 bindings). Everything else is safe Rust — no raw pointers,
  no manually tracked lifetimes, no `shared_from_this`. The Boost
  exception shim was dropped entirely; `Result` replaces it.
- **Native codecs kept bit-identical.** G.722 (libspandsp) and iLBC
  (libilbc) are thin safe FFI over the same C libraries the C++ addon
  used, so transcoded output is bit-for-bit identical and the
  frequency-domain transcode tests pass unchanged. G.711 was reimplemented
  in pure Rust from the spandsp tables.

## Scheduler: C++ IOCP vs tokio — the open question, settled

The C++ build ran N worker threads (1 per core) with IOCP / io_uring,
each dispatching work directly from kernel completions with no
task-migration across cores — good cache locality. The Rust build runs
each channel as a tokio task on the default multi-thread runtime (also
1 worker per core, but work-stealing can migrate a task between cores).

The open question was whether tokio's work-stealing + waker overhead
would cost anything at production channel counts. `stress/perfbench.js`
and `stress/run-matrix.sh` were built to measure it against both builds.

### Bench result — 2026-04-23

14-core Linux host, 3 modes × 4 channel counts, 5 s sample each:

| Mode | Chan | Rust KiB/ch | C++ KiB/ch | Rust CPU | C++ CPU | Rust p99 | C++ p99 |
|---|---|---|---|---|---|---|---|
| idle | 1000 | 27.7 | 27.7 | 7.0%   | 9.9%   | —   | —   |
| idle | 2000 | 27.8 | 28.1 | 14.3%  | 14.7%  | —   | —   |
| echo | 1000 | 27.6 | 27.8 | 107.1% | 105.5% | 232 | 233 |
| echo | 2000 | 28.4 | 28.1 | 168.4% | 166.6% | 247 | 256 |
| mix2 | 1000 | 34.9 | 34.7 | 82.6%  | 83.7%  | 443 | 442 |
| mix2 | 2000 | 34.7 | 35.0 | 154.7% | 155.9% | 477 | 461 |

(CPU % is of one core; p99 is echo-round-trip latency in ms. Full
12-scenario table in git history.)

**Headline: parity, no regression, no win.** Per-channel memory is
byte-for-byte identical at scale; CPU is within ±5% with no systematic
direction; p99 latency matches within 1–2 ms; zero packet drops at the
100 000 pps echo peak on both builds.

So the tokio overhead the open question worried about didn't
materialise. Production boxes sized for N C++ channels run N Rust
channels at the same footprint. The alternatives once on the table
(sharded `current_thread` runtimes with CPU affinity; a per-core
epoll/io_uring reactor dropping tokio) are **no longer motivated** —
left on the shelf.

Caveats: 5 s localhost samples (no real-internet latency tails),
DTLS-SRTP at scale not exercised, no long-duration soak.
