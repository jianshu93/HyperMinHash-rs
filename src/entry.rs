use std::{fmt, hash::Hash, io};

/// Incremental xxh3-128 hashing for a single object assembled from multiple parts.
#[derive(Clone, Default)]
pub struct Entry {
    hasher: xxhash_rust::xxh3::Xxh3Default,
}

impl Entry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, value: impl Hash) -> &mut Self {
        value.hash(&mut self.hasher);
        self
    }
    /// Append raw bytes. Chunk boundaries are not included in the hash.
    pub fn add_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.hasher.update(bytes);
        self
    }
    pub fn add_reader(&mut self, mut reader: impl io::Read) -> io::Result<u64> {
        io::copy(&mut reader, &mut self.hasher)
    }
    #[must_use]
    pub fn digest(&self) -> u128 {
        self.hasher.digest128()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.digest() == Self::default().digest()
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.digest() == other.digest()
    }
}
impl Eq for Entry {}
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("digest", &self.digest())
            .finish()
    }
}
