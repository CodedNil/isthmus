use naga::{
    back::wgsl::{WriterFlags, write_string},
    front::spv::{Options, parse_u8_slice},
    valid::{Capabilities, ValidationFlags, Validator},
};
use spirv_builder::{ModuleResult, SpirvBuilder, SpirvMetadata};
use std::{
    env, fs,
    io::Error,
    path::{Path, PathBuf},
};

const SHADER_TARGET: &str = "spirv-unknown-vulkan1.4";

/// Describes a Rust-GPU shader compiled as part of a host package's build script.
pub struct ShaderBuild {
    pub name: String,
    pub source: PathBuf,
    pub isthmus: PathBuf,
    pub workspace: PathBuf,
    pub output: PathBuf,
}

impl ShaderBuild {
    /// Builds one SPIR-V module in Cargo's output directory while retaining the nested cache.
    ///
    /// # Errors
    /// Returns filesystem and Rust-GPU build failures.
    pub fn build(self) -> Result<(), String> {
        let cache = self.workspace.join("target/isthmus").join(&self.name);
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
                "#![no_std]\n#![expect(unused_imports, reason = \"the generated shader crate compiles only the selected entry points\")]\n{}#[path = \"{}\"]\npub mod render;\n",
                if self.name == "cantus" {
                    "#![expect(dead_code, reason = \"host-only rendering helpers are unused in the generated shader crate\")]\n"
                } else {
                    ""
                },
                self.source.display(),
            ),
        )?;

        println!("cargo:rerun-if-changed={}", self.workspace.join("Cargo.lock").display());
        fs::copy(self.workspace.join("Cargo.lock"), source_crate.join("Cargo.lock"))
            .map_err(io_error("copy workspace Cargo.lock into shader workspace"))?;

        let build = SpirvBuilder::new(&source_crate, SHADER_TARGET)
            .deny_warnings(true)
            .shader_crate_default_features(false)
            .target_dir_path(target)
            .spirv_metadata(SpirvMetadata::None)
            .scalar_block_layout(true)
            .release(true)
            .build()
            .map_err(|error| format!("Rust-GPU shader build failed: {error}"))?;
        let module = match build.module {
            ModuleResult::SingleModule(module) => module,
            ModuleResult::MultiModule(_) => {
                return Err(String::from("Rust-GPU unexpectedly produced multiple shader modules"));
            }
        };
        if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
            let bytes = fs::read(module).map_err(io_error("read generated SPIR-V module"))?;
            let options =
                Options { adjust_coordinate_space: false, strict_capabilities: true, block_ctx_dump_prefix: None };
            let module = parse_u8_slice(&bytes, &options)
                .map_err(|error| format!("failed to parse generated SPIR-V: {error}"))?;
            let info = Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .map_err(|error| format!("failed to validate generated SPIR-V: {error}"))?;
            let wgsl = write_string(&module, &info, WriterFlags::empty())
                .map_err(|error| format!("failed to generate WGSL: {error}"))?;
            fs::write(self.output.with_extension("wgsl"), wgsl).map_err(io_error("write generated WGSL"))
        } else {
            fs::copy(module, self.output).map(|_| ()).map_err(io_error("copy generated SPIR-V module"))
        }
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
