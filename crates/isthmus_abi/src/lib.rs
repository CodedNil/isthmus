#![no_std]
//! The binding layout shared by the host runtime and the generated shader entry points.
//!
//! Both sides must agree on these numbers, but only one of them may depend on the other's
//! crate graph (shaders compile for SPIR-V, code generation needs `syn`), so they live here.

/// Descriptor set holding the frame and resource storage buffers.
pub const FRAME_SET: u32 = 0;
/// Binding of the per-frame draw records.
pub const FRAME_BINDING: u32 = 0;
/// Binding of host resources addressed by captured handles.
pub const RESOURCES_BINDING: u32 = 1;
/// Number of storage buffers in [`FRAME_SET`].
pub const BUFFER_COUNT: usize = 2;

/// Descriptor set holding the sampler and captured images.
pub const IMAGE_SET: u32 = 1;
/// Binding of the shared sampler in [`IMAGE_SET`].
pub const SAMPLER_BINDING: u32 = 0;
/// Binding of the first captured image in [`IMAGE_SET`]; later images follow in capture order.
pub const FIRST_IMAGE_BINDING: u32 = 1;
