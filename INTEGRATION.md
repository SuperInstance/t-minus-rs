# Integration Guide: t-minus

## What This Crate Provides

- **`CronExpr`** — Parse and evaluate 5-field cron expressions (`"0 18 * * *"`)
- **`CronField`** — Individual cron field (All, Exact, List, Range, Step)
- **`DeadlineNode`** — Tree-structured deadline propagation: parent deadlines distribute to children, cancellation cascades on parent expiry
- **`DeadlineStatus`** — Active / Expired / Cancelled states
- **`TokenBucket`** — Token bucket rate limiter with burst capacity, refill rate, and hard deadline cutoff
- **`LeakyBucket`** — Leaky bucket rate limiter with drain rate and deadline awareness

This crate provides temporal coordination for the SuperInstance ecosystem: cron-like scheduling, hierarchical deadline propagation, and rate limiting that respects deadlines. Agents don't sync to a shared clock — they predict events using t-minus.

## How to Add This Crate

```bash
cargo add t-minus
```

```rust
use t_minus::schedule::CronExpr;

// Parse "every day at 6 PM"
let expr = CronExpr::parse("0 18 * * *").unwrap();
let now = std::time::SystemTime::now();
let next = expr.next_fire_after(now);
println!("Next fire: {:?}", next);
```

## Integration Points

### conservation-rhythm

- **Why**: conservation-rhythm enforces the γ + H = C invariant over time; t-minus provides the temporal structure. Every conservation audit must be scheduled, and every schedule must respect conservation deadlines.
- **How**: Schedule periodic conservation audits using `CronExpr`, and use `DeadlineNode` to ensure each audit completes before the fleet state changes.

```rust
use t_minus::schedule::CronExpr;
use t_minus::deadline::DeadlineNode;
use std::time::Duration;

// Schedule conservation audit every hour
let audit_cron = CronExpr::parse("0 * * * *").unwrap();

// Create a deadline tree: parent audit → child checks
let parent = DeadlineNode::new(0, Some(Duration::from_secs(300)));
let child_check = DeadlineNode::new(1, Some(Duration::from_secs(60)));
parent.add_child(child_check);

println!("Audit status: {:?}", parent.status());
```

### fleet-warden

- **Why**: fleet-warden runs periodic cleanup sweeps; t-minus schedules when sweeps fire and ensures they complete within deadlines. Fleet-warden's watch mode IS t-minus scheduling.
- **How**: Replace fleet-warden's internal interval loop with t-minus cron scheduling for more precise control (e.g., "clean at 3 AM on weekdays, every 2 hours on weekends").

```rust
use t_minus::schedule::CronExpr;
use t_minus::backpressure::TokenBucket;

// Rate-limit fleet-warden cleanup operations
let bucket = TokenBucket::new(
    5.0,                           // max burst: 5 cleanups
    1.0 / 3600.0,                  // refill: ~1 per hour
    Some(Duration::from_secs(86400)), // hard deadline: 24h
);

if bucket.try_acquire(1.0) {
    println!("Cleanup allowed");
} else {
    println!("Rate limited or past deadline");
}
```

### spectral-fleet

- **Why**: spectral-fleet's `EigenmodeScheduler` produces task priorities; t-minus executes those priorities with proper deadline enforcement. Spectral computations must complete before their deadline expires.
- **How**: Feed spectral-fleet's `Deadline` objects into t-minus's `DeadlineNode` tree for hierarchical deadline propagation.

```rust
use t_minus::deadline::DeadlineNode;
use std::time::Duration;

// Spectral eigenmode tasks become deadline nodes
let root = DeadlineNode::new(0, Some(Duration::from_secs(60)));
let mode_0 = DeadlineNode::new(1, Some(Duration::from_secs(10))); // bottleneck
let mode_1 = DeadlineNode::new(2, Some(Duration::from_secs(20)));
root.add_child(mode_0);
root.add_child(mode_1);

// If root expires, all children cascade to Cancelled
```

## For AI Agents

- **Context needed**: Desired schedule (cron expression or interval), deadline budget, rate limits
- **Key imports**: `t_minus::schedule::CronExpr`, `t_minus::deadline::DeadlineNode`, `t_minus::backpressure::TokenBucket`
- **Integration pattern**: Parse cron → compute next fire → create `DeadlineNode` tree → attach rate limiter → execute on schedule
- **Error handling**: `CronParseError` for invalid expressions, `DeadlineStatus::Expired`/`Cancelled` for missed deadlines, `TokenBucket::try_acquire` returns `false` when rate-limited or past deadline

## For Humans

- **Prerequisites**: Cron expression syntax, understanding of token bucket rate limiting
- **Learning path**: Start with `schedule.rs` (cron parsing), then `deadline.rs` (tree propagation), then `backpressure.rs` (rate limiting)
- **Common pitfalls**:
  - Cron fields are: minute, hour, day-of-month, month, day-of-week — not the standard Vixie cron order of seconds
  - `DeadlineNode` status checks update to `Expired` lazily — call `.status()` to trigger the check
  - Token bucket deadline is a hard cutoff: after it passes, ALL requests are denied regardless of available tokens
  - The leaky bucket drain rate is in tokens/second — use small values (e.g., 0.01) for human-timescale rates
