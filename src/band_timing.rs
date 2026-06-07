//! Band timing: coordinated timing for agent ensembles without a shared clock.
//!
//! Each agent gets an independent clock with configurable drift. Beat/bar/phrase
//! tracking lets agents stay loosely synchronised. Tempo negotiation converges
//! proposals via weighted average while respecting a conservation budget.

/// An independent clock for a single agent in the band.
///
/// Every clock drifts from the nominal tempo at its own rate, modelling
/// real-world timing imprecision between autonomous agents.
#[derive(Debug, Clone)]
pub struct BandClock {
    /// Unique agent identifier.
    pub agent_id: u64,
    /// Base tempo in BPM.
    pub base_tempo: f64,
    /// Drift rate: fraction of tempo change per second (can be negative).
    pub drift_rate: f64,
    /// Accumulated phase offset in seconds.
    pub phase_offset: f64,
    /// Time of last tick (seconds).
    last_tick: f64,
    /// Current effective tempo (BPM), accounting for drift.
    pub current_tempo: f64,
}

impl BandClock {
    /// Create a new clock for `agent_id` at `base_tempo` BPM with the given `drift_rate`.
    pub fn new(agent_id: u64, base_tempo: f64, drift_rate: f64) -> Self {
        Self {
            agent_id,
            base_tempo,
            drift_rate,
            phase_offset: 0.0,
            last_tick: 0.0,
            current_tempo: base_tempo,
        }
    }

    /// Advance the clock to `current_time` and return the effective tempo.
    pub fn tick(&mut self, current_time: f64) -> f64 {
        let dt = current_time - self.last_tick;
        self.current_tempo = self.base_tempo + self.drift_rate * current_time;
        self.phase_offset += self.drift_rate * dt;
        self.last_tick = current_time;
        self.current_tempo
    }

    /// Beat duration in seconds at the current effective tempo.
    pub fn beat_duration(&self) -> f64 {
        60.0 / self.current_tempo
    }

    /// Bar duration (4 beats) in seconds.
    pub fn bar_duration(&self) -> f64 {
        4.0 * self.beat_duration()
    }

    /// Phrase duration (4 bars) in seconds.
    pub fn phrase_duration(&self) -> f64 {
        4.0 * self.bar_duration()
    }

    /// Reset drift / phase to factory state.
    pub fn reset(&mut self) {
        self.phase_offset = 0.0;
        self.last_tick = 0.0;
        self.current_tempo = self.base_tempo;
    }
}

// ---------------------------------------------------------------------------
// BeatTracker
// ---------------------------------------------------------------------------

/// Track beat / bar / phrase boundaries from a single agent's clock.
#[derive(Debug, Clone)]
pub struct BeatTracker {
    /// Beat count within the current bar (0..3).
    pub beat_in_bar: u32,
    /// Bar count within the current phrase (0..3).
    pub bar_in_phrase: u32,
    /// Total beats elapsed.
    pub total_beats: u64,
    /// Time of the last beat boundary.
    last_beat_time: f64,
    /// Accumulated phase towards the next beat.
    phase: f64,
}

impl BeatTracker {
    /// Create a tracker starting at beat 0.
    pub fn new() -> Self {
        Self {
            beat_in_bar: 0,
            bar_in_phrase: 0,
            total_beats: 0,
            last_beat_time: 0.0,
            phase: 0.0,
        }
    }

    /// Advance the tracker by `dt` seconds at the given `beat_duration`.
    /// Returns `true` if a beat boundary was crossed during this update.
    pub fn update(&mut self, dt: f64, beat_duration: f64) -> bool {
        self.phase += dt;
        if self.phase >= beat_duration {
            self.phase -= beat_duration;
            self.total_beats += 1;
            self.beat_in_bar += 1;
            if self.beat_in_bar >= 4 {
                self.beat_in_bar = 0;
                self.bar_in_phrase += 1;
                if self.bar_in_phrase >= 4 {
                    self.bar_in_phrase = 0;
                }
            }
            return true;
        }
        false
    }

    /// Current phrase number (0-indexed).
    pub fn phrase_number(&self) -> u64 {
        self.total_beats / 16
    }

    /// Current bar number (0-indexed globally).
    pub fn bar_number(&self) -> u64 {
        self.total_beats / 4
    }
}

impl Default for BeatTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TempoNegotiator
// ---------------------------------------------------------------------------

/// Negotiates a shared tempo from per-agent proposals using weighted averaging,
/// while respecting an entropy-conservation budget that limits how far the
/// negotiated tempo can drift from the baseline.
pub struct TempoNegotiator {
    /// Maximum allowed deviation from the baseline tempo (BPM).
    pub conservation_budget: f64,
    /// Baseline (reference) tempo.
    pub baseline_tempo: f64,
}

impl TempoNegotiator {
    /// Create a negotiator with the given baseline tempo and conservation budget.
    pub fn new(baseline_tempo: f64, conservation_budget: f64) -> Self {
        Self {
            conservation_budget,
            baseline_tempo,
        }
    }

