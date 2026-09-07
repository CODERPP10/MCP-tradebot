//! Client-side token-bucket limiter, sized under Kite's published limits
//! (~3 req/s general; 10 order req/s but 200/min; 3000 orders/day) — DESIGN.md §2.1.
//!
//! Hand-rolled rather than pulling in `governor` to keep the dependency surface
//! small (DESIGN.md rationale: "no npm supply chain"). One bucket guards all
//! calls; `KiteClient` awaits [`RateLimiter::acquire`] before every request.

use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// A refilling token bucket. `capacity` tokens allow a short burst; the bucket
/// refills at `refill_per_sec` tokens per second.
#[derive(Debug)]
pub struct RateLimiter {
    refill_per_sec: f64,
    capacity: f64,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    /// `refill_per_sec` steady-state requests/second, `capacity` maximum burst.
    /// Panics if either is non-positive.
    pub fn new(refill_per_sec: f64, capacity: u32) -> Self {
        assert!(refill_per_sec > 0.0, "refill_per_sec must be positive");
        assert!(capacity > 0, "capacity must be positive");
        let capacity = f64::from(capacity);
        Self {
            refill_per_sec,
            capacity,
            state: Mutex::new(State {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Conservative default: 2 req/s steady, burst of 1 (no bursting).
    pub fn conservative() -> Self {
        Self::new(2.0, 1)
    }

    /// Block until a token is available, then consume it.
    pub async fn acquire(&self) {
        let wait = {
            let mut st = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(st.last_refill).as_secs_f64();
            st.tokens = (st.tokens + elapsed * self.refill_per_sec).min(self.capacity);
            st.last_refill = now;

            if st.tokens >= 1.0 {
                st.tokens -= 1.0;
                Duration::ZERO
            } else {
                // Charge the token now (go negative) so concurrent callers queue
                // in order rather than all waking to race for the same token.
                let deficit = 1.0 - st.tokens;
                st.tokens -= 1.0;
                Duration::from_secs_f64(deficit / self.refill_per_sec)
            }
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn burst_then_throttle() {
        let rl = RateLimiter::new(10.0, 3);
        let start = Instant::now();
        // 3 burst tokens are immediate.
        for _ in 0..3 {
            rl.acquire().await;
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
        // 4th waits ~1/10s for a refill.
        rl.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn serialised_callers_queue_in_order() {
        let rl = RateLimiter::new(1.0, 1);
        let start = Instant::now();
        rl.acquire().await; // immediate
        rl.acquire().await; // +1s
        rl.acquire().await; // +2s
        assert!(start.elapsed() >= Duration::from_secs(2));
    }
}
