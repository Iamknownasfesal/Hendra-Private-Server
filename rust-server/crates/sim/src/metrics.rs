//! Tick timing.
//!
//! The reason anyone knows the C# server is slow is that a number in a config file said `tps: 6`.
//! Nothing measured it; nothing would have noticed it drifting. This exists so the Rust server
//! never needs that kind of archaeology.
//!
//! The budget is what one tick may take: 50 ms at 20 ticks per second. The number worth watching is
//! not the mean — a mean hides the tick that stuttered — but the high percentiles, because those
//! are what a player actually feels.

use std::time::Duration;

/// Buckets, in microseconds. Logarithmic, because the interesting range spans four orders of
/// magnitude and a linear histogram would spend all its resolution where nothing happens.
const BOUNDS: [u64; 16] = [
    50,
    100,
    200,
    400,
    800,
    1_600,
    3_200,
    6_400,
    12_800,
    25_600,
    51_200,
    102_400,
    204_800,
    409_600,
    819_200,
    u64::MAX,
];

/// A histogram of how long ticks took.
#[derive(Debug, Clone)]
pub struct TickMetrics {
    buckets: [u64; 16],
    count: u64,
    total_us: u64,
    max_us: u64,

    /// How many ticks ran over budget.
    over_budget: u64,
    budget_us: u64,
}

impl TickMetrics {
    /// Tracks against a tick rate, deriving the budget from it.
    pub fn for_rate(ticks_per_second: u32) -> TickMetrics {
        let budget_us = if ticks_per_second == 0 {
            u64::MAX
        } else {
            1_000_000 / ticks_per_second as u64
        };

        TickMetrics {
            buckets: [0; 16],
            count: 0,
            total_us: 0,
            max_us: 0,
            over_budget: 0,
            budget_us,
        }
    }

    /// Records one tick.
    pub fn record(&mut self, elapsed: Duration) {
        let micros = elapsed.as_micros().min(u64::MAX as u128) as u64;

        let bucket = BOUNDS
            .iter()
            .position(|&bound| micros <= bound)
            .unwrap_or(BOUNDS.len() - 1);
        self.buckets[bucket] += 1;

        self.count += 1;
        self.total_us += micros;
        self.max_us = self.max_us.max(micros);
        if micros > self.budget_us {
            self.over_budget += 1;
        }
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn budget(&self) -> Duration {
        Duration::from_micros(self.budget_us)
    }

    pub fn mean(&self) -> Duration {
        if self.count == 0 {
            return Duration::ZERO;
        }
        Duration::from_micros(self.total_us / self.count)
    }

    pub fn max(&self) -> Duration {
        Duration::from_micros(self.max_us)
    }

    /// How many ticks exceeded the budget.
    pub fn over_budget(&self) -> u64 {
        self.over_budget
    }

    /// An upper bound on the given percentile.
    ///
    /// Bucketed, so this reports the top of the bucket the percentile falls in rather than an exact
    /// figure — it answers "no worse than" rather than "exactly", which is the honest thing a
    /// histogram can say and the thing an alarm should be set against.
    pub fn percentile(&self, percentile: f64) -> Duration {
        if self.count == 0 {
            return Duration::ZERO;
        }

        let target = ((self.count as f64) * percentile.clamp(0.0, 1.0)).ceil() as u64;
        let mut seen = 0u64;

        for (index, &occupants) in self.buckets.iter().enumerate() {
            seen += occupants;
            if seen >= target {
                // Capped at the largest tick actually observed. That also handles the final,
                // unbounded bucket, which would otherwise report u64::MAX. Without the cap a bucket
                // bound can
                // read higher than the maximum, which is arithmetically defensible and reads as
                // nonsense — "p99 1.6ms, max 1.0ms" invites the reader to distrust both numbers.
                return Duration::from_micros(BOUNDS[index].min(self.max_us));
            }
        }

        Duration::from_micros(self.max_us)
    }

    /// Whether the simulation is comfortably inside its budget.
    ///
    /// Deliberately p99 rather than the mean: a world that meets its budget on average and misses
    /// it one tick in fifty is a world that visibly stutters twice a second.
    pub fn healthy(&self) -> bool {
        self.count == 0 || self.percentile(0.99).as_micros() as u64 <= self.budget_us
    }

    pub fn reset(&mut self) {
        *self = TickMetrics::for_rate(if self.budget_us == 0 {
            0
        } else {
            (1_000_000 / self.budget_us.max(1)) as u32
        });
    }
}

impl std::fmt::Display for TickMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ticks — mean {:.2?}, p50 {:.2?}, p99 {:.2?}, max {:.2?}, budget {:.2?}, over {}",
            self.count,
            self.mean(),
            self.percentile(0.50),
            self.percentile(0.99),
            self.max(),
            self.budget(),
            self.over_budget
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_histogram_reports_nothing_rather_than_dividing_by_zero() {
        let metrics = TickMetrics::for_rate(20);
        assert_eq!(metrics.count(), 0);
        assert_eq!(metrics.mean(), Duration::ZERO);
        assert_eq!(metrics.percentile(0.99), Duration::ZERO);
        assert!(metrics.healthy(), "no data is not a failure");
    }

