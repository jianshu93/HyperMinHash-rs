use hyperminhash::{DEFAULT_P, Sketch};
use rayon::prelude::*;

#[test]
fn sketch_is_a_small_handle_at_every_precision() {
    assert!(size_of::<Sketch>() <= 4 * size_of::<usize>());
    for p in [4, 14, 20] {
        let sketch = Sketch::with_precision(p).unwrap();
        assert_eq!(sketch.memory_size(), 2 << p);
        assert!(sketch.is_empty());
    }
}

fn lifecycle(p: u32, value: u64) -> Sketch {
    let mut sketch = Sketch::with_precision(p).unwrap();
    sketch.add_many(value..value + 100);
    let mut cloned = sketch.clone();
    cloned.add(value + 100);
    let comparison = sketch.compare(&cloned).unwrap();
    assert!((0.0..=1.0).contains(&comparison.similarity));
    let mut bytes = Vec::new();
    sketch.save(&mut bytes).unwrap();
    assert_eq!(Sketch::load(bytes.as_slice()).unwrap(), sketch);
    #[cfg(feature = "serde")]
    {
        let encoded = bincode::serialize(&sketch).unwrap();
        assert_eq!(bincode::deserialize::<Sketch>(&encoded).unwrap(), sketch);
        let encoded = serde_json::to_string(&sketch).unwrap();
        assert_eq!(serde_json::from_str::<Sketch>(&encoded).unwrap(), sketch);
    }
    sketch
}

#[test]
fn complete_lifecycle_on_a_64_kib_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            for p in [4, DEFAULT_P, 20] {
                lifecycle(p, 0);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn many_samples_on_64_kib_rayon_worker_stacks() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .stack_size(64 * 1024)
        .build()
        .unwrap();
    pool.install(|| {
        for (p, samples) in [(6, 16_384u64), (DEFAULT_P, 2_048), (18, 32)] {
            let sketches: Vec<Sketch> = (0..samples)
                .into_par_iter()
                .map(|sample| lifecycle(p, sample * 100))
                .collect();
            assert_eq!(sketches.len(), samples as usize);
            assert!(
                sketches
                    .iter()
                    .all(|sketch| sketch.p() == p && !sketch.is_empty())
            );
            let combined = sketches.par_iter().cloned().reduce(
                || Sketch::with_precision(p).unwrap(),
                |mut left, right| {
                    left.merge(&right).unwrap();
                    left
                },
            );
            assert!(!combined.is_empty());
        }
    });
}
