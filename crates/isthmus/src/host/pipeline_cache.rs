//! Compiled pipelines kept on disk between runs, where the driver supports it (Vulkan), so a launch after the first
//! skips most of the driver's shader compilation.
use std::{env, fs, path::PathBuf};
use wgpu::util::pipeline_cache_key;

/// A driver pipeline cache, loaded from and saved back to a file named for the driver and the program.
pub struct DiskCache {
    pub cache: wgpu::PipelineCache,
    path: PathBuf,
    loaded: Vec<u8>,
}

impl DiskCache {
    /// Opens the cache for `program` on this device, or `None` where the device can't keep one.
    pub fn open(device: &wgpu::Device, adapter: &wgpu::Adapter, program: &str) -> Option<Self> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let key = pipeline_cache_key(&adapter.get_info())?;
        let program: String = program.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let path = directory()?.join(format!("{key}_{program}.bin"));
        let loaded = fs::read(&path).unwrap_or_default();
        // SAFETY: the data is empty or was written by `save` from `PipelineCache::get_data`, and `fallback` starts
        // an empty cache if the driver rejects it.
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("isthmus"),
                data: (!loaded.is_empty()).then_some(loaded.as_slice()),
                fallback: true,
            })
        };
        Some(Self { cache, path, loaded })
    }

    /// Writes the cache back if compiling added to it. A failure only costs the next launch its speedup.
    pub fn save(&self) {
        let Some(data) = self.cache.get_data().filter(|data| *data != self.loaded) else { return };
        let temporary = self.path.with_extension("tmp");
        let written = self.path.parent().is_none_or(|parent| fs::create_dir_all(parent).is_ok())
            && fs::write(&temporary, &data).is_ok()
            && fs::rename(&temporary, &self.path).is_ok();
        if !written {
            tracing::debug!(path = %self.path.display(), "Could not save the pipeline cache");
        }
    }
}

/// The user's cache directory for isthmus programs.
fn directory() -> Option<PathBuf> {
    let base = env::var_os("XDG_CACHE_HOME")
        .or_else(|| env::var_os("LOCALAPPDATA"))
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("isthmus"))
}
