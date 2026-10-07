use hyperminhash::{DEFAULT_P, Entry, HyperMinHashError, MAX_P, MIN_P, Sketch};

#[test]
fn validates_precision_and_requested_error_rate() {
    for p in [0, MIN_P - 1, MAX_P + 1, u32::MAX] {
        assert_eq!(
            Sketch::with_precision(p),
            Err(HyperMinHashError::InvalidPrecision(p))
        );
    }
    assert_eq!(Sketch::new().p(), DEFAULT_P);
    for error in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-10] {
        assert_eq!(
            Sketch::with_error_rate(error),
            Err(HyperMinHashError::InvalidErrorRate)
        );
    }
    for error in [0.26, 0.1, 0.02, 0.01] {
        let sketch = Sketch::with_error_rate(error).unwrap();
        assert!(sketch.expected_relative_error() <= error);
        if sketch.p() > MIN_P {
            let previous = Sketch::with_precision(sketch.p() - 1).unwrap();
            assert!(previous.expected_relative_error() > error);
        }
    }
    let maximum = Sketch::with_precision(MAX_P).unwrap();
    assert_eq!(maximum.register_count(), 1 << MAX_P);
    assert_eq!(maximum.memory_size(), 2 << MAX_P);
}

#[test]
fn hashes_and_merges_match_the_fixed_precision_reference() {
    for p in [4, 8, 14, 18] {
        let mut sketch = Sketch::with_precision(p).unwrap();
        let mut reference = vec![0u16; 1 << p];
        for value in 0..10_000u64 {
            let hash = Sketch::hash_value(value);
            let index_bits = hash as u64;
            let index = (index_bits >> (64 - p)) as usize;
            let remaining = index_bits & (u64::MAX >> p);
            let rank = if remaining == 0 {
                65 - p
            } else {
                remaining.leading_zeros() - p + 1
            };
            let candidate = ((rank as u16) << 10) | ((hash >> 64) as u16 & 1023);
            reference[index] = reference[index].max(candidate);
            sketch.add(value);
        }
        assert_eq!(sketch.state(), reference);
        let mut left = Sketch::with_precision(p).unwrap();
        let mut right = Sketch::with_precision(p).unwrap();
        left.add_many(0..5_000u64);
        right.add_many(5_000..10_000u64);
        left.merge(&right).unwrap();
        assert_eq!(left, sketch);
    }
}

#[test]
fn prehashed_inputs_rank_boundaries_and_incremental_hashing() {
    for p in [4, 10, 14, 18] {
        let mut sketch = Sketch::with_precision(p).unwrap();
        sketch.add_hash(0);
        assert_eq!(sketch.state()[0], ((65 - p) as u16) << 10);
        sketch.add_hash(u128::MAX);
        assert_eq!(
            sketch.state()[sketch.register_count() - 1],
            (1 << 10) | 1023
        );
        let mut entry = Entry::new();
        entry.add_bytes(b"abc").add_bytes(b"def");
        let mut streamed = Sketch::with_precision(p).unwrap();
        streamed.add_entry(&entry);
        let mut direct = Sketch::with_precision(p).unwrap();
        direct.add_bytes(b"abcdef");
        assert_eq!(streamed, direct);
        let mut read = Sketch::with_precision(p).unwrap();
        assert_eq!(read.add_reader(&b"abcdef"[..]).unwrap(), 6);
        assert_eq!(read, direct);
    }
}

#[test]
fn mismatch_errors_are_explicit_and_do_not_modify_the_receiver() {
    let mut left = Sketch::with_precision(10).unwrap();
    left.add_many(0..1_000u64);
    let before = left.clone();
    let right = Sketch::with_precision(12).unwrap();
    let error = HyperMinHashError::IncompatiblePrecision {
        left: 10,
        right: 12,
    };
    assert_eq!(left.merge(&right), Err(error));
    assert_eq!(left, before);
    assert_eq!(left.merged(&right), Err(error));
    assert_eq!(left.compare(&right), Err(error));
    assert_eq!(left.try_similarity(&right), Err(error));
    assert_eq!(left.try_intersection(&right), Err(error));
}

#[test]
fn validates_restored_state_and_reuses_its_allocation() {
    assert!(Sketch::wrap(4, vec![0; 15]).is_err());
    for invalid in [1, 1023, u16::MAX] {
        let mut registers = vec![0; 16];
        registers[0] = invalid;
        assert!(matches!(
            Sketch::wrap(4, registers),
            Err(HyperMinHashError::InvalidRegister { .. })
        ));
    }
    let mut sketch = Sketch::with_precision(12).unwrap();
    sketch.add_many(0..100u64);
    let registers = sketch.clone().into_state();
    let pointer = registers.as_ptr();
    let mut restored = Sketch::wrap(12, registers).unwrap();
    assert_eq!(restored.state().as_ptr(), pointer);
    assert_eq!(restored, sketch);
    restored.reset();
    assert!(restored.is_empty());
    assert_eq!(restored.state().as_ptr(), pointer);
}

#[test]
fn overlap_counters_work_above_65535_registers() {
    let p = 17;
    let left = Sketch::wrap(p, vec![1 << 10; 1 << p]).unwrap();
    let mut registers = left.clone().into_state();
    registers[0] = 2 << 10;
    let right = Sketch::wrap(p, registers).unwrap();
    let comparison = left.compare(&right).unwrap();
    assert_eq!(comparison.occupied_registers, 1 << p);
    assert_eq!(comparison.matching_registers, (1 << p) - 1);
    assert!(comparison.similarity.is_finite());
    assert!(comparison.intersection <= comparison.left_cardinality);
}

#[test]
fn statistical_accuracy_across_precisions_and_cardinalities() {
    for p in [6, 10, 14] {
        for n in [10u64, 1_000, 100_000] {
            let mut squared_error = 0.0;
            for trial in 0..12u64 {
                let mut sketch = Sketch::with_precision(p).unwrap();
                for value in 0..n {
                    sketch.add_with_seed(value, trial);
                }
                squared_error += (sketch.cardinality() / n as f64 - 1.0).powi(2);
            }
            let rms = (squared_error / 12.0).sqrt();
            let tolerance = 3.0 * 1.04 / ((1u64 << p) as f64).sqrt();
            assert!(
                rms < tolerance,
                "p={p}, n={n}, rms={rms}, tolerance={tolerance}"
            );
        }
    }
}

#[test]
fn comparison_estimates_are_symmetric_bounded_and_handle_empty_sets() {
    for p in [8, 12, 16] {
        let mut left = Sketch::with_precision(p).unwrap();
        let mut right = Sketch::with_precision(p).unwrap();
        left.add_many(0..10_000u64);
        right.add_many(5_000..15_000u64);
        let a = left.compare(&right).unwrap();
        let b = right.compare(&left).unwrap();
        assert_eq!(a.similarity.to_bits(), b.similarity.to_bits());
        assert_eq!(a.intersection.to_bits(), b.intersection.to_bits());
        let tolerance = 4.0 / ((1u64 << p) as f64).sqrt();
        assert!((a.similarity - 1.0 / 3.0).abs() < tolerance);
        assert!((a.intersection / 5_000.0 - 1.0).abs() < tolerance * 3.0);
        assert_eq!(left.similarity(&left), 1.0);
        assert_eq!(left.intersection(&left), left.cardinality());
        let empty = Sketch::with_precision(p).unwrap();
        assert_eq!(left.similarity(&empty), 0.0);
        assert_eq!(left.intersection(&empty), 0.0);
        assert_eq!(empty.similarity(&empty), 1.0);
        assert_eq!(empty.intersection(&empty), 0.0);
    }
}
