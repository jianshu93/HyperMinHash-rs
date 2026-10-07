//! Heap-backed HyperMinHash sketches with runtime precision.
//!
//! Each sketch owns `2^p` packed 16-bit registers. Construction, cloning,
//! deserialization, and moves never place the register array on the stack.
//! The default precision is 14; [`Sketch::with_precision`] accepts 4 through 26.
//!
//! ```
//! use hyperminhash::Sketch;
//!
//! let mut left = Sketch::with_precision(12)?;
//! let mut right = Sketch::with_precision(12)?;
//! left.add_many(0..10_000u64);
//! right.add_many(5_000..15_000u64);
//! let comparison = left.compare(&right)?;
//! assert!((comparison.similarity - 1.0 / 3.0).abs() < 0.05);
//! left.merge(&right)?;
//! assert!((left.cardinality() / 15_000.0 - 1.0).abs() < 0.05);
//! # Ok::<(), hyperminhash::HyperMinHashError>(())
//! ```
//!
//! Use the same hash function, seed, and input encoding for sketches that will
//! be compared or merged. Precision mismatches are rejected by the checked APIs.

#![forbid(unsafe_code)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;

mod entry;
mod estimation;
mod io;
#[cfg(feature = "serialize")]
mod serde_support;

use std::{fmt, hash::Hash};

pub use entry::Entry;

/// Smallest supported register-index precision.
pub const MIN_P: u32 = 4;
/// Largest supported precision (128 MiB of register storage).
pub const MAX_P: u32 = 26;
/// Precision used by [`Sketch::new`] and [`Default`].
pub const DEFAULT_P: u32 = 14;
/// Number of bits reserved for the rank in a register.
pub const RANK_BITS: u32 = 6;
/// Number of bits reserved for the signature in a register.
pub const SIGNATURE_BITS: u32 = 10;
/// Bytes in the self-describing binary header.
pub const HEADER_SIZE: usize = 8;
/// Serialized size at the default precision, including the header.
pub const SERIALIZED_SIZE: usize = HEADER_SIZE + (2 << DEFAULT_P);
const SIGNATURE_MASK: u16 = (1 << SIGNATURE_BITS) - 1;

/// Errors in parameters, restored state, or operations between sketches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HyperMinHashError {
    InvalidPrecision(u32),
    InvalidErrorRate,
    InvalidStateLength { expected: usize, actual: usize },
    InvalidRegister { index: usize, value: u16 },
    IncompatiblePrecision { left: u32, right: u32 },
}

impl fmt::Display for HyperMinHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidPrecision(p) => write!(f, "precision {p} must be in {MIN_P}..={MAX_P}"),
            Self::InvalidErrorRate => {
                f.write_str("requested error rate is invalid or below the supported range")
            }
            Self::InvalidStateLength { expected, actual } => {
                write!(f, "expected {expected} registers, received {actual}")
            }
            Self::InvalidRegister { index, value } => {
                write!(f, "invalid register {value} at index {index}")
            }
            Self::IncompatiblePrecision { left, right } => {
                write!(f, "precisions differ: {left} and {right}")
            }
        }
    }
}

impl std::error::Error for HyperMinHashError {}

/// A small owning handle to a heap-allocated register vector.
///
/// The register layout matches the original Rust implementation: six rank
/// bits followed by ten signature bits. Higher packed values win during updates
/// and merges. The cardinality and collision estimators support runtime precision.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Sketch {
    p: u32,
    registers: Vec<u16>,
}

/// Descriptive alias for [`Sketch`].
pub type HyperMinHash = Sketch;

impl Default for Sketch {
    fn default() -> Self {
        Self::with_precision(DEFAULT_P).expect("default precision is valid")
    }
}

impl fmt::Debug for Sketch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sketch")
            .field("p", &self.p)
            .field("register_count", &self.registers.len())
            .field("cardinality", &self.cardinality())
            .finish()
    }
}

/// Estimates calculated together in one pass over two sketches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub left_cardinality: f64,
    pub right_cardinality: f64,
    pub union_cardinality: f64,
    pub intersection: f64,
    pub similarity: f64,
    pub matching_registers: usize,
    pub occupied_registers: usize,
}

impl Sketch {
    /// Construct an empty sketch at [`DEFAULT_P`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an empty sketch with `2^p` registers allocated on the heap.
    pub fn with_precision(p: u32) -> Result<Self, HyperMinHashError> {
        let count = register_count(p)?;
        Ok(Self {
            p,
            registers: vec![0; count],
        })
    }

