//! Build-script support: extracts `shader!` declarations from a crate's source, compiles them
//! with Rust-GPU and emits WGSL plus the manifest the `program!` macro includes.
#![expect(clippy::wildcard_enum_match_arm, reason = "syn's syntax enums are non-exhaustive")]
use anyhow::{Context as _, Result, bail};
use cargo::{Consumer, directive, write_if_changed};
use quote::{format_ident, quote};
use spirv_builder::{ModuleResult, SpirvBuilder, SpirvMetadata};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

mod cargo;
mod generate;
mod manifest;
mod wgsl;

/// Extracts shaders from a Rust source module.
pub fn build(source: &str) -> Result<()> {
    let consumer = Consumer::discover()?;
    let name = format_ident!("{}", consumer.isthmus_name()?);
    let isthmus = quote!(::#name);
    let source = consumer.directory.join(source);
    let generated =
        generate::generate(&source, consumer.module_path(&source)?, &isthmus).map_err(anyhow::Error::msg)?;

    let cache = consumer.cache();
    let shader_crate = cache.join("source");
    fs::create_dir_all(&shader_crate)?;
    write_if_changed(&shader_crate.join("Cargo.toml"), &manifest::render(&consumer, &generated.identifiers)?)?;
    generate::split::write(&shader_crate.join("lib.rs"), &generated.source)?;
    let workspace = &consumer.metadata.workspace_root;
    let lock = workspace.join("Cargo.lock");
    write_if_changed(&shader_crate.join("Cargo.lock"), &fs::read_to_string(&lock)?)?;
    for watched in [workspace.join("Cargo.toml"), lock, consumer.package.manifest_path.clone()] {
        directive(format_args!("rerun-if-changed={watched}"));
    }

    let spirv = compile(&shader_crate, &cache)?;
    let output = env::var("OUT_DIR")?;
    let output = Path::new(&output);
    let entries: Vec<_> =
        generated.shaders.iter().flat_map(|shader| [shader.vertex_entry(), shader.entry.value()]).collect();
    wgsl::write_stages(&spirv, &entries, output)?;
    let metadata = generated.shaders.iter().map(|shader| shader.metadata(&isthmus));
    write_if_changed(
        &output.join("isthmus.manifest.rs"),
        &quote!({
            use #isthmus::{Blend, __private::ShaderEntry};
            &[#(#metadata),*]
        })
        .to_string(),
    )
}

/// Builds the shader crate with Rust-GPU into a single SPIR-V module.
fn compile(shader_crate: &Path, cache: &Path) -> Result<PathBuf> {
    let mut builder = SpirvBuilder::new(shader_crate, "spirv-unknown-vulkan1.4")
        .deny_warnings(true)
        .target_dir_path(cache.join("target"))
        .spirv_metadata(SpirvMetadata::None);
    builder.build_script.dependency_info = Some(true);
    let build = builder.build().context("Rust-GPU shader build failed")?;
    let ModuleResult::SingleModule(module) = build.module else {
        bail!("Rust-GPU unexpectedly produced multiple shader modules");
    };
    Ok(module)
}
