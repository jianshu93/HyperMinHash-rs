#[test]
fn generated_sketch_round_trip_is_stable() {
    let sketch: hyperminhash::Sketch = (0..10_000u64).collect();
    let mut serialized = Vec::with_capacity(hyperminhash::SERIALIZED_SIZE);
    sketch.save(&mut serialized).unwrap();
    assert_eq!(serialized.len(), sketch.serialized_size());
    let loaded = hyperminhash::Sketch::load(serialized.as_slice()).unwrap();
    assert_eq!(loaded, sketch);
    assert_eq!(
        loaded.cardinality().to_bits(),
        sketch.cardinality().to_bits()
    );
    let mut saved = Vec::new();
    loaded.save(&mut saved).unwrap();
    assert_eq!(saved, serialized);
}
