//! Extracts Rust shader code and compiles validated SPIR-V and WGSL modules.
#![warn(missing_docs)]

use naga::{
    back::wgsl::{WriterFlags, write_string},
    front::{
        spv::{Options, parse_u8_slice},
        wgsl::parse_str,
    },
    valid::{Capabilities, ValidationFlags, Validator},
};
use spirv_builder::{ModuleResult, SpirvBuilder, SpirvMetadata};
use std::{
    env, fs,
    io::Error,
    path::{Path, PathBuf},
};

const SHADER_TARGET: &str = "spirv-unknown-vulkan1.4";

mod source;
mod syntax;

/// Builds the render module or installs its prebuilt shader and generated manifest.
///
/// # Errors
/// Returns environment, filesystem and shader compiler failures.
pub fn build(source: &str) -> Result<(), String> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is missing")?);
    let name = env::var("CARGO_PKG_NAME").map_err(|error| error.to_string())?;
    let workspace = manifest
        .ancestors()
        .find(|path| path.join("crates/isthmus").is_dir())
        .ok_or("Isthmus workspace root was not found")?
        .to_path_buf();
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?).join("isthmus.spv");
    let web = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    println!(
        "cargo:rustc-env=ISTHMUS_SHADER_PATH={}",
        output.with_extension(if web { "wgsl" } else { "spv" }).display()
    );
    let source = manifest.join(source);
    println!("cargo:rerun-if-changed={}", manifest.join("src").display());
    println!("cargo:rerun-if-changed={}", manifest.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", workspace.join("crates/isthmus/src").display());
    let variable = format!("{}_SHADER_SPV", name.replace('-', "_").to_uppercase());
    println!("cargo:rerun-if-env-changed={variable}");
    if let Some(shader) = env::var_os(variable)
        && !web
    {
        let shader = PathBuf::from(shader);
        for extension in ["spv", "manifest.rs"] {
            let from = shader.with_extension(extension);
            println!("cargo:rerun-if-changed={}", from.display());
            fs::copy(from, output.with_extension(extension)).map_err(io_error("copy prebuilt shader"))?;
        }
        return Ok(());
    }
    ShaderBuild { name, source, isthmus: workspace.join("crates/isthmus"), workspace, output }.build()
}

/// Describes a Rust-GPU shader compiled as part of a host package's build script.
pub struct ShaderBuild {
    /// Unique shader package name used for its build cache.
    pub name: String,
    /// Root Rust module containing the program and shader declarations.
    pub source: PathBuf,
    /// Path to the Isthmus runtime crate.
    pub isthmus: PathBuf,
    /// Workspace root containing the shared shader build cache.
    pub workspace: PathBuf,
    /// Output SPIR-V path, with WGSL and manifest files written alongside it.
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
        let generated = source::generate(&self.source)?;
        for (path, source) in &generated.files {
            let path = source_crate.join(path);
            let parent = path.parent().ok_or("generated source has no parent directory")?;
            fs::create_dir_all(parent).map_err(io_error("create generated module directory"))?;
            let alternative = parent.with_extension("rs");
            if path.ends_with("mod.rs") && alternative.is_file() {
                fs::remove_file(alternative).map_err(io_error("remove superseded generated module"))?;
            }
            write_if_changed(&path, source)?;
        }

        println!("cargo:rerun-if-changed={}", self.workspace.join("Cargo.lock").display());
        fs::copy(self.workspace.join("Cargo.lock"), source_crate.join("Cargo.lock"))
            .map_err(io_error("copy workspace Cargo.lock into shader workspace"))?;

        let build = SpirvBuilder::new(&source_crate, SHADER_TARGET)
            .deny_warnings(true)
            .target_dir_path(target)
            .spirv_metadata(SpirvMetadata::None)
            .build()
            .map_err(|error| format!("Rust-GPU shader build failed: {error}"))?;
        let web = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
        for shader in &generated.shaders {
            if !build.entry_points.contains(&shader.entry()) {
                return Err(format!("shader entry {} was not exported", shader.entry()));
            }
        }
        let metadata = generated.shaders.iter().map(syntax::Shader::metadata);
        let manifest = if generated.shaders.is_empty() {
            quote::quote!(&[])
        } else {
            quote::quote!({
                use ::isthmus::{Blend, __private::ShaderEntry};
                &[#(#metadata),*]
            })
        }
        .to_string();
        fs::write(self.output.with_extension("manifest.rs"), manifest).map_err(io_error("write shader manifest"))?;
        let module = match build.module {
            ModuleResult::SingleModule(module) => module,
            ModuleResult::MultiModule(_) => {
                return Err(String::from("Rust-GPU unexpectedly produced multiple shader modules"));
            }
        };
        let bytes = fs::read(&module).map_err(io_error("read generated SPIR-V module"))?;
        let options =
            Options { adjust_coordinate_space: false, strict_capabilities: true, block_ctx_dump_prefix: None };
        let reflected =
            parse_u8_slice(&bytes, &options).map_err(|error| format!("failed to parse generated SPIR-V: {error}"))?;
        // Packed half floats use core WGSL operations without requiring the f16 extension.
        let capabilities = Capabilities::default() | Capabilities::SHADER_FLOAT16_IN_FLOAT32;
        let mut validator = Validator::new(ValidationFlags::all(), capabilities);
        let info = validator
            .validate(&reflected)
            .map_err(|error| format!("failed to validate generated SPIR-V: {error:?}"))?;
        if web {
            let wgsl = write_string(&reflected, &info, WriterFlags::empty())
                .map_err(|error| format!("failed to generate WGSL: {error}"))?;
            let translated = parse_str(&wgsl).map_err(|error| format!("failed to parse generated WGSL: {error}"))?;
            validator.validate(&translated).map_err(|error| format!("failed to validate generated WGSL: {error:?}"))?;
            for entry in &reflected.entry_points {
                if !translated.entry_points.iter().any(|other| other.name == entry.name && other.stage == entry.stage) {
                    return Err(format!("WGSL translation changed shader entry {}", entry.name));
                }
            }
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
