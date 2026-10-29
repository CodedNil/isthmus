//! Lossless solved fixture responses. No ray paths or temporal state in this asset.
use super::{Irradiance, cache::Cache};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use isthmus::glam::Vec3;
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

pub struct Controls {
    pub active: Box<[usize]>,
    pub responses: BTreeMap<Vec<u32>, Box<[Irradiance]>>,
}

fn number(input: &mut impl Read) -> Result<u32, io::Error> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

impl Controls {
    pub(crate) fn decode(bytes: &[u8], signature: u64) -> Result<Self, &'static str> {
        let mut input = GzDecoder::new(bytes);
        let mut header = [0; 16];
        input.read_exact(&mut header).map_err(|_error| "invalid lighting asset")?;
        if &header[..8] != b"LARESL01" || header[8..] != signature.to_le_bytes() {
            return Err("lighting asset is stale; rebuild with just lares");
        }
        let mut parse = || -> Result<Self, io::Error> {
            let count = number(&mut input)? as usize;
            let groups = number(&mut input)? as usize;
            if count > 1_000_000 || groups > 256 {
                return Err(io::Error::other("oversized lighting asset"));
            }
            let active: Box<[_]> =
                (0..count).map(|_| number(&mut input).map(|value| value as usize)).collect::<Result<_, _>>()?;
            let mut responses = BTreeMap::new();
            for _ in 0..groups {
                let fixtures = number(&mut input)? as usize;
                if fixtures > 256 {
                    return Err(io::Error::other("oversized lighting group"));
                }
                let slots: Vec<_> = (0..fixtures).map(|_| number(&mut input)).collect::<Result<_, _>>()?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    let mut vector = || -> Result<Vec3, io::Error> {
                        Ok(Vec3::new(
                            f32::from_bits(number(&mut input)?),
                            f32::from_bits(number(&mut input)?),
                            f32::from_bits(number(&mut input)?),
                        ))
                    };
                    values.push(Irradiance { constant: vector()?, x: vector()?, y: vector()?, z: vector()? });
                }
                responses.insert(slots, values.into_boxed_slice());
            }
            let mut tail = [0];
            if input.read(&mut tail)? != 0 {
                return Err(io::Error::other("oversized lighting payload"));
            }
            Ok(Self { active, responses })
        };
        parse().map_err(|_error| "corrupt lighting asset")
    }
}

impl Cache {
    pub(crate) fn encode_controls(&self, signature: u64) -> Result<Vec<u8>, io::Error> {
        let mut output = GzEncoder::new(Vec::new(), Compression::best());
        output.write_all(b"LARESL01")?;
        output.write_all(&signature.to_le_bytes())?;
        output.write_all(&(self.active.len() as u32).to_le_bytes())?;
        output.write_all(&(self.responses.len() as u32).to_le_bytes())?;
        for &index in &self.active {
            output.write_all(&(index as u32).to_le_bytes())?;
        }
        for (slots, values) in &self.responses {
            output.write_all(&(slots.len() as u32).to_le_bytes())?;
            for slot in slots {
                output.write_all(&slot.to_le_bytes())?;
            }
            for value in values {
                for vector in [value.constant, value.x, value.y, value.z] {
                    for channel in vector.to_array() {
                        output.write_all(&channel.to_le_bytes())?;
                    }
                }
            }
        }
        output.finish()
    }
}
