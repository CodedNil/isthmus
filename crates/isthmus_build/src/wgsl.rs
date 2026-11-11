//! Translates the compiled SPIR-V module into one WGSL file per shader stage.
use crate::cargo::write_if_changed;
use anyhow::{Context as _, Result};
use naga::{
    Module,
    back::wgsl::{WriterFlags, write_string},
    compact::{KeepUnused, compact},
    front::{spv, wgsl},
    valid::{Capabilities, ValidationFlags, Validator},
};
use std::{fs, path::Path};

/// Writes `<entry>.wgsl` into `output` for every named entry point of the SPIR-V module at `spirv`.
pub fn write_stages(spirv: &Path, entries: &[String], output: &Path) -> Result<()> {
    let options =
        spv::Options { adjust_coordinate_space: false, strict_capabilities: true, block_ctx_dump_prefix: None };
    let reflected = spv::parse_u8_slice(&fs::read(spirv)?, &options)?;
    // Packed half floats use core WGSL operations without requiring the f16 extension.
    let mut validator =
        Validator::new(ValidationFlags::all(), Capabilities::default() | Capabilities::SHADER_FLOAT16_IN_FLOAT32);
    validator.validate(&reflected)?;
    for name in entries {
        write_stage(&reflected, &mut validator, name, output)?;
    }
    Ok(())
}

/// Writes one stage as WGSL containing only that stage's entry point.
fn write_stage(reflected: &Module, validator: &mut Validator, name: &str, output: &Path) -> Result<()> {
    // A single entry point lets wgpu skip runtime pruning and revalidation for each stage.
    let mut module = reflected.clone();
    module.entry_points.retain(|entry| entry.name == name);
    let entry = module.entry_points.first_mut().with_context(|| format!("shader entry {name} was not exported"))?;
    // Each stage owns a separate module. Keep its GPU entry name stable
    // when unrelated Rust edits move shader! declarations to new lines.
    entry.name = "main".into();
    compact(&mut module, KeepUnused::No);
    let info = validator.validate(&module)?;
    // Tint rejects Naga's rounded decimal spelling of the largest finite float.
    let code =
        write_string(&module, &info, WriterFlags::empty())?.replace(&format!("{}f", f32::MAX), "0x1.fffffep+127f");
    // Preserve SPIR-V derivatives when WGSL cannot prove control-flow uniformity.
    let code = code.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
    let code = format!("diagnostic(warning, derivative_uniformity);\n{code}");
    validator.validate(&wgsl::parse_str(&code)?)?;
    write_if_changed(&output.join(format!("{name}.wgsl")), &code)
}
