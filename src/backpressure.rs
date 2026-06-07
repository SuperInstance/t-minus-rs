//! Token bucket + leaky bucket rate limiters that respect deadlines.
//! Configure burst, refill rate, and hard deadline cutoff.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Token bucket rate limiter.
pub struct TokenBucket {
    state: Arc<Mutex<TokenBucketState>>,
}

struct TokenBucketState {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
    deadline: Option<Instant>,
}

impl TokenBucket {
    /// Create a new token bucket.
    /// * `max_tokens` - maximum burst capacity
    /// * `refill_rate` - tokens added per second
    /// * `deadline` - optional hard deadline after which all requests are denied
    pub fn new(max_tokens: f64, refill_rate: f64, deadline: Option<Duration>) -> Self {
        TokenBucket {
            state: Arc::new(Mutex::new(TokenBucketState {
                tokens: max_tokens,
                max_tokens,
                refill_rate,
                last_refill: Instant::now(),
                deadline: deadline.map(|d| Instant::now() + d),
            })),
        }
    }

    /// Try to acquire `count` tokens. Returns true if allowed.
    pub fn try_acquire(&self, count: f64) -> bool {
        let mut state = self.state.lock().unwrap();

        // Check deadline
        if let Some(dl) = state.deadline {
            if Instant::now() >= dl {
                return false;
            }
        }

        // Refill
        let now = Instant::now();
        let elapsed = (now - state.last_refill).as_secs_f64();
        state.tokens = (state.tokens + elapsed * state.refill_rate).min(state.max_tokens);
        state.last_refill = now;

        if state.tokens >= count {
            state.tokens -= count;
            true
        } else {
            false
        }
    }

    /// Get current number of available tokens.
    pub fn available_tokens(&self) -> f64 {
        let state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = (now - state.last_refill).as_secs_f64();
        (state.tokens + elapsed * state.refill_rate).min(state.max_tokens)
    }

    /// Check if the deadline has passed.
    pub fn is_expired(&self) -> bool {
        let state = self.state.lock().unwrap();
        match state.deadline {
            Some(dl) => Instant::now() >= dl,
            None => false,
        }
    }

    /// Reset to full capacity.
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.tokens = state.max_tokens;
        state.last_refill = Instant::now();
    }
}

/// Leaky bucket rate limiter.
pub struct LeakyBucket {
    state: Arc<Mutex<LeakyBucketState>>,
}

struct LeakyBucketState {
    queue_level: f64,
    capacity: f64,
    drain_rate: f64, // tokens drained per second
    last_drain: Instant,
    deadline: Option<Instant>,
}

impl LeakyBucket {
    /// Create a new leaky bucket.
    /// * `capacity` - maximum queue level before rejecting
    /// * `drain_rate` - tokens drained per second
    /// * `deadline` - optional hard deadline
    pub fn new(capacity: f64, drain_rate: f64, deadline: Option<Duration>) -> Self {
        LeakyBucket {
            state: Arc::new(Mutex::new(LeakyBucketState {
                queue_level: 0.0,
                capacity,
                drain_rate,
                last_drain: Instant::now(),
                deadline: deadline.map(|d| Instant::now() + d),
            })),
        }
    }

    /// Try to add `amount` to the bucket. Returns true if allowed (within capacity).
    pub fn try_send(&self, amount: f64) -> bool {
        let mut state = self.state.lock().unwrap();

        // Check deadline
        if let Some(dl) = state.deadline {
            if Instant::now() >= dl {
                return false;
            }
        }

        // Drain
        let now = Instant::now();
        let elapsed = (now - state.last_drain).as_secs_f64();
        state.queue_level = (state.queue_level - elapsed * state.drain_rate).max(0.0);
        state.last_drain = now;

        if state.queue_level + amount <= state.capacity {
            state.queue_level += amount;
            true
        } else {
            false
        }
    }

    /// Get current queue level.
    pub fn queue_level(&self) -> f64 {
        let state = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = (now - state.last_drain).as_secs_f64();
        (state.queue_level - elapsed * state.drain_rate).max(0.0)
    }

