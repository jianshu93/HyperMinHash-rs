use hyperminhash::{HEADER_SIZE, Sketch};
use std::io::{Cursor, ErrorKind};

#[test]
fn versioned_stream_round_trips_mixed_precisions_without_overreading() {
    let mut bytes = Vec::new();
    let mut sketches = Vec::new();
    for p in [4, 10, 14, 18] {
        let mut sketch = Sketch::with_precision(p).unwrap();
        sketch.add_many(0..10_000u64);
        let before = bytes.len();
        sketch.save(&mut bytes).unwrap();
        assert_eq!(bytes.len() - before, sketch.serialized_size());
        sketches.push(sketch);
    }
    let mut reader = Cursor::new(&bytes);
    for expected in sketches {
        assert_eq!(Sketch::load(&mut reader).unwrap(), expected);
    }
    assert_eq!(reader.position() as usize, bytes.len());
}

#[test]
fn rejects_bad_headers_truncation_invalid_ranks_and_excessive_precision() {
    let sketch = Sketch::with_precision(4).unwrap();
    let mut bytes = Vec::new();
    sketch.save(&mut bytes).unwrap();
    for (index, value) in [(0, b'X'), (4, 2), (5, 3), (5, 27), (6, 5), (7, 9)] {
        let mut invalid = bytes.clone();
        invalid[index] = value;
        assert_eq!(
            Sketch::load(invalid.as_slice()).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
    for length in [0, HEADER_SIZE - 1, bytes.len() - 1] {
        assert_eq!(
            Sketch::load(&bytes[..length]).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }
    bytes[HEADER_SIZE..HEADER_SIZE + 2].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        Sketch::load(bytes.as_slice()).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    let header = [b'H', b'M', b'H', 0, 1, 26, 6, 10];
    assert_eq!(
        Sketch::load_with_max_precision(&header[..], 14)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        Sketch::load(&[0u8; HEADER_SIZE][..]).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}
