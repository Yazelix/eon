//! The statistics every suite reports over its samples.

/// The count, median, 95th percentile, minimum and maximum of a set of
/// samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub median: f64,
    pub p95: f64,
    pub min: f64,
    pub max: f64,
}

impl Summary {
    /// The median is the mean of the two middle samples when their count is
    /// even. The 95th percentile is nearest-rank, the `ceil(0.95 n)`-th
    /// smallest, so it is always an observed sample, and below 20 samples it
    /// is the maximum. `None` for no samples.
    pub fn of(samples: &[f64]) -> Option<Self> {
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let (&min, &max) = (sorted.first()?, sorted.last()?);
        let median = if n % 2 == 1 {
            sorted[n / 2]
        } else {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        };
        let p95 = sorted[(n * 95).div_ceil(100) - 1];
        Some(Self {
            n,
            median,
            p95,
            min,
            max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Summary;

    fn summary(samples: &[f64]) -> (f64, f64, f64, f64) {
        let summary = Summary::of(samples).unwrap();
        (summary.median, summary.p95, summary.min, summary.max)
    }

    /// Order does not matter; the median averages the middle pair only for an
    /// even count; p95 is the nearest-rank sample, which is the maximum below
    /// 20 samples and the 19th of 20.
    #[test]
    fn summary_takes_median_nearest_rank_p95_and_extremes() {
        assert_eq!(Summary::of(&[]), None);
        assert_eq!(summary(&[4.0]), (4.0, 4.0, 4.0, 4.0));
        assert_eq!(summary(&[3.0, 1.0, 2.0]), (2.0, 3.0, 1.0, 3.0));
        assert_eq!(summary(&[4.0, 1.0, 3.0, 2.0]), (2.5, 4.0, 1.0, 4.0));
        let ten: Vec<f64> = (1..=10).rev().map(f64::from).collect();
        assert_eq!(summary(&ten), (5.5, 10.0, 1.0, 10.0));
        let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(summary(&twenty), (10.5, 19.0, 1.0, 20.0));
        assert_eq!(Summary::of(&twenty).unwrap().n, 20);
        let hundred_one: Vec<f64> = (0..=100).map(f64::from).collect();
        assert_eq!(summary(&hundred_one), (50.0, 95.0, 0.0, 100.0));
    }
}