    /// Compute the negotiated tempo from a set of proposals and weights.
    ///
    /// The result is the weighted average of proposals, clamped to
    /// `[baseline - budget, baseline + budget]`.
    pub fn negotiate_tempo(&self, proposals: &[f64], weights: &[f64]) -> f64 {
        assert_eq!(
            proposals.len(),
            weights.len(),
            "proposals and weights must have the same length"
        );
        if proposals.is_empty() {
            return self.baseline_tempo;
        }
        let total_weight: f64 = weights.iter().copied().sum();
        if total_weight.abs() < 1e-15 {
            return self.baseline_tempo;
        }
        let weighted: f64 = proposals
            .iter()
            .zip(weights.iter())
            .map(|(&p, &w)| p * w)
            .sum();
        let avg = weighted / total_weight;
        let lo = self.baseline_tempo - self.conservation_budget;
        let hi = self.baseline_tempo + self.conservation_budget;
        avg.clamp(lo, hi)
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Compute the time of the next beat for a given clock, starting from
/// `current_time`.
pub fn next_beat(clock: &BandClock, current_time: f64) -> f64 {
    let beat_dur = clock.beat_duration();
    // How far are we past the last beat?
    let elapsed_since_tick = current_time - clock.last_tick;
    let remaining = beat_dur - elapsed_since_tick;
    if remaining > 0.0 {
        current_time + remaining
    } else {
        // Already past the next beat – snap forward one beat.
        current_time + beat_dur
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- BandClock ----

    #[test]
    fn test_clock_new() {
        let c = BandClock::new(1, 120.0, 0.0);
        assert_eq!(c.agent_id, 1);
        assert!((c.current_tempo - 120.0).abs() < 1e-10);
        assert!((c.beat_duration() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_clock_tick_updates_tempo() {
        let mut c = BandClock::new(0, 120.0, 1.0); // +1 BPM/s drift
        let t = c.tick(10.0);
        // current_tempo = 120 + 1.0 * 10 = 130
        assert!((t - 130.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_drift_accumulates_phase() {
        let mut c = BandClock::new(0, 100.0, 0.5);
        c.tick(2.0); // dt=2, phase += 0.5*2 = 1.0
        assert!((c.phase_offset - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_bar_and_phrase_durations() {
        let c = BandClock::new(0, 120.0, 0.0);
        // beat=0.5s, bar=2.0s, phrase=8.0s
        assert!((c.bar_duration() - 2.0).abs() < 1e-10);
        assert!((c.phrase_duration() - 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_clock_reset() {
        let mut c = BandClock::new(0, 110.0, 2.0);
        c.tick(5.0);
        c.reset();
        assert!((c.phase_offset).abs() < 1e-10);
        assert!((c.current_tempo - 110.0).abs() < 1e-10);
    }

    // ---- BeatTracker ----

    #[test]
    fn test_beat_tracker_single_beat() {
        let mut bt = BeatTracker::new();
        let crossed = bt.update(0.5, 0.5); // exactly one beat
        assert!(crossed);
        assert_eq!(bt.total_beats, 1);
        assert_eq!(bt.beat_in_bar, 1);
    }

    #[test]
    fn test_beat_tracker_bar_boundary() {
        let mut bt = BeatTracker::new();
        for _ in 0..4 {
            bt.update(0.5, 0.5);
        }
        assert_eq!(bt.total_beats, 4);
        assert_eq!(bt.beat_in_bar, 0); // wrapped
        assert_eq!(bt.bar_in_phrase, 1);
    }

    #[test]
    fn test_beat_tracker_phrase_boundary() {
        let mut bt = BeatTracker::new();
        for _ in 0..16 {
            bt.update(0.5, 0.5);
        }
        assert_eq!(bt.total_beats, 16);
        assert_eq!(bt.beat_in_bar, 0);
        assert_eq!(bt.bar_in_phrase, 0); // wrapped
        assert_eq!(bt.phrase_number(), 1);
    }

    #[test]
    fn test_beat_tracker_partial() {
        let mut bt = BeatTracker::new();
        let crossed = bt.update(0.1, 0.5); // not enough for a beat
        assert!(!crossed);
        assert_eq!(bt.total_beats, 0);
    }

    #[test]
    fn test_beat_tracker_accumulates_phase() {
        let mut bt = BeatTracker::new();
        bt.update(0.3, 0.5);
        bt.update(0.3, 0.5); // total phase 0.6 > 0.5 → beat
        assert_eq!(bt.total_beats, 1);
    }

    // ---- TempoNegotiator ----

    #[test]
    fn test_negotiate_basic_weighted_average() {
        let tn = TempoNegotiator::new(120.0, 40.0);
        let result = tn.negotiate_tempo(&[100.0, 140.0], &[1.0, 1.0]);
        assert!((result - 120.0).abs() < 1e-10);
    }

    #[test]
    fn test_negotiate_clamped_by_budget() {
        let tn = TempoNegotiator::new(120.0, 5.0); // tight budget
        let result = tn.negotiate_tempo(&[200.0], &[1.0]);
        // 200 clamped to 120+5 = 125
        assert!((result - 125.0).abs() < 1e-10);
    }

    #[test]
    fn test_negotiate_clamped_lower() {
        let tn = TempoNegotiator::new(120.0, 5.0);
        let result = tn.negotiate_tempo(&[50.0], &[1.0]);
        assert!((result - 115.0).abs() < 1e-10);
    }

    #[test]
    fn test_negotiate_empty_returns_baseline() {
        let tn = TempoNegotiator::new(120.0, 10.0);
        let result = tn.negotiate_tempo(&[], &[]);
        assert!((result - 120.0).abs() < 1e-10);
    }

    #[test]
    fn test_negotiate_weighted_unequal() {
        let tn = TempoNegotiator::new(120.0, 100.0);
        let result = tn.negotiate_tempo(&[100.0, 200.0], &[3.0, 1.0]);
        // (100*3 + 200*1) / 4 = 125
        assert!((result - 125.0).abs() < 1e-10);
    }

    // ---- next_beat ----

    #[test]
    fn test_next_beat_basic() {
        let c = BandClock::new(0, 120.0, 0.0);
        let nb = next_beat(&c, 0.1);
        // beat_dur=0.5, elapsed=0.1, remaining=0.4
        assert!((nb - 0.5).abs() < 1e-10);
    }
}
