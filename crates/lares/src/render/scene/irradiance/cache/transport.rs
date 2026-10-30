//! Receiver geometry and compact solved daylight; runtime has no transport rays.
use super::{
    Cache, Grid, Irradiance, Probe,
    daylight::{Daylight, Packed, SUN_FIELDS},
};
use core::array::from_fn;
use isthmus::ShaderData;
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

const CHUNK: usize = 65_536;

// Byte planes expose shared exponents and integer prefixes to DEFLATE. Chunking
// bounds scratch memory independently of the size of the transport bake.
fn append<T: ShaderData>(output: &mut impl Write, values: &[T]) -> io::Result<()> {
    output.write_all(&(values.len() as u32).to_le_bytes())?;
    for chunk in values.chunks(CHUNK) {
        let mut words = vec![0; chunk.len() * T::WORDS];
        for (index, &value) in chunk.iter().enumerate() {
            value.write(&mut words, index * T::WORDS);
        }
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for lane in 0..T::WORDS * 4 {
            for index in 0..chunk.len() {
                bytes.push(words[index * T::WORDS + lane / 4].to_le_bytes()[lane % 4]);
            }
        }
        output.write_all(&bytes)?;
    }
    Ok(())
}

struct Input<R> {
    reader: R,
    remaining: usize,
}
impl<R: Read> Input<R> {
    fn take<T: ShaderData>(&mut self) -> Option<Box<[T]>> {
        let mut count = [0; 4];
        self.reader.read_exact(&mut count).ok()?;
        let count = u32::from_le_bytes(count) as usize;
        let length = count.checked_mul(T::WORDS)?.checked_mul(4)?;
        self.remaining = self.remaining.checked_sub(length)?;
        let mut values = Vec::with_capacity(count);
        while values.len() < count {
            let count = (count - values.len()).min(CHUNK);
            let mut bytes = vec![0; count * T::WORDS * 4];
            self.reader.read_exact(&mut bytes).ok()?;
            let mut words = vec![0; count * T::WORDS];
            for lane in 0..T::WORDS * 4 {
                for index in 0..count {
                    words[index * T::WORDS + lane / 4] |= u32::from(bytes[lane * count + index]) << ((lane % 4) * 8);
                }
            }
            values.extend((0..count).map(|index| T::read(&words, index * T::WORDS)));
        }
        Some(values.into_boxed_slice())
    }
}

impl Cache {
    pub(crate) fn encode_transport(&self, output: &mut impl Write, daylight: &Daylight) -> io::Result<()> {
        append(output, &[self.grid])?;
        append(output, &self.surface_slots)?;
        let receivers: Vec<_> =
            self.direct.iter().map(|&probe| Probe { light: Irradiance::default(), ..probe }).collect();
        append(output, &receivers)?;
        append(output, &self.lights)?;
        append(output, &daylight.sky)?;
        append(output, &daylight.ground)?;
        append(output, &daylight.sun)?;
        Ok(())
    }

    pub(crate) fn decode_transport(reader: impl Read, active: usize) -> Option<Self> {
        let mut input = Input { reader, remaining: 128_000_000 };
        let grids = input.take::<Grid>()?;
        if grids.len() != 1 {
            return None;
        }
        let grid = grids[0];
        if grid.size.min_element() < 2 {
            return None;
        }
        let surface_slots = input.take::<u32>()?;
        let direct = input.take::<Probe>()?;
        let lights = input.take::<u32>()?;
        let sky = input.take::<Packed>()?;
        let ground = input.take::<Packed>()?;
        let sun = input.take::<Packed>()?;
        let count = direct.len();
        let lattice = (grid.size.x as usize).checked_mul(grid.size.y as usize)?.checked_mul(grid.size.z as usize)?;
        if sky.iter().chain(&ground).chain(&sun).any(|value| !value.valid())
            || lattice > count
            || sky.len() != active
            || ground.len() != active
            || sun.len() != active.checked_mul(SUN_FIELDS)?
            || surface_slots.iter().any(|&v| v != u32::MAX && v as usize >= count)
            || lights.iter().any(|&v| v as usize >= surface_slots.len())
        {
            return None;
        }
        Some(Self {
            daylight: Some(Daylight { sky, ground, sun }),
            surface_slots,
            paths: Vec::new(),
            volume_weights: Vec::new(),
            path_offsets: Box::default(),
            path_lengths: Box::default(),
            sky: Box::default(),
            ground: Box::default(),
            sun: Box::default(),
            fixtures: Box::default(),
            raw: Box::default(),
            bounces: from_fn(|_| Box::default()),
            direct,
            active: Box::default(),
            grid,
            surface_neighbours: Box::default(),
            lights,
            responses: BTreeMap::new(),
            applied: Vec::new(),
            groups: Vec::new(),
            cursor: 0,
            remaining: 0,
        })
    }
}