    /// Choose the smallest supported precision meeting a nominal HLL error rate.
    ///
    /// `1.04 / sqrt(2^p)` is a statistical standard error for cardinality,
    /// not a per-estimate bound or a Jaccard error guarantee.
    pub fn with_error_rate(error: f64) -> Result<Self, HyperMinHashError> {
        if !error.is_finite() || error <= 0.0 {
            return Err(HyperMinHashError::InvalidErrorRate);
        }
        let p = (2.0 * (1.04 / error).log2()).ceil().max(MIN_P as f64);
        if p > MAX_P as f64 {
            return Err(HyperMinHashError::InvalidErrorRate);
        }
        Self::with_precision(p as u32)
    }

    /// Restore caller-owned registers after validating their length and ranks.
    pub fn wrap(p: u32, registers: Vec<u16>) -> Result<Self, HyperMinHashError> {
        let expected = register_count(p)?;
        if registers.len() != expected {
            return Err(HyperMinHashError::InvalidStateLength {
                expected,
                actual: registers.len(),
            });
        }
        for (index, &value) in registers.iter().enumerate() {
            if !valid_register(p, value) {
                return Err(HyperMinHashError::InvalidRegister { index, value });
            }
        }
        Ok(Self { p, registers })
    }

    #[must_use]
    pub fn p(&self) -> u32 {
        self.p
    }
    #[must_use]
    pub fn register_count(&self) -> usize {
        self.registers.len()
    }
    #[must_use]
    pub fn state(&self) -> &[u16] {
        &self.registers
    }
    #[must_use]
    pub fn into_state(self) -> Vec<u16> {
        self.registers
    }
    #[must_use]
    pub fn memory_size(&self) -> usize {
        self.registers.len() * size_of::<u16>()
    }
    #[must_use]
    pub fn expected_relative_error(&self) -> f64 {
        1.04 / (self.registers.len() as f64).sqrt()
    }

    pub fn reset(&mut self) -> &mut Self {
        self.registers.fill(0);
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registers.iter().all(|&register| register == 0)
    }

    /// Hash a Rust value with xxh3-128. Rust's `Hash` encoding is not a portable wire format.
    #[must_use]
    pub fn hash_value(value: impl Hash) -> u128 {
        let mut hasher = xxhash_rust::xxh3::Xxh3Default::new();
        value.hash(&mut hasher);
        hasher.digest128()
    }

    /// Add a uniformly distributed 128-bit hash supplied by the caller.
    pub fn add_hash(&mut self, hash: u128) -> &mut Self {
        let index_hash = hash as u64;
        let index = (index_hash >> (64 - self.p)) as usize;
        let sentinel = (1u64 << self.p) - 1;
        let rank = ((index_hash << self.p) | sentinel).leading_zeros() + 1;
        let signature = (hash >> 64) as u16 & SIGNATURE_MASK;
        let register = ((rank as u16) << SIGNATURE_BITS) | signature;
        self.registers[index] = self.registers[index].max(register);
        self
    }

    pub fn add_hashes(&mut self, hashes: impl IntoIterator<Item = u128>) -> &mut Self {
        for hash in hashes {
            self.add_hash(hash);
        }
        self
    }

    pub fn add(&mut self, value: impl Hash) -> &mut Self {
        self.add_hash(Self::hash_value(value))
    }

    pub fn add_many<T: Hash>(&mut self, values: impl IntoIterator<Item = T>) -> &mut Self {
        let mut hasher = xxhash_rust::xxh3::Xxh3Default::new();
        for value in values {
            value.hash(&mut hasher);
            self.add_hash(hasher.digest128());
            hasher.reset();
        }
        self
    }

