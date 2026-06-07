# t-minus

> Cron-like scheduling, deadline propagation trees, and backpressure-aware rate limiters for agent fleets.

## What This Does

This crate gives your agents three time primitives: a cron expression parser that computes next-fire times without external dependencies, a hierarchical deadline tree where parent expiry cascades to children, and dual rate limiters (token bucket + leaky bucket) with optional hard deadlines. Together they let you schedule fleet-wide operations, enforce end-to-end timeouts that propagate through task hierarchies, and shed load gracefully when downstream agents are overwhelmed.

## Why It Matters

Time is not a side effect — it is a first-class resource in any distributed system. AGI fleets that cannot reason about scheduling, deadlines, and backpressure will collapse under their own coordination overhead. `t-minus` treats time as a composable structure: cron expressions define rhythm, deadline trees define lifespan, and rate limiters define breathing room. A fleet that understands its own temporal constraints is a fleet that survives overload.

## Quick Start

```bash
cargo add t-minus
```

```rust
use std::time::Duration;
use t_minus::schedule::CronExpr;
use t_minus::deadline::DeadlineNode;
use t_minus::backpressure::TokenBucket;

fn main() {
    // Schedule: every 15 minutes
    let cron = CronExpr::parse("*/15 * * * *").unwrap();
    let now = t_minus::schedule::now_secs();
    let next = cron.next_after(now).unwrap();
    println!("Next fire: {}", next);

    // Deadline tree: parent 60s, child 120s → child inherits 60s
    let parent = DeadlineNode::new(1, Some(Duration::from_secs(60)));
    let child = parent.add_child(2, Some(Duration::from_secs(120)));
    assert!(child.remaining().unwrap() <= Duration::from_secs(60));

    // Token bucket: burst of 10, refill 2/sec, expires after 5 minutes
    let bucket = TokenBucket::new(10.0, 2.0, Some(Duration::from_secs(300)));
    assert!(bucket.try_acquire(3.0));
    println!("Tokens left: {:.1}", bucket.available_tokens());
}
```

## Architecture

| Module | Purpose |
|--------|---------|
| `schedule` | Parse cron expressions (`"0 18 * * *"`), match timestamps, compute next fire time |
| `deadline` | Hierarchical deadline nodes with parent→child inheritance and cascade cancellation |
| `backpressure` | Token bucket and leaky bucket rate limiters with optional expiry |

## API Tour

### `CronExpr`

Parse and evaluate standard 5-field cron expressions.

```rust
pub fn parse(expr: &str) -> Result<CronExpr, ParseError>
pub fn matches(&self, timestamp_secs: u64) -> bool
pub fn next_after(&self, after: u64) -> Option<u64>
```

```rust
let expr = CronExpr::parse("0 9-17 * * 1-5").unwrap(); // Every hour, weekdays
assert!(expr.matches(1736164800)); // Monday noon
```

### `DeadlineNode`

A node in a deadline propagation tree. Children inherit the tighter of their own deadline or their parent's.

```rust
pub fn new(id: u64, deadline: Option<Duration>) -> Self
pub fn add_child(&self, child_id: u64, child_deadline: Option<Duration>) -> DeadlineNode
pub fn status(&self) -> DeadlineStatus  // Active | Expired | Cancelled
pub fn cancel(&self)  // Cascades to all descendants
pub fn remaining(&self) -> Option<Duration>
```

```rust
let root = DeadlineNode::new(1, Some(Duration::from_secs(60)));
let task = root.add_child(2, Some(Duration::from_secs(120)));
let subtask = task.add_child(3, None); // Inherits 60s
root.cancel(); // task and subtask also become Cancelled
```

### `TokenBucket`

Burst-capable rate limiter with automatic refill and optional hard deadline.

```rust
pub fn new(max_tokens: f64, refill_rate: f64, deadline: Option<Duration>) -> Self
pub fn try_acquire(&self, count: f64) -> bool
pub fn available_tokens(&self) -> f64
pub fn reset(&self)
```

### `LeakyBucket`

Smooths traffic by queueing and draining at a fixed rate.

```rust
pub fn new(capacity: f64, drain_rate: f64, deadline: Option<Duration>) -> Self
pub fn try_send(&self, amount: f64) -> bool
pub fn queue_level(&self) -> f64
```

## Performance

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Cron parse | O(1) | Five fields, constant-size tokenization |
| Cron match | O(1) | Direct field comparison |
| `next_after` | O(years × minutes) | Brute-force minute scan; up to ~4 years horizon |
| Deadline status | O(1) | Mutex lock + Instant comparison |
| Cancel cascade | O(k) | k = number of descendants |
| Token acquire | O(1) | Mutex lock + arithmetic |
| Leaky send | O(1) | Mutex lock + arithmetic |

The brute-force `next_after` trades asymptotic elegance for implementation simplicity and correctness. For sub-millisecond scheduling, a precomputed event calendar would be the next step.

## Ecosystem

- **[fleet-warden](https://github.com/SuperInstance/fleet-warden-rs)** — Trigger disk cleanup on cron schedules from `t-minus`
- **[conservation-law](https://github.com/SuperInstance/conservation-law-rs)** — Bind symplectic integrator timesteps to `DeadlineNode` expiry
- **[categorical-agents](https://github.com/SuperInstance/categorical-agents-rs)** — Model scheduling pipelines as state-monad computations
- **[spectral-fleet](https://github.com/SuperInstance/spectral-fleet-rs)** — Recompute fleet clusters on scheduled intervals

## Ideas for Improvement

1. **Calendar queue `next_after`** — Replace brute-force scan with an O(1) calendar queue for high-frequency scheduling.
2. **Async deadline watchers** — Spawn tokio tasks that emit signals when deadlines expire instead of polling.
3. **Distributed deadline propagation** — Serialize deadline trees to JSON and synchronize across fleet nodes.
4. **Adaptive backpressure** — Dynamically adjust token bucket refill rates based on observed queue depths.
5. **CRON extensions** — Support `@daily`, `@weekly`, L-last-day-of-month, and timezone-aware scheduling.

## License

MIT OR Apache-2.0
