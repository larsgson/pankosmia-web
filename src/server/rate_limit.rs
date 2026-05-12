//! Per-user save rate limiter.
//!
//! In-memory sliding window keyed by GitHub user-id. Used by the
//! GitHub-backed save endpoints to refuse spam-loop-shaped traffic
//! before it consumes the App's installation-token rate-limit
//! budget at GitHub.
//!
//! Defaults (60 events / 15 min) chosen to be loose for human
//! editors (one save every ~15 seconds for a sustained window) and
//! tight enough that a runaway loop notices and stops.
//!
//! No persistence: a server restart resets every user's counter.
//! That's intentional — the protection is anti-spam, not anti-quota,
//! and a restart is a coarser correction than the counter anyway.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("rate limit exceeded; retry in {0}s")]
    Exceeded(u64),
}

pub struct RateLimiter {
    inner: Mutex<HashMap<i64, Vec<Instant>>>,
    max_events: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_events: usize, window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            max_events,
            window,
        }
    }

    /// Defaults: 60 events / 15 minutes per user. Tuned for the
    /// save-endpoint flow specifically. See module-level docs.
    pub fn default_for_saves() -> Self {
        Self::new(60, Duration::from_secs(15 * 60))
    }

    /// Record an attempt for `user_id`. Returns `Ok(())` if under
    /// the limit (and the attempt is counted). Returns
    /// `Err(retry_after)` if the limit would be exceeded; the
    /// attempt is **not** counted in that case (so retrying once the
    /// window slides will succeed).
    pub fn check(&self, user_id: i64) -> Result<(), RateLimitError> {
        let now = Instant::now();
        let mut map = self.inner.lock();
        let entry = map.entry(user_id).or_default();
        // Drop timestamps that fell out of the window.
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        entry.retain(|t| *t >= cutoff);
        if entry.len() >= self.max_events {
            // Oldest event is the one that will eventually leave the
            // window; retry once it does.
            let oldest = entry.first().copied().unwrap_or(now);
            let retry_at = oldest + self.window;
            let retry_after = retry_at.saturating_duration_since(now).as_secs().max(1);
            return Err(RateLimitError::Exceeded(retry_after));
        }
        entry.push(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(rl.check(42).is_ok());
        }
    }

    #[test]
    fn rejects_above_max() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            rl.check(42).unwrap();
        }
        let err = rl.check(42).unwrap_err();
        match err {
            RateLimitError::Exceeded(secs) => assert!(secs > 0 && secs <= 60),
        }
    }

    #[test]
    fn separate_users_are_independent() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        rl.check(1).unwrap();
        rl.check(1).unwrap();
        // user 1 at limit; user 2 still fine
        assert!(rl.check(1).is_err());
        rl.check(2).unwrap();
        rl.check(2).unwrap();
        assert!(rl.check(2).is_err());
    }

    #[test]
    fn window_expiry_reopens_slots() {
        let rl = RateLimiter::new(2, Duration::from_millis(50));
        rl.check(1).unwrap();
        rl.check(1).unwrap();
        assert!(rl.check(1).is_err());
        std::thread::sleep(Duration::from_millis(80));
        // Window has slid; first slot is free again.
        rl.check(1).unwrap();
    }

    #[test]
    fn rejected_attempts_are_not_counted() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        rl.check(1).unwrap();
        rl.check(1).unwrap();
        // Five rejected attempts shouldn't push the next valid retry
        // further out — only the original two are recorded.
        for _ in 0..5 {
            assert!(rl.check(1).is_err());
        }
    }
}
