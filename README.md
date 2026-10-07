# HyperMinHash-rs🦀

Heap-backed HyperMinHash sketches with runtime precision for cardinality,
Jaccard similarity, intersection, and union estimation.

Every sketch owns a `Vec<u16>`. Construction, cloning, loading, and Serde
deserialization allocate register storage on the heap. Moving a sketch moves
its small owning handle, not its register array. No `heap` feature, `Box<Sketch>`,
or special initializer is needed.

The default precision is `P = 14`. `Sketch::with_precision(p)` accepts
`4..=26`, with `2^p` registers and two bytes per register. The packed register
layout retains six rank bits and ten signature bits; those widths are fixed.

## Installation

For another local project:

```toml
[dependencies]
hyperminhash = { package = "hyperminhash-rs", path = "../HyperMinHash-rs" }
```

The package is named `hyperminhash-rs`; its Rust library name remains
`hyperminhash`. `HyperMinHash` is an alias for `Sketch`.

## Basic API

```rust
use hyperminhash::{HyperMinHashError, Sketch};

fn main() -> Result<(), HyperMinHashError> {
    let mut left = Sketch::with_precision(12)?;
    let mut right = Sketch::with_precision(12)?;
    left.add_many(0..10_000u64);
    right.add_many(5_000..15_000u64);

    let comparison = left.compare(&right)?;
    println!("left: {}", comparison.left_cardinality);
    println!("union: {}", comparison.union_cardinality);
    println!("intersection: {}", comparison.intersection);
    println!("Jaccard: {}", comparison.similarity);

    left.merge(&right)?;
    assert_eq!(left.p(), 12);
    left.reset();
    assert!(left.is_empty());
    Ok(())
}
```

`Sketch::new()`, `Sketch::default()`, and collecting into a `Sketch` use
`P = 14`. To collect at another precision, construct with `with_precision`
and call `extend` or `add_many`.

`merge`, `merged`, `compare`, `try_similarity`, and `try_intersection` return
an error when precisions differ. A failed merge leaves its receiver unchanged.
Convenience methods `union`, `similarity`, `intersection`, and their fast and
batch variants retain the original call style but panic on a precision mismatch.
There is no implicit precision conversion or downsampling.

## Precision and Memory

| P | Registers | Register payload | Nominal cardinality standard error |
|---|---:|---:|---:|
| 4 | 16 | 32 B | 26.0% |
| 8 | 256 | 512 B | 6.5% |
| 10 | 1,024 | 2 KiB | 3.25% |
| 12 | 4,096 | 8 KiB | 1.625% |
| 14 | 16,384 | 32 KiB | 0.8125% |
| 16 | 65,536 | 128 KiB | 0.40625% |
| 20 | 1,048,576 | 2 MiB | 0.1016% |
| 26 | 67,108,864 | 128 MiB | 0.0127% |

`with_error_rate(error)` selects the smallest supported precision satisfying
the nominal cardinality standard error `1.04 / sqrt(2^p)`. This is not a
guaranteed error bound for an individual estimate, and it is not a Jaccard
relative-error bound. Small intersections and low similarities need particular
care when choosing precision.

`p`, `register_count`, `memory_size`, and `expected_relative_error` expose
the configuration. `memory_size` reports the logical register payload, excluding
the owning handle, allocator overhead, and any spare vector capacity.
`state` borrows the registers, `into_state` transfers the vector, and
`wrap(p, registers)` validates and reuses caller-owned storage.

## Hashing

`add` and `add_many` hash Rust `Hash` values using xxh3-128. `add_bytes` hashes
raw bytes; it deliberately differs from `add` on a byte slice because Rust's
`Hash` includes input-encoding details. Use `add_hash` or `add_hashes` for
uniformly distributed caller-supplied 128-bit hashes. `add_with_hash_fn` accepts
a custom one-shot 128-bit hash function.

Seeded methods are available for Rust values, bytes, and readers. `Entry`
supports incremental hashing of an object assembled from multiple parts.
`add_reader` streams a reader as one object without retaining its contents.

Compared or merged sketches must use the same hash function, seed, and input
encoding. Seeds and hash identities are not stored in the sketch. For portable
datasets, use explicitly encoded bytes or stable precomputed hashes rather
than relying on Rust's platform-dependent `Hash` encoding.

## Estimators

