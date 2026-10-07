#![cfg(feature = "serde")]
use hyperminhash::Sketch;

#[test]
fn json_and_bincode_preserve_precision_and_registers() {
    let mut sketches = Vec::new();
    for p in [4, 10, 14, 18] {
        let mut sketch = Sketch::with_precision(p).unwrap();
        sketch.add_many(0..1_000u64);
        let json = serde_json::to_string(&sketch).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["p"], p);
        assert_eq!(value["registers"].as_array().unwrap().len(), 1 << p);
        assert_eq!(serde_json::from_str::<Sketch>(&json).unwrap(), sketch);
        assert_eq!(
            bincode::deserialize::<Sketch>(&bincode::serialize(&sketch).unwrap()).unwrap(),
            sketch
        );
        sketches.push(sketch);
    }
    let bytes = bincode::serialize(&sketches).unwrap();
    assert_eq!(
        bincode::deserialize::<Vec<Sketch>>(&bytes).unwrap(),
        sketches
    );
}

#[test]
fn rejects_invalid_metadata_lengths_and_register_values() {
    let valid = serde_json::to_value(Sketch::with_precision(4).unwrap()).unwrap();
    for (field, value) in [
        ("p", serde_json::json!(3)),
        ("p", serde_json::json!(27)),
        ("version", serde_json::json!(2)),
        ("registers", serde_json::json!([])),
        ("registers", serde_json::json!(vec![0; 17])),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<Sketch>(invalid).is_err());
    }
    for value in [1, 65535, 65536, -1] {
        let mut invalid = valid.clone();
        invalid["registers"][0] = serde_json::json!(value);
        assert!(serde_json::from_value::<Sketch>(invalid).is_err());
    }
    let mut unknown = valid;
    unknown["extra"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Sketch>(unknown).is_err());
    let mut bytes = bincode::serialize(&Sketch::with_precision(4).unwrap()).unwrap();
    bytes[5..13].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(bincode::deserialize::<Sketch>(&bytes).is_err());
}
