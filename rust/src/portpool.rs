// Even-port pool — port of the `availableports` queue in
// src/projectrtpchannel.cpp.
//
// At run() time, JS can pass `{ ports: { start, end } }`. We fill a FIFO
// with every even port in `[start, end)` and hand them out on openchannel(),
// reclaiming on channel close. This matches the C++ model: known-free ports
// inside a managed range, no ephemeral-allocation races, a hard ceiling on
// concurrent channels (end-start)/2.
//
// When `init` has not been called, `acquire()` returns None and the facade
// falls back to the ephemeral-bind-with-retry path. That keeps tests that
// call `projectrtp.run()` with no args working without a pool.

use std::collections::VecDeque;
use std::sync::Mutex;

struct PortPool {
    available: VecDeque<u16>,
    #[allow(dead_code)]
    total: u32,
}

static POOL: Mutex<Option<PortPool>> = Mutex::new(None);

/// Initialize (or re-initialize) the pool with even ports in [start, end).
/// Odd `start` is bumped to the next even port. Must be called before any
/// openchannel() that relies on the pool; safe to call multiple times (the
/// pool is replaced wholesale).
pub fn init(start: u16, end: u16) {
    let mut p = start;
    if p % 2 != 0 {
        p = p.saturating_add(1);
    }
    let mut q = VecDeque::new();
    while p + 1 < end {
        q.push_back(p);
        p = match p.checked_add(2) {
            Some(v) => v,
            None => break,
        };
    }
    let total = q.len() as u32;
    *POOL.lock().unwrap() = Some(PortPool {
        available: q,
        total,
    });
}

pub fn is_initialized() -> bool {
    POOL.lock().unwrap().is_some()
}

/// Take the next even port. Returns None when the pool is uninitialized or
/// exhausted — the caller distinguishes the two cases via `is_initialized`.
pub fn acquire() -> Option<u16> {
    POOL.lock()
        .unwrap()
        .as_mut()
        .and_then(|p| p.available.pop_front())
}

/// Return a port to the pool — no-op if the pool is uninitialized.
pub fn release(port: u16) {
    if let Some(p) = POOL.lock().unwrap().as_mut() {
        p.available.push_back(port);
    }
}

pub fn available_count() -> u32 {
    POOL.lock()
        .unwrap()
        .as_ref()
        .map(|p| p.available.len() as u32)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn total_count() -> u32 {
    POOL.lock().unwrap().as_ref().map(|p| p.total).unwrap_or(0)
}

/// Guard that releases the port on drop. Stored on ChannelState so the port
/// returns to the pool when the actor ends (and its state drops) — same
/// lifetime as the sockets bound to the port.
pub struct PortReservation {
    port: u16,
}

impl PortReservation {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for PortReservation {
    fn drop(&mut self) {
        release(self.port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_populates_only_even_ports() {
        init(10_000, 10_010);
        let mut acquired = Vec::new();
        while let Some(p) = acquire() {
            acquired.push(p);
        }
        assert_eq!(acquired, vec![10_000, 10_002, 10_004, 10_006, 10_008]);
    }

    #[test]
    fn reservation_drop_returns_port() {
        init(20_000, 20_004);
        assert_eq!(available_count(), 2);
        let r = PortReservation::new(acquire().unwrap());
        assert_eq!(available_count(), 1);
        drop(r);
        assert_eq!(available_count(), 2);
    }
}
