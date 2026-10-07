use crate::{
    HEADER_SIZE, MAX_P, RANK_BITS, SIGNATURE_BITS, Sketch, register_count, valid_register,
};
use std::io::{self, Read, Write};

const MAGIC: &[u8; 4] = b"HMH\0";
const VERSION: u8 = 1;

impl Sketch {
    #[must_use]
    pub fn serialized_size(&self) -> usize {
        HEADER_SIZE + self.memory_size()
    }

    /// Write a versioned header, precision, and little-endian registers.
    /// Wrap expensive writers in `BufWriter` and explicitly flush afterward.
    pub fn save(&self, mut writer: impl Write) -> io::Result<()> {
        writer.write_all(MAGIC)?;
        writer.write_all(&[VERSION, self.p as u8, RANK_BITS as u8, SIGNATURE_BITS as u8])?;
        self.save_registers(writer)
    }
    /// Read one self-describing sketch, leaving subsequent bytes unread.
    pub fn load(reader: impl Read) -> io::Result<Self> {
        Self::load_with_max_precision(reader, MAX_P)
    }

    /// Reject an input precision above a caller-selected memory budget before allocation.
    pub fn load_with_max_precision(mut reader: impl Read, maximum: u32) -> io::Result<Self> {
        register_count(maximum).map_err(invalid_data)?;
        let mut header = [0; HEADER_SIZE];
        reader.read_exact(&mut header)?;
        if &header[..4] != MAGIC
            || header[4] != VERSION
            || header[6] != RANK_BITS as u8
            || header[7] != SIGNATURE_BITS as u8
        {
            return Err(invalid_data("unsupported HyperMinHash header"));
        }
        let p = u32::from(header[5]);
        register_count(p).map_err(invalid_data)?;
        if p > maximum {
            return Err(invalid_data(
                "precision exceeds the configured memory budget",
            ));
        }
        Self::load_registers(p, reader)
    }
    fn save_registers(&self, mut writer: impl Write) -> io::Result<()> {
        let mut bytes = [0u8; 4096];
        for chunk in self.registers.chunks(bytes.len() / 2) {
            for (index, register) in chunk.iter().enumerate() {
                bytes[index * 2..index * 2 + 2].copy_from_slice(&register.to_le_bytes());
            }
            writer.write_all(&bytes[..chunk.len() * 2])?;
        }
        Ok(())
    }
    fn load_registers(p: u32, mut reader: impl Read) -> io::Result<Self> {
        let mut sketch = Self::with_precision(p).map_err(invalid_data)?;
        let mut bytes = [0u8; 4096];
        for chunk in sketch.registers.chunks_mut(bytes.len() / 2) {
            reader.read_exact(&mut bytes[..chunk.len() * 2])?;
            for (index, register) in chunk.iter_mut().enumerate() {
                *register = u16::from_le_bytes([bytes[index * 2], bytes[index * 2 + 1]]);
                if !valid_register(p, *register) {
                    return Err(invalid_data("invalid register rank"));
                }
            }
        }
        Ok(sketch)
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
