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
use sha2::{Digest, Sha256};
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
    println!("cargo:rerun-if-changed={}", workspace.join("crates/isthmus_sdf/src").display());
    let variable = format!("{}_SHADER_SPV", name.replace('-', "_").to_uppercase());
    println!("cargo:rerun-if-env-changed={variable}");
    if let Some(shader) = env::var_os(variable)
        && !web
    {
        let shader = PathBuf::from(shader);
        let expected =
            fs::read_to_string(shader.with_extension("fingerprint")).map_err(io_error("read prebuilt fingerprint"))?;
        if fingerprint(&workspace, &source, &shader)? != expected {
            return Err("prebuilt shader is stale or damaged; rebuild it from the current sources".into());
        }
        for extension in ["spv", "manifest.rs", "fingerprint"] {
            let from = shader.with_extension(extension);
            println!("cargo:rerun-if-changed={}", from.display());
            fs::copy(from, output.with_extension(extension)).map_err(io_error("copy prebuilt shader"))?;
        }
        return Ok(());
    }
    compile(&name, &source, &workspace, &output)
}

/// Builds one SPIR-V module in Cargo's output directory while retaining the nested cache.
///
/// # Errors
/// Returns filesystem and Rust-GPU build failures.
pub fn compile(name: &str, source: &Path, workspace: &Path, output: &Path) -> Result<(), String> {
    let cache = workspace.join("target/isthmus").join(name);
    let source_crate = cache.join("source");
    let target = cache.join("target");
    fs::create_dir_all(&source_crate).map_err(io_error("create shader source directory"))?;
    fs::create_dir_all(&target).map_err(io_error("create shader target directory"))?;

    write_if_changed(
        &source_crate.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"{}-shader\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"lib.rs\"\n\n[workspace]\n\n[dependencies]\nisthmus = {{ path = \"{}\" }}\nisthmus_sdf = {{ path = \"{}\" }}\n",
            name,
            workspace.join("crates/isthmus").display(),
            workspace.join("crates/isthmus_sdf").display(),
        ),
    )?;
    let generated = source::generate(source)?;
    write_if_changed(&source_crate.join("lib.rs"), &generated.source)?;

    println!("cargo:rerun-if-changed={}", workspace.join("Cargo.lock").display());
    fs::copy(workspace.join("Cargo.lock"), source_crate.join("Cargo.lock"))
        .map_err(io_error("copy workspace Cargo.lock into shader workspace"))?;

    let build = SpirvBuilder::new(&source_crate, SHADER_TARGET)
        .deny_warnings(true)
        .target_dir_path(target)
        .spirv_metadata(SpirvMetadata::None)
        .build()
        .map_err(|error| format!("Rust-GPU shader build failed: {error}"))?;
    let web = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    for entry in generated.shaders.iter().flat_map(|shader| [shader.entry.value(), shader.vertex_entry()]) {
        if !build.entry_points.contains(&entry) {
            return Err(format!("shader entry {entry} was not exported"));
        }
    }
    let metadata = generated.shaders.iter().map(syntax::shader::Shader::metadata);
    let manifest = if generated.shaders.is_empty() {
        quote::quote!(&[])
    } else {
        quote::quote!({
            use ::isthmus::{Blend, __private::ShaderEntry};
            &[#(#metadata),*]
        })
    }
    .to_string();
    fs::write(output.with_extension("manifest.rs"), manifest).map_err(io_error("write shader manifest"))?;
    let module = match build.module {
        ModuleResult::SingleModule(module) => module,
        ModuleResult::MultiModule(_) => {
            return Err(String::from("Rust-GPU unexpectedly produced multiple shader modules"));
        }
    };
    let bytes = fs::read(&module).map_err(io_error("read generated SPIR-V module"))?;
    let options = Options { adjust_coordinate_space: false, strict_capabilities: true, block_ctx_dump_prefix: None };
    let reflected =
        parse_u8_slice(&bytes, &options).map_err(|error| format!("failed to parse generated SPIR-V: {error}"))?;
    // Packed half floats use core WGSL operations without requiring the f16 extension.
    let capabilities = Capabilities::default() | Capabilities::SHADER_FLOAT16_IN_FLOAT32;
    let mut validator = Validator::new(ValidationFlags::all(), capabilities);
    let info =
        validator.validate(&reflected).map_err(|error| format!("failed to validate generated SPIR-V: {error:?}"))?;
    if web {
        let wgsl = write_string(&reflected, &info, WriterFlags::empty())
            .map_err(|error| format!("failed to generate WGSL: {error}"))?;
        // Preserve SPIR-V derivative behavior when WGSL cannot prove control-flow uniformity.
        let wgsl = format!("diagnostic(warning, derivative_uniformity);\n{wgsl}");
        let translated = parse_str(&wgsl).map_err(|error| format!("failed to parse generated WGSL: {error}"))?;
        validator.validate(&translated).map_err(|error| format!("failed to validate generated WGSL: {error:?}"))?;
        for entry in &reflected.entry_points {
            if !translated.entry_points.iter().any(|other| other.name == entry.name && other.stage == entry.stage) {
                return Err(format!("WGSL translation changed shader entry {}", entry.name));
            }
        }
        fs::write(output.with_extension("wgsl"), wgsl).map_err(io_error("write generated WGSL"))
    } else {
        fs::copy(module, output).map_err(io_error("copy generated SPIR-V module"))?;
        let fingerprint = fingerprint(workspace, source, output)?;
        fs::write(output.with_extension("fingerprint"), fingerprint).map_err(io_error("write shader fingerprint"))
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

fn fingerprint(workspace: &Path, source: &Path, shader: &Path) -> Result<String, String> {
    fn collect(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        if path.is_dir() {
            for entry in fs::read_dir(path).map_err(io_error("list shader inputs"))? {
                collect(&entry.map_err(io_error("read shader input entry"))?.path(), files)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "rs" || extension == "toml") {
            files.push(path.to_owned());
        }
        Ok(())
    }
    let app = source
        .ancestors()
        .find(|path| path.ends_with("src"))
        .and_then(Path::parent)
        .ok_or("shader source must be inside src")?;
    let mut files = vec![workspace.join("Cargo.lock"), workspace.join("Cargo.toml"), workspace.join("rustfmt.toml")];
    for directory in [
        app.to_owned(),
        workspace.join("crates/isthmus"),
        workspace.join("crates/isthmus_sdf"),
        workspace.join("crates/isthmus_build"),
        workspace.join("crates/isthmus_macros"),
    ] {
        files.push(directory.join("Cargo.toml"));
        collect(&directory.join("src"), &mut files)?;
    }
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let name = file.strip_prefix(workspace).map_err(|error| error.to_string())?.to_string_lossy();
        hash.update(name.as_bytes());
        hash.update([0]);
        let bytes = fs::read(file).map_err(io_error("read shader input"))?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    for extension in ["spv", "manifest.rs"] {
        let bytes = fs::read(shader.with_extension(extension)).map_err(io_error("read shader artifact"))?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(hex::encode(hash.finalize()))
}
