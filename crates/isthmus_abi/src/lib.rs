#![no_std]
//! The binding layout shared by the host runtime and the generated shader entry points.
//!
//! Both sides must agree on these numbers, but only one of them may depend on the other's
//! crate graph (shaders compile for SPIR-V, code generation needs `syn`), so they live here.

/// Descriptor set holding the frame and shared storage buffers.
pub const FRAME_SET: u32 = 0;
/// Binding of the per-frame draw records.
pub const FRAME_BINDING: u32 = 0;
/// Binding of the words that draws share, which change far less often than the draw records.
pub const SHARED_BINDING: u32 = 1;
/// Number of storage buffers in [`FRAME_SET`].
pub const BUFFER_COUNT: usize = 2;

/// Descriptor set holding the shared sampler and bindless image table.
pub const IMAGE_SET: u32 = 1;
/// Binding of the shared sampler in [`IMAGE_SET`].
pub const SAMPLER_BINDING: u32 = 0;
/// Binding of the runtime texture array in [`IMAGE_SET`].
pub const IMAGE_BINDING: u32 = 1;
