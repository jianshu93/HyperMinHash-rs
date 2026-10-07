use crate::SIGNATURE_BITS;

pub(crate) fn histogram(registers: &[u16]) -> [usize; 64] {
    let mut histogram = [0; 64];
    for &register in registers {
        histogram[(register >> SIGNATURE_BITS) as usize] += 1;
    }
    histogram
}

// Ertl's improved raw estimator (arXiv:1702.01284, Algorithm 6). It avoids
// empirical bias coefficients tied to a particular register count.
pub(crate) fn cardinality(p: u32, histogram: &[usize; 64]) -> f64 {
    let count = (1usize << p) as f64;
    let maximum_rank = (65 - p) as usize;
    if histogram[0] as f64 == count {
        return 0.0;
    }
    let mut denominator = count * tau(1.0 - histogram[maximum_rank] as f64 / count);
    for rank in (1..maximum_rank).rev() {
        denominator = (denominator + histogram[rank] as f64) * 0.5;
    }
    denominator += count * sigma(histogram[0] as f64 / count);
    count * count / (2.0 * std::f64::consts::LN_2 * denominator)
}

fn sigma(mut fraction: f64) -> f64 {
    if fraction == 1.0 {
        return f64::INFINITY;
    }
    let mut sum = fraction;
    let mut weight = 1.0;
    loop {
        fraction *= fraction;
        let previous = sum;
        sum += fraction * weight;
        if sum == previous {
            return sum;
        }
        weight *= 2.0;
    }
}

fn tau(mut fraction: f64) -> f64 {
    if fraction == 0.0 || fraction == 1.0 {
        return 0.0;
    }
    let mut sum = 1.0 - fraction;
    let mut weight = 1.0;
    loop {
        fraction = fraction.sqrt();
        weight *= 0.5;
        let previous = sum;
        sum -= (1.0 - fraction).powi(2) * weight;
        if sum == previous {
            return sum / 3.0;
        }
    }
}

// HyperMinHash's approximate collision correction (arXiv:1710.08436,
// Algorithm 6). No per-precision lookup tables are needed.
pub(crate) fn expected_collisions(p: u32, left: f64, right: f64) -> f64 {
    let large = left.max(right);
    let small = left.min(right);
    if small == 0.0 {
        return 0.0;
    }
    if !large.is_finite() {
        return f64::INFINITY;
    }
    let registers = (1usize << p) as f64;
    let signatures = (1u32 << SIGNATURE_BITS) as f64;
    if large > 32.0 * registers {
        let ratio = small / large;
        return 0.169_919_487_159_739_1 * registers / signatures
            * (4.0 * ratio / (1.0 + ratio).powi(2));
    }
    let mut collisions = 0.0;
    for rank in 1..=64 - p {
        let lower = 2.0f64.powi(-((p + rank) as i32));
        let upper = 2.0 * lower;
        collisions +=
            interval_probability(left, lower, upper) * interval_probability(right, lower, upper);
    }
    let upper = 2.0f64.powi(-64);
    collisions += interval_probability(left, 0.0, upper) * interval_probability(right, 0.0, upper);
    collisions * registers / signatures
}

fn interval_probability(cardinality: f64, lower: f64, upper: f64) -> f64 {
    let low_log = cardinality * (-lower).ln_1p();
    let high_log = cardinality * (-upper).ln_1p();
    low_log.exp() * -(high_log - low_log).exp_m1()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collision_correction_is_symmetric_finite_and_scales_with_buckets() {
        for p in [4, 10, 14, 20, 26] {
            // Two independent singleton sets match a bucket with probability
            // 1/2^p, a geometric rank with probability 1/3, and a signature
            // with probability 1/2^r (finite-hash tail effects are negligible).
            let singleton = 1.0 / (3.0 * (1usize << p) as f64 * (1u32 << SIGNATURE_BITS) as f64);
            assert!((expected_collisions(p, 1.0, 1.0) / singleton - 1.0).abs() < 1e-12);
            for (left, right) in [(1.0, 2.0), (1000.0, 5000.0), (1e12, 1e10)] {
                let value = expected_collisions(p, left, right);
                assert!(value.is_finite() && value >= 0.0);
                assert_eq!(
                    value.to_bits(),
                    expected_collisions(p, right, left).to_bits()
                );
            }
            assert_eq!(expected_collisions(p, 0.0, 100.0), 0.0);
        }
        assert_eq!(
            expected_collisions(14, 1e12, 1e12),
            expected_collisions(10, 1e12, 1e12) * 16.0
        );
    }
    #[test]
    fn histogram_counts_do_not_overflow_at_large_precision() {
        let p = 20;
        let mut counts = [0; 64];
        counts[0] = 1 << p;
        assert_eq!(cardinality(p, &counts), 0.0);
        counts[0] = 0;
        counts[1] = 1 << p;
        assert!(cardinality(p, &counts).is_finite());
        counts[1] = 0;
        counts[(65 - p) as usize] = 1 << p;
        assert_eq!(cardinality(p, &counts), f64::INFINITY);
    }
}
