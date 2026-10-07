use crate::{MAX_P, Sketch, register_count};
use serde::de::{Error, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Serialize)]
struct SketchRef<'a> {
    version: u8,
    p: u32,
    registers: &'a [u16],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SketchState {
    #[serde(deserialize_with = "read_version")]
    version: u8,
    #[serde(deserialize_with = "read_precision")]
    p: u32,
    #[serde(deserialize_with = "read_registers")]
    registers: Vec<u16>,
}

fn read_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let version = u8::deserialize(deserializer)?;
    if version == 1 {
        Ok(version)
    } else {
        Err(D::Error::custom("unsupported HyperMinHash version"))
    }
}

fn read_precision<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let p = u32::deserialize(deserializer)?;
    register_count(p).map_err(D::Error::custom)?;
    Ok(p)
}

impl Serialize for Sketch {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SketchRef {
            version: 1,
            p: self.p,
            registers: &self.registers,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Sketch {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let state = SketchState::deserialize(deserializer)?;
        if state.version != 1 {
            return Err(D::Error::custom("unsupported HyperMinHash version"));
        }
        Self::wrap(state.p, state.registers).map_err(D::Error::custom)
    }
}

fn read_registers<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u16>, D::Error> {
    struct RegistersVisitor;
    impl<'de> Visitor<'de> for RegistersVisitor {
        type Value = Vec<u16>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded sequence of 16-bit HyperMinHash registers")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let maximum = 1usize << MAX_P;
            if sequence.size_hint().is_some_and(|length| length > maximum) {
                return Err(A::Error::custom("register count exceeds maximum precision"));
            }
            // Grow from the actual input, never an untrusted advertised length.
            let mut registers = Vec::new();
            while let Some(register) = sequence.next_element()? {
                if registers.len() == maximum {
                    return Err(A::Error::custom("too many registers"));
                }
                registers.try_reserve(1).map_err(A::Error::custom)?;
                registers.push(register);
            }
            Ok(registers)
        }
    }
    deserializer.deserialize_seq(RegistersVisitor)
}
