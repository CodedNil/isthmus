use spirv_builder::{Capability, ModuleResult, SpirvBuilder, SpirvMetadata};
use std::{
    fs,
    io::Error,
    path::{Path, PathBuf},
};

const SHADER_TARGET: &str = "spirv-unknown-vulkan1.4";

/// Describes a Rust-GPU shader compiled as part of a host package's build script.
pub struct ShaderBuild {
    pub name: &'static str,
    pub source: PathBuf,
    pub isthmus: PathBuf,
    pub workspace: PathBuf,
    pub output: PathBuf,
}

impl ShaderBuild {
    /// Builds the shader into Cargo's output directory while retaining the nested Cargo cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the generated shader workspace cannot be written or Rust-GPU fails
    /// to produce exactly one SPIR-V module.
    pub fn build(self) -> Result<(), String> {
        let cache = self.workspace.join("target/isthmus").join(self.name);
        let source_crate = cache.join("source");
        let target = cache.join("target");
        fs::create_dir_all(&source_crate).map_err(io_error("create shader source directory"))?;
        fs::create_dir_all(&target).map_err(io_error("create shader target directory"))?;

        write_if_changed(
            &source_crate.join("Cargo.toml"),
            &format!(
                "[package]\nname = \"{}-shader\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"lib.rs\"\n\n[workspace]\n\n[dependencies]\nisthmus = {{ path = \"{}\" }}\n",
                self.name,
                self.isthmus.display(),
            ),
        )?;
        write_if_changed(
            &source_crate.join("lib.rs"),
            &format!(
                "#![no_std]\n#![allow(dead_code, unused_imports)]\n#[path = \"{}\"]\npub mod render;\n",
                self.source.display(),
            ),
        )?;

        let build = SpirvBuilder::new(&source_crate, SHADER_TARGET)
            .deny_warnings(true)
            .shader_crate_default_features(false)
            .target_dir_path(target)
            .spirv_metadata(SpirvMetadata::None)
            .scalar_block_layout(true)
            .capability(Capability::RuntimeDescriptorArray)
            .capability(Capability::SampledImageArrayNonUniformIndexing)
            .capability(Capability::ShaderNonUniform)
            .extension("SPV_EXT_descriptor_indexing")
            .release(true)
            .build()
            .map_err(|error| format!("Rust-GPU shader build failed: {error}"))?;
        let module = match build.module {
            ModuleResult::SingleModule(module) => module,
            ModuleResult::MultiModule(_) => {
                return Err(String::from("Rust-GPU unexpectedly produced multiple shader modules"));
            }
        };
        fs::copy(module, self.output)
            .map(|_| ())
            .map_err(io_error("copy generated SPIR-V module"))
    }
}

fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    fs::write(path, contents).map_err(io_error("write generated shader source"))
}

fn io_error(operation: &'static str) -> impl FnOnce(Error) -> String {
    move |error| format!("failed to {operation}: {error}")
}
