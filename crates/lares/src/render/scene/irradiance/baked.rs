//! Quantized solved lighting responses and lossless receiver geometry.
use super::cache::{Cache, Packed};
use crate::{
    home::Object,
    render::{Sun, acceleration::Node},
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use isthmus::ShaderData;
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

pub struct Controls {
    pub transport: Cache,
    pub active: Box<[usize]>,
    pub responses: BTreeMap<Vec<u32>, Box<[Packed]>>,
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
        if &header[..8] != b"LARESL04" || header[8..] != signature.to_le_bytes() {
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
                    let mut words = [0; 4];
                    for word in &mut words {
                        *word = number(&mut input)?;
                    }
                    let value = Packed::read(&words, 0);
                    if !value.valid() {
                        return Err(io::Error::other("invalid lighting coefficients"));
                    }
                    values.push(value);
                }
                responses.insert(slots, values.into_boxed_slice());
            }
            let transport = Cache::decode_transport(&mut input, count)
                .ok_or_else(|| io::Error::other("invalid transport layout"))?;
            if active.iter().any(|&index| index >= transport.probe_count())
                || responses
                    .keys()
                    .any(|slots| slots.is_empty() || slots.iter().any(|&slot| slot as usize >= transport.light_count()))
            {
                return Err(io::Error::other("invalid active receiver"));
            }
            let mut tail = [0];
            if input.read(&mut tail)? != 0 {
                return Err(io::Error::other("oversized lighting payload"));
            }
            Ok(Self { transport, active, responses })
        };
        parse().map_err(|_error| "corrupt lighting asset")
    }
}

impl Cache {
    pub(crate) fn encode_controls(
        &mut self,
        signature: u64,
        sun: Sun,
        objects: &[Object],
        nodes: &[Node],
    ) -> Result<Vec<u8>, io::Error> {
        let mut output = GzEncoder::new(Vec::new(), Compression::best());
        output.write_all(b"LARESL04")?;
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
                let mut words = [0; 4];
                value.write(&mut words, 0);
                for word in words {
                    output.write_all(&word.to_le_bytes())?;
                }
            }
        }
        let baked = self.daylight.is_none().then(|| self.bake_daylight(sun, objects, nodes));
        self.encode_transport(&mut output, baked.as_ref().or(self.daylight.as_ref()).unwrap())?;
        output.finish()
    }
}