Cardinality uses Ertl's improved raw HyperLogLog estimator with small-range
and saturation corrections, rather than the original P=14-specific LogLog-Beta
polynomial. Rank histograms use `usize` counters, including above 65,535 registers.

Jaccard estimation uses HyperMinHash's approximate collision correction with
the current precision. Small sets use rank-collision probabilities divided by
the signature space; larger sets use the paper's asymptotic approximation.
The correction is approximate, not an exact unbiased guarantee for every input.
It requires no large collision tables or per-sketch estimator caches.

`compare` computes both cardinalities, union cardinality, similarity, and
intersection in one register pass. Similarity is clamped to `[0, 1]` and
intersection to the smaller estimated input cardinality. Two identical sketches,
including two empty sketches, have similarity 1. Empty/nonempty pairs have
similarity and intersection 0. A fully rank-saturated sketch has infinite
estimated cardinality because the state no longer resolves a finite count.

`similarity_fast` and `intersection_fast` omit collision correction. Batch
methods are lazy and match the corresponding pairwise methods bit for bit.

## Binary I/O

`save` and `load` are always available:

```rust
use hyperminhash::Sketch;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sketch = Sketch::with_precision(10)?;
    sketch.add_many(0..1_000u64);
    let mut bytes = Vec::new();
    sketch.save(&mut bytes)?;
    let restored = Sketch::load(bytes.as_slice())?;
    assert_eq!(restored, sketch);
    Ok(())
}
```

The format is an eight-byte header followed by `2^p` little-endian `u16`
registers. Header bytes are `HMH\0`, version 1, P, rank width 6, and signature
width 10. `serialized_size()` includes the header. `SERIALIZED_SIZE` is the
size at P=14 only; it is not a capacity bound for every precision.

Readers consume exactly one sketch, so streams may contain multiple sketches
with different precisions. Headers, precision, truncation, and register ranks
are validated. `load_with_max_precision(reader, maximum)` rejects a precision
above an application-selected memory budget before allocating its registers.
Without that limit, a valid P=26 header requests 128 MiB of register storage.
Use buffered readers and writers for expensive I/O and explicitly flush writers.

Only the versioned format is supported. Regenerate sketches written by older
implementations before loading them with this library.

## Serde

Enable `serde` for JSON, bincode, and other Serde formats. The `serialize` feature is an alias for `serde`; storage is always heap-backed.

```toml
hyperminhash = { package = "hyperminhash-rs", path = "../HyperMinHash-rs", features = ["serde"] }
serde_json = "1"
```

The representation contains `version`, `p`, and `registers`. Restored metadata,
register count, and register ranks are validated. Register sequences are bounded
by MAX_P and grow from actual input rather than allocating from an untrusted
length hint. Serde formats have their own encoding and do not necessarily match
`save`/`load` bytes.

## Parallel Applications

A `Vec<Sketch>` holds small owning handles; each register vector is separately
allocated. Rayon can construct, move, clone, and collect sketches without
stack-sized register arrays. Each sketch still costs an allocation, and total
heap consumption grows with both sample count and precision. For example,
10,000 P=14 samples require 312.5 MiB of register payload before overhead.

`cargo run --release --example parallel -- 12 file1.txt file2.txt` counts
unique lines in multiple files and merges the sketches.

## Validation

```sh
cargo test
cargo test --features serde
cargo test --features serialize
cargo test --release --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Tests cover runtime precision, exact union state, register validation,
mixed-precision serialization streams, malformed input, seeded accuracy trials,
batch/pairwise parity, and lifecycle operations on 64 KiB thread and Rayon worker
stacks. The Rayon tests include 16,384 small sketches and 2,048 P=14 sketches.
These are regression checks, not a guarantee against every possible stack
overflow elsewhere in an application.

## References and Attribution

- [HyperMinHash: MinHash in LogLog space, Yu and Weber](https://arxiv.org/abs/1710.08436)
- [New cardinality estimation algorithms for HyperLogLog sketches, Ertl](https://arxiv.org/abs/1702.01284)
- [Original Go implementation](https://github.com/axiomhq/hyperminhash)
- [Original Rust implementation by Lukas Lueg](https://github.com/lukaslueg/hyperminhash)

This implementation rewrites the core around dynamic heap storage and runtime
precision while retaining the original Rust register encoding and default hash
mapping. Existing copyright attribution is retained under the MIT license.
