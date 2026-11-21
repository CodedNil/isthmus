//! Dynamic-rendering pipelines with shared descriptor heap mappings.
use super::{error::RenderError, heap::Layout, images::TextureFormat, setup::Device};
use crate::{Blend, Program, program::ShaderEntry};
use ash::{
    util::read_spv,
    vk::{self, TaggedStructure as _},
};
use std::{ffi::CStr, io::Cursor, rc::Rc};

pub struct Pipelines {
    device: Rc<Device>,
    pub shaders: Vec<vk::Pipeline>,
}

impl Pipelines {
    pub fn new<P: Program>(device: &Rc<Device>, format: TextureFormat) -> Result<Self, RenderError> {
        let mut pipelines = Self { device: Rc::clone(device), shaders: Vec::new() };
        let mappings = Layout::new(&device.heap_properties).mappings();
        for entry in P::SHADERS {
            pipelines.shaders.push(compile(&device.raw, format, &mappings, entry)?);
        }
        Ok(pipelines)
    }
}

impl Drop for Pipelines {
    fn drop(&mut self) {
        for &shader in &self.shaders {
            // SAFETY: Gpu waits for pending commands before dropping its pipelines.
            unsafe {
                self.device.raw.destroy_pipeline(shader, None);
            }
        }
    }
}

fn compile(
    device: &ash::Device,
    surface_format: TextureFormat,
    mappings: &[vk::DescriptorSetAndBindingMappingEXT<'_>],
    entry: &ShaderEntry,
) -> Result<vk::Pipeline, RenderError> {
    let [vertex_name, fragment_name] = entry
        .entries
        .map(|name| CStr::from_bytes_with_nul(name.as_bytes()).map_err(|error| RenderError::Gpu(error.to_string())));
    let [vertex_code, fragment_code] =
        entry.code.map(|code| read_spv(&mut Cursor::new(code)).map_err(|error| RenderError::Gpu(error.to_string())));
    let (vertex_code, fragment_code) = (vertex_code?, fragment_code?);
    let mut vertex_module = vk::ShaderModuleCreateInfo::default().code(&vertex_code);
    let mut fragment_module = vk::ShaderModuleCreateInfo::default().code(&fragment_code);
    let mut vertex_mapping = vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings);
    let mut fragment_mapping = vertex_mapping;
    let vertex_stage =
        vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).name(vertex_name?);
    let fragment_stage =
        vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).name(fragment_name?);
    // SAFETY: pointers in each stage chain refer to structs that live through compilation.
    let vertex_stage = unsafe { vertex_stage.extend(&mut vertex_module) };
    // SAFETY: the mapping points into the live layout and starts an independent chain.
    let vertex_stage = unsafe { vertex_stage.extend(&mut vertex_mapping) };
    // SAFETY: the fragment module lives through compilation.
    let fragment_stage = unsafe { fragment_stage.extend(&mut fragment_module) };
    // SAFETY: the fragment stage has its own mapping referencing the live layout.
    let fragment_stage = unsafe { fragment_stage.extend(&mut fragment_mapping) };
    let stages = [vertex_stage, fragment_stage];
    let vertex = vk::PipelineVertexInputStateCreateInfo::default();
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
    let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
    let raster =
        vk::PipelineRasterizationStateCreateInfo::default().polygon_mode(vk::PolygonMode::FILL).line_width(1.0);
    let samples = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let blends = [blend(entry.blend)];
    let blending = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blends);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let formats = [entry.format.unwrap_or(surface_format).raw()];
    let mut rendering = vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&samples)
        .color_blend_state(&blending)
        .dynamic_state(&dynamic);
    let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
    // SAFETY: both extension structs live through compilation and start with empty chains.
    let info = unsafe { info.extend(&mut rendering) };
    // SAFETY: the flags extend this graphics pipeline and remain live through compilation.
    let info = unsafe { info.extend(&mut flags) };
    // SAFETY: all referenced state is live and the layout matches the shader ABI.
    match unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) } {
        Ok(pipelines) => Ok(pipelines[0]),
        Err((pipelines, error)) => {
            for pipeline in pipelines {
                // SAFETY: any returned pipelines are unused.
                unsafe {
                    device.destroy_pipeline(pipeline, None);
                }
            }
            Err(RenderError::Gpu(format!("pipeline {:?}: {error}", entry.name)))
        }
    }
}

fn blend(mode: Blend) -> vk::PipelineColorBlendAttachmentState {
    let state = vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);
    match mode {
        Blend::Replace | Blend::Raw => state,
        Blend::Add | Blend::Over => state
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(if mode == Blend::Add {
                vk::BlendFactor::ONE
            } else {
                vk::BlendFactor::ONE_MINUS_SRC_ALPHA
            })
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD),
    }
}