    /// Check if deadline has passed.
    pub fn is_expired(&self) -> bool {
        let state = self.state.lock().unwrap();
        match state.deadline {
            Some(dl) => Instant::now() >= dl,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // ---- Token Bucket Tests ----

    #[test]
    fn token_bucket_allows_within_capacity() {
        let tb = TokenBucket::new(10.0, 1.0, None);
        assert!(tb.try_acquire(5.0));
        assert!(tb.try_acquire(5.0));
    }

    #[test]
    fn token_bucket_rejects_over_capacity() {
        let tb = TokenBucket::new(5.0, 1.0, None);
        assert!(tb.try_acquire(3.0));
        assert!(!tb.try_acquire(3.0)); // Only 2 left
    }

    #[test]
    fn token_bucket_refills_over_time() {
        let tb = TokenBucket::new(10.0, 100.0, None); // 100 tokens/sec
        tb.try_acquire(10.0);
        assert!(!tb.try_acquire(1.0));
        thread::sleep(Duration::from_millis(50));
        // Should have ~5 tokens refilled
        assert!(tb.try_acquire(1.0));
    }

    #[test]
    fn token_bucket_respects_max() {
        let tb = TokenBucket::new(10.0, 1000.0, None);
        thread::sleep(Duration::from_millis(50));
        // Even after time passes, shouldn't exceed max
        assert!(tb.available_tokens() <= 10.0);
    }

    #[test]
    fn token_bucket_deadline_blocks() {
        let tb = TokenBucket::new(100.0, 100.0, Some(Duration::from_millis(10)));
        assert!(tb.try_acquire(1.0));
        thread::sleep(Duration::from_millis(20));
        assert!(!tb.try_acquire(1.0));
        assert!(tb.is_expired());
    }

    #[test]
    fn token_bucket_no_deadline_never_expires() {
        let tb = TokenBucket::new(10.0, 1.0, None);
        assert!(!tb.is_expired());
    }

    #[test]
    fn token_bucket_reset_refills() {
        let tb = TokenBucket::new(10.0, 1.0, None);
        tb.try_acquire(10.0);
        assert!(!tb.try_acquire(1.0));
        tb.reset();
        assert!(tb.try_acquire(10.0));
    }

    // ---- Leaky Bucket Tests ----

    #[test]
    fn leaky_bucket_allows_within_capacity() {
        let lb = LeakyBucket::new(10.0, 1.0, None);
        assert!(lb.try_send(5.0));
        assert!(lb.try_send(5.0));
    }

    #[test]
    fn leaky_bucket_rejects_over_capacity() {
        let lb = LeakyBucket::new(5.0, 1.0, None);
        assert!(lb.try_send(3.0));
        assert!(!lb.try_send(3.0));
    }

    #[test]
    fn leaky_bucket_drains_over_time() {
        let lb = LeakyBucket::new(10.0, 100.0, None); // drain 100/sec
        lb.try_send(5.0);
        thread::sleep(Duration::from_millis(50));
        // Should have drained ~5 tokens
        assert!(lb.queue_level() < 1.0);
    }

    #[test]
    fn leaky_bucket_allows_after_drain() {
        let lb = LeakyBucket::new(5.0, 200.0, None);
        lb.try_send(5.0);
        assert!(!lb.try_send(1.0));
        thread::sleep(Duration::from_millis(30));
        // Should have drained ~6 tokens
        assert!(lb.try_send(1.0));
    }

    #[test]
    fn leaky_bucket_deadline_blocks() {
        let lb = LeakyBucket::new(100.0, 1.0, Some(Duration::from_millis(10)));
        assert!(lb.try_send(1.0));
        thread::sleep(Duration::from_millis(20));
        assert!(!lb.try_send(1.0));
        assert!(lb.is_expired());
    }

    #[test]
    fn leaky_bucket_no_deadline_never_expires() {
        let lb = LeakyBucket::new(10.0, 1.0, None);
        assert!(!lb.is_expired());
    }

    #[test]
    fn leaky_bucket_queue_level_never_negative() {
        let lb = LeakyBucket::new(10.0, 100.0, None);
        thread::sleep(Duration::from_millis(50));
        assert!(lb.queue_level() >= 0.0);
    }

    #[test]
    fn token_bucket_available_decreases_after_acquire() {
        let tb = TokenBucket::new(10.0, 0.0, None); // No refill
        let before = tb.available_tokens();
        tb.try_acquire(3.0);
        let after = tb.available_tokens();
        assert!(after < before);
        assert!((after - 7.0).abs() < 0.01);
    }
}
