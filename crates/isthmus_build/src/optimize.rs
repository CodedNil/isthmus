//! Simplify control flow introduced when reconstructing structured shaders.
use anyhow::{Context, Result};
use naga::{Module, back::spv, valid::ModuleInfo};
use spirv::{FunctionControl, LoopControl, Op};
use spirv_tools::opt::{self, Optimizer};
use std::collections::HashMap;

pub(super) fn shader(module: &Module, info: &ModuleInfo) -> Result<Module> {
    let options = spv::Options { flags: spv::WriterFlags::empty(), force_loop_bounding: false, ..Default::default() };
    let mut words = spv::write_vec(module, info, &options, None).context("shader optimization input")?;
    // Preserve bounded loops and shared helpers while folding the entry wrapper
    // and its private temporaries. Exhaustive unrolling bloats SDF march loops.
    let mut calls = HashMap::<u32, usize>::new();
    let mut cursor = 5;
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        if words[cursor] & 65535 == Op::FunctionCall as u32 {
            *calls.entry(words[cursor + 3]).or_default() += 1;
        }
        cursor += count;
    }
    cursor = 5;
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        let opcode = words[cursor] & 65535;
        if opcode == Op::LoopMerge as u32 {
            words[cursor + 3] = (words[cursor + 3] & !LoopControl::UNROLL.bits()) | LoopControl::DONT_UNROLL.bits();
        }
        if opcode == Op::Function as u32 && calls.get(&words[cursor + 2]).copied().unwrap_or(0) > 1 {
            words[cursor + 3] = FunctionControl::DONT_INLINE.bits();
        }
        cursor += count;
    }
    let mut optimizer = opt::create(None);
    optimizer.register_performance_passes();
    let binary = optimizer.optimize(&words, &mut |_message| {}, None).context("shader control-flow optimization")?;
    let options = naga::front::spv::Options {
        adjust_coordinate_space: false,
        strict_capabilities: true,
        block_ctx_dump_prefix: None,
    };
    naga::front::spv::Frontend::new(binary.as_words().iter().copied(), &options)
        .parse()
        .context("optimized shader import")
}
