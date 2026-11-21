//! Optional GPU timestamps, collected after the renderer's normal frame fences.
use super::setup::Device;
use ash::vk;
use smallvec::SmallVec;
use std::{collections::BTreeMap, env, rc::Rc};

const REPORT_FRAMES: usize = 120;

type Region = (&'static str, Option<&'static str>);

pub struct Timings {
    mask: u64,
    period_ns: f64,
    frames: usize,
    samples: BTreeMap<Region, Vec<u64>>,
    pub shaders: bool,
}

impl Timings {
    pub fn new(device: &Device) -> Option<Self> {
        let shaders = match env::var("ISTHMUS_GPU_TIMINGS").as_deref() {
            Ok("1") => false,
            Ok("shaders") => true,
            _ => return None,
        };
        // SAFETY: the selected queue family belongs to this live physical device.
        let families = unsafe { device.vulkan.instance.get_physical_device_queue_family_properties(device.physical) };
        let bits = families[device.family as usize].timestamp_valid_bits;
        if bits == 0 {
            tracing::warn!(target: "isthmus::gpu_timings", "The graphics queue does not support GPU timestamps");
            return None;
        }
        Some(Self {
            mask: u64::MAX >> (64 - bits),
            period_ns: f64::from(device.properties.limits.timestamp_period),
            frames: 0,
            samples: BTreeMap::new(),
            shaders,
        })
    }

    pub fn collect(&mut self, queries: &mut Queries) {
        if !queries.submitted {
            return;
        }
        queries.submitted = false;
        let mut values = SmallVec::<[u64; 16]>::from_elem(0, 2 * queries.names.len());
        // SAFETY: the frame fence has signalled, and each recorded pass wrote two queries.
        let result = unsafe {
            queries.device.raw.get_query_pool_results(queries.pool, 0, &mut values, vk::QueryResultFlags::TYPE_64)
        };
        if let Err(error) = result {
            tracing::warn!(target: "isthmus::gpu_timings", %error, "Could not read GPU timestamps");
            return;
        }
        let mut totals = BTreeMap::<Region, u64>::new();
        for (&region, [start, end]) in queries.names.iter().zip(values.as_chunks::<2>().0) {
            let elapsed = end.wrapping_sub(*start) & self.mask;
            let total = totals.entry(region).or_default();
            *total = total.saturating_add(elapsed);
        }
        for (region, total) in totals {
            self.samples.entry(region).or_default().push(total);
        }
        self.frames += 1;
        if self.frames == REPORT_FRAMES {
            self.report();
        }
    }

    pub fn report(&mut self) {
        if self.frames == 0 {
            return;
        }
        for (&(pass, shader), samples) in &mut self.samples {
            if samples.is_empty() {
                continue;
            }
            samples.sort_unstable();
            let count = samples.len();
            let ms = |ticks| ticks as f64 * self.period_ns / 1e6;
            let (lower, upper) = (samples[(count - 1) / 2], samples[count / 2]);
            let median_ms = f64::midpoint(ms(lower), ms(upper));
            let p95_ms = ms(samples[(count * 95).div_ceil(100) - 1]);
            tracing::info!(
                target: "isthmus::gpu_timings",
                pass, shader, frames = self.frames, samples = count,
                median_ms = format_args!("{median_ms:.3}"), p95_ms = format_args!("{p95_ms:.3}"),
                "GPU timing"
            );
            samples.clear();
        }
        self.frames = 0;
    }
}

pub struct Queries {
    device: Rc<Device>,
    pool: vk::QueryPool,
    capacity: u32,
    names: SmallVec<[Region; 8]>,
    submitted: bool,
}

impl Queries {
    fn new(device: &Rc<Device>, capacity: u32) -> Result<Self, vk::Result> {
        // SAFETY: capacity is positive and this device's graphics queue supports timestamps.
        let pool = unsafe {
            device.raw.create_query_pool(
                &vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count(capacity),
                None,
            )
        }?;
        Ok(Self { device: Rc::clone(device), pool, capacity, names: SmallVec::new(), submitted: false })
    }

    pub fn prepare(slot: &mut Option<Self>, device: &Rc<Device>, cmd: vk::CommandBuffer, count: usize) -> bool {
        let Ok(count) = u32::try_from(count.saturating_mul(2)) else { return false };
        if count == 0 {
            return false;
        }
        if slot.as_ref().is_none_or(|queries| queries.capacity < count) {
            match Self::new(device, count) {
                Ok(queries) => *slot = Some(queries),
                Err(error) => {
                    tracing::warn!(target: "isthmus::gpu_timings", %error, "Could not allocate GPU timestamp queries");
                    return false;
                }
            }
        }
        let queries = slot.as_mut().unwrap();
        queries.names.clear();
        // SAFETY: the previous frame fence has signalled and the command buffer is recording.
        unsafe {
            device.raw.cmd_reset_query_pool(cmd, queries.pool, 0, count);
        }
        true
    }

    pub fn start(&mut self, cmd: vk::CommandBuffer, region: Region) -> u32 {
        let query = self.names.len() as u32 * 2;
        self.names.push(region);
        self.write(cmd, query);
        query
    }

    pub fn end(&self, cmd: vk::CommandBuffer, start: u32) {
        self.write(cmd, start + 1);
    }

    fn write(&self, cmd: vk::CommandBuffer, query: u32) {
        // SAFETY: prepare reserved and reset two queries for each region.
        unsafe {
            self.device.raw.cmd_write_timestamp2(cmd, vk::PipelineStageFlags2::ALL_COMMANDS, self.pool, query);
        }
    }

    pub fn finish(&mut self) {
        self.submitted = !self.names.is_empty();
    }
}

impl Drop for Queries {
    fn drop(&mut self) {
        // SAFETY: the owning frame has completed, or the renderer is tearing down its device.
        unsafe {
            self.device.raw.destroy_query_pool(self.pool, None);
        }
    }
}