    /// Hash raw bytes without Rust's `Hash` length/prefix encoding.
    pub fn add_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.add_hash(xxhash_rust::xxh3::xxh3_128(bytes))
    }

    pub fn add_with_seed(&mut self, value: impl Hash, seed: u64) -> &mut Self {
        let mut hasher = xxhash_rust::xxh3::Xxh3::with_seed(seed);
        value.hash(&mut hasher);
        self.add_hash(hasher.digest128())
    }

    pub fn add_bytes_with_seed(&mut self, bytes: &[u8], seed: u64) -> &mut Self {
        self.add_hash(xxhash_rust::xxh3::xxh3_128_with_seed(bytes, seed))
    }

    pub fn add_with_hash_fn<T, F: FnOnce(&T) -> u128>(
        &mut self,
        value: T,
        hash_fn: F,
    ) -> &mut Self {
        self.add_hash(hash_fn(&value))
    }

    pub fn add_entry(&mut self, entry: &Entry) -> &mut Self {
        self.add_hash(entry.digest())
    }

    pub fn add_reader(&mut self, reader: impl std::io::Read) -> std::io::Result<u64> {
        self.add_reader_with_seed(reader, 0)
    }

    pub fn add_reader_with_seed(
        &mut self,
        mut reader: impl std::io::Read,
        seed: u64,
    ) -> std::io::Result<u64> {
        let mut hasher = xxhash_rust::xxh3::Xxh3::with_seed(seed);
        let count = std::io::copy(&mut reader, &mut hasher)?;
        self.add_hash(hasher.digest128());
        Ok(count)
    }

    /// Estimate cardinality using Ertl's improved raw HLL estimator.
    #[must_use]
    pub fn cardinality(&self) -> f64 {
        estimation::cardinality(self.p, &estimation::histogram(&self.registers))
    }

    /// Merge a sketch of the same precision; errors leave the receiver unchanged.
    pub fn merge(&mut self, other: &Self) -> Result<&mut Self, HyperMinHashError> {
        self.check_precision(other)?;
        for (left, &right) in self.registers.iter_mut().zip(&other.registers) {
            *left = (*left).max(right);
        }
        Ok(self)
    }

    /// Convenience merge for existing code.
    ///
    /// # Panics
    /// Panics if precisions differ. Use [`Sketch::merge`] for a checked operation.
    pub fn union(&mut self, other: &Self) -> &mut Self {
        self.merge(other)
            .expect("cannot union sketches with different precisions")
    }

    /// Return the union as a new heap-backed sketch.
    pub fn merged(&self, other: &Self) -> Result<Self, HyperMinHashError> {
        self.check_precision(other)?;
        let mut result = self.clone();
        result.merge(other)?;
        Ok(result)
    }

    /// Estimate cardinalities, corrected Jaccard similarity, and intersection together.
    pub fn compare(&self, other: &Self) -> Result<Comparison, HyperMinHashError> {
        self.compare_inner(other, false)
    }

    pub fn try_similarity(&self, other: &Self) -> Result<f64, HyperMinHashError> {
        Ok(self.compare(other)?.similarity)
    }
    pub fn try_intersection(&self, other: &Self) -> Result<f64, HyperMinHashError> {
        Ok(self.compare(other)?.intersection)
    }

    /// Corrected Jaccard estimate. Panics on a precision mismatch; see [`Sketch::try_similarity`].
    #[must_use]
    pub fn similarity(&self, other: &Self) -> f64 {
        self.try_similarity(other)
            .expect("cannot compare sketches with different precisions")
    }

    /// Corrected intersection estimate. Panics on a precision mismatch; see [`Sketch::try_intersection`].
    #[must_use]
    pub fn intersection(&self, other: &Self) -> f64 {
        self.try_intersection(other)
            .expect("cannot compare sketches with different precisions")
    }

    /// Uncorrected Jaccard estimate; faster but includes accidental fingerprint matches.
    /// Panics on a precision mismatch.
    #[must_use]
    pub fn similarity_fast(&self, other: &Self) -> f64 {
        self.check_precision(other)
            .expect("cannot compare sketches with different precisions");
        let (matches, occupied) = self.overlap_counts(other);
        if occupied == 0 {
            1.0
        } else {
            matches as f64 / occupied as f64
        }
    }

    /// Uncorrected intersection estimate. Panics on a precision mismatch.
    #[must_use]
    pub fn intersection_fast(&self, other: &Self) -> f64 {
        self.compare_inner(other, true)
            .expect("cannot compare sketches with different precisions")
            .intersection
    }

    pub fn similarity_many<'a, I>(&'a self, others: I) -> BatchIter<'a, I::IntoIter>
    where
        I: IntoIterator<Item = &'a Self>,
    {
        BatchIter {
            left: self,
            others: others.into_iter(),
            operation: Self::similarity,
        }
    }
    pub fn similarity_many_fast<'a, I>(&'a self, others: I) -> BatchIter<'a, I::IntoIter>
    where
        I: IntoIterator<Item = &'a Self>,
    {
        BatchIter {
            left: self,
            others: others.into_iter(),
            operation: Self::similarity_fast,
        }
    }
    pub fn intersection_many<'a, I>(&'a self, others: I) -> BatchIter<'a, I::IntoIter>
    where
        I: IntoIterator<Item = &'a Self>,
    {
        BatchIter {
            left: self,
            others: others.into_iter(),
            operation: Self::intersection,
        }
    }
    pub fn intersection_many_fast<'a, I>(&'a self, others: I) -> BatchIter<'a, I::IntoIter>
    where
        I: IntoIterator<Item = &'a Self>,
    {
        BatchIter {
            left: self,
            others: others.into_iter(),
            operation: Self::intersection_fast,
        }
    }

    fn check_precision(&self, other: &Self) -> Result<(), HyperMinHashError> {
        if self.p == other.p {
            Ok(())
        } else {
            Err(HyperMinHashError::IncompatiblePrecision {
                left: self.p,
                right: other.p,
            })
        }
    }

    fn overlap_counts(&self, other: &Self) -> (usize, usize) {
        let mut matches = 0;
        let mut occupied = 0;
        for (&left, &right) in self.registers.iter().zip(&other.registers) {
            matches += usize::from(left != 0 && left == right);
            occupied += usize::from((left | right) != 0);
        }
        (matches, occupied)
    }

    fn compare_inner(&self, other: &Self, fast: bool) -> Result<Comparison, HyperMinHashError> {
        self.check_precision(other)?;
        let mut left_histogram = [0usize; 64];
        let mut right_histogram = [0usize; 64];
        let mut union_histogram = [0usize; 64];
        let mut matching_registers = 0;
        let mut occupied_registers = 0;
        let mut identical = true;
        for (&left, &right) in self.registers.iter().zip(&other.registers) {
            left_histogram[(left >> SIGNATURE_BITS) as usize] += 1;
            right_histogram[(right >> SIGNATURE_BITS) as usize] += 1;
            union_histogram[(left.max(right) >> SIGNATURE_BITS) as usize] += 1;
            matching_registers += usize::from(left != 0 && left == right);
            occupied_registers += usize::from((left | right) != 0);
            identical &= left == right;
        }
        let left_cardinality = estimation::cardinality(self.p, &left_histogram);
        let right_cardinality = estimation::cardinality(self.p, &right_histogram);
        let union_cardinality = estimation::cardinality(self.p, &union_histogram);
        let similarity = if identical {
            1.0
        } else if matching_registers == 0 {
            0.0
        } else {
            let collisions = if fast {
                0.0
            } else {
                estimation::expected_collisions(self.p, left_cardinality, right_cardinality)
            };
            ((matching_registers as f64 - collisions) / occupied_registers as f64).clamp(0.0, 1.0)
        };
        let intersection = if identical {
            left_cardinality
        } else if similarity == 0.0 {
            0.0
        } else {
            (similarity * union_cardinality).min(left_cardinality.min(right_cardinality))
        };
        Ok(Comparison {
            left_cardinality,
            right_cardinality,
            union_cardinality,
            intersection,
            similarity,
            matching_registers,
            occupied_registers,
        })
    }
}