    #[test]
    fn the_budget_follows_the_tick_rate() {
        assert_eq!(
            TickMetrics::for_rate(20).budget(),
            Duration::from_millis(50)
        );
        assert_eq!(
            TickMetrics::for_rate(6).budget(),
            Duration::from_micros(166_666)
        );
        assert_eq!(
            TickMetrics::for_rate(60).budget(),
            Duration::from_micros(16_666)
        );
    }

    #[test]
    fn mean_and_max_track_what_was_recorded() {
        let mut metrics = TickMetrics::for_rate(20);
        for micros in [100u64, 200, 300, 400] {
            metrics.record(Duration::from_micros(micros));
        }

        assert_eq!(metrics.count(), 4);
        assert_eq!(metrics.mean(), Duration::from_micros(250));
        assert_eq!(metrics.max(), Duration::from_micros(400));
    }

    #[test]
    fn percentiles_bound_the_distribution() {
        let mut metrics = TickMetrics::for_rate(20);

        // Ninety-nine fast ticks and one slow one.
        for _ in 0..99 {
            metrics.record(Duration::from_micros(80));
        }
        metrics.record(Duration::from_millis(200));

        assert!(
            metrics.percentile(0.50) <= Duration::from_micros(100),
            "the median should sit with the fast ticks"
        );
        assert!(
            metrics.percentile(0.99) <= Duration::from_micros(100),
            "so should p99, with only one outlier in a hundred"
        );
        assert_eq!(metrics.max(), Duration::from_millis(200));
    }

    #[test]
    fn a_mean_inside_budget_does_not_make_a_stuttering_world_healthy() {
        let mut metrics = TickMetrics::for_rate(20);

        // Every fiftieth tick blows the budget. The mean stays comfortable; the world does not.
        for n in 0..1000 {
            if n % 50 == 0 {
                metrics.record(Duration::from_millis(120));
            } else {
                metrics.record(Duration::from_micros(500));
            }
        }

        assert!(
            metrics.mean() < metrics.budget(),
            "the mean is inside budget, which is exactly the trap"
        );
        assert!(!metrics.healthy(), "but p99 is not, so it is not healthy");
        assert_eq!(metrics.over_budget(), 20);
    }

    #[test]
    fn a_world_inside_its_budget_reads_as_healthy() {
        let mut metrics = TickMetrics::for_rate(20);
        for _ in 0..500 {
            metrics.record(Duration::from_micros(900));
        }

        assert!(metrics.healthy());
        assert_eq!(metrics.over_budget(), 0);
        assert!(metrics.percentile(0.99) < metrics.budget());
    }

    #[test]
    fn a_percentile_never_reads_higher_than_the_maximum() {
        let mut metrics = TickMetrics::for_rate(20);

        // Ticks that all land well inside one bucket, so the bucket's bound is far above them.
        for _ in 0..100 {
            metrics.record(Duration::from_micros(1_000));
        }

        assert_eq!(metrics.max(), Duration::from_micros(1_000));
        for quantile in [0.5, 0.9, 0.99, 1.0] {
            assert!(
                metrics.percentile(quantile) <= metrics.max(),
                "p{quantile} exceeded the maximum"
            );
        }
    }

    #[test]
    fn an_enormous_tick_lands_in_the_final_bucket_without_overflowing() {
        let mut metrics = TickMetrics::for_rate(20);
        metrics.record(Duration::from_secs(30));

        assert_eq!(metrics.count(), 1);
        assert_eq!(metrics.max(), Duration::from_secs(30));
        assert_eq!(
            metrics.percentile(0.99),
            Duration::from_secs(30),
            "the unbounded bucket reports the real maximum, not u64::MAX"
        );
        assert!(!metrics.healthy());
    }
}