impl<T: Hash> Extend<T> for Sketch {
    fn extend<I: IntoIterator<Item = T>>(&mut self, values: I) {
        self.add_many(values);
    }
}
impl<T: Hash> FromIterator<T> for Sketch {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut result = Self::new();
        result.extend(values);
        result
    }
}

/// Lazy batch comparisons, with exactly the same results as the pairwise methods.
pub struct BatchIter<'a, I: Iterator<Item = &'a Sketch>> {
    left: &'a Sketch,
    others: I,
    operation: fn(&Sketch, &Sketch) -> f64,
}
impl<'a, I: Iterator<Item = &'a Sketch>> Iterator for BatchIter<'a, I> {
    type Item = f64;
    fn next(&mut self) -> Option<Self::Item> {
        self.others
            .next()
            .map(|other| (self.operation)(self.left, other))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.others.size_hint()
    }
}
impl<'a, I: ExactSizeIterator<Item = &'a Sketch>> ExactSizeIterator for BatchIter<'a, I> {}
impl<'a, I: std::iter::FusedIterator<Item = &'a Sketch>> std::iter::FusedIterator
    for BatchIter<'a, I>
{
}

fn register_count(p: u32) -> Result<usize, HyperMinHashError> {
    if (MIN_P..=MAX_P).contains(&p) {
        Ok(1usize << p)
    } else {
        Err(HyperMinHashError::InvalidPrecision(p))
    }
}
fn valid_register(p: u32, value: u16) -> bool {
    value == 0 || (1..=65 - p).contains(&u32::from(value >> SIGNATURE_BITS))
}
