use anyhow::{Context, Result, bail};
use cargo_metadata::{CargoOpt, DependencyKind, MetadataCommand};
use naga::{
    back::wgsl::{WriterFlags, write_string},
    front::{spv, wgsl},
    valid::{Capabilities, ValidationFlags, Validator},
};
use quote::{format_ident, quote};
use spirv_builder::{ModuleResult, SpirvBuilder, SpirvMetadata};
use std::{collections::BTreeMap, env, fs, path::Path};

mod source;
mod syntax;

/// Extracts shaders from a Rust source module.
pub fn build(source: &str) -> Result<()> {
    let manifest = env::var("CARGO_MANIFEST_DIR")?;
    let mut command = MetadataCommand::new();
    command.manifest_path(Path::new(&manifest).join("Cargo.toml"));
    let metadata = command.clone().no_deps().exec()?;
    let package = metadata
        .packages
        .iter()
        .find(|package| package.manifest_path.parent() == Some(manifest.as_str().into()))
        .context("shader consumer is absent from Cargo metadata")?;
    let dependency = package
        .dependencies
        .iter()
        .find(|dep| dep.name == "isthmus" && dep.kind == DependencyKind::Normal)
        .context("shader consumer must depend on isthmus")?;
    let isthmus = format_ident!("{}", dependency.rename.as_deref().unwrap_or("isthmus").replace('-', "_"));
    let source = Path::new(&manifest).join(source);
    let module = source.strip_prefix(Path::new(&manifest).join("src"))?.with_extension("");
    let mut modules: Vec<_> = module.iter().map(|part| part.to_string_lossy().into_owned()).collect();
    if modules.last().is_some_and(|name| matches!(name.as_str(), "mod" | "lib" | "main")) {
        modules.pop();
    }
    let generated = source::generate(&source, modules, &quote!(::#isthmus)).map_err(anyhow::Error::msg)?;
    let features: Vec<_> = package
        .features
        .keys()
        .filter(|feature| {
            env::var_os(format!("CARGO_FEATURE_{}", feature.as_str().replace('-', "_").to_uppercase())).is_some()
        })
        .map(ToString::to_string)
        .collect();
    let resolved =
        command.features(CargoOpt::NoDefaultFeatures).features(CargoOpt::SomeFeatures(features.clone())).exec()?;
    let nodes = &resolved.resolve.as_ref().context("Cargo did not resolve shader dependencies")?.nodes;
    let consumer = nodes.iter().find(|node| node.id == package.id).context("shader consumer was not resolved")?;
    let cache = metadata.target_directory.join("isthmus").join(package.name.as_str());
    let source_crate = cache.join("source");
    fs::create_dir_all(&source_crate)?;
    let mut shader_manifest: toml::Table = format!(
        "[package]\nname = {:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[workspace]\n",
        format!("{}-shader", package.name),
    )
    .parse()?;
    let mut dependencies = toml::Table::new();
    let mut targets = BTreeMap::<String, BTreeMap<&str, toml::Table>>::new();
    for dep in consumer.deps.iter().filter(|dep| {
        generated.identifiers.contains(&dep.name)
            && dep.dep_kinds.iter().any(|kind| kind.kind == DependencyKind::Normal)
    }) {
        let package = &resolved[&dep.pkg];
        let node = nodes.iter().find(|node| node.id == dep.pkg).expect("resolved dependency has a node");
        let mut value = toml::toml! {
            package = (package.name.as_str())
            default-features = false
            features = (node.features.iter().map(ToString::to_string).collect::<Vec<_>>())
        };
        match package.source.as_ref().map(|source| source.repr.as_str()) {
            None => {
                value.insert("path".into(), package.manifest_path.parent().unwrap().as_str().into());
            }
            Some(source) if source.starts_with("registry+") => {
                value.insert("registry-index".into(), source[9..].into());
                value.insert("version".into(), format!("={}", package.version).into());
            }
            Some(source) if source.starts_with("git+") => {
                let git = source[4..].split('#').next().unwrap();
                let (url, reference) = git.split_once('?').unwrap_or((git, ""));
                value.insert("git".into(), url.into());
                value.extend(
                    form_urlencoded::parse(reference.as_bytes())
                        .map(|(key, value)| (key.into_owned(), value.into_owned().into())),
                );
            }
            Some(source) => bail!("unsupported shader dependency source: {source}"),
        }
        for kind in dep.dep_kinds.iter().filter(|kind| kind.kind == DependencyKind::Normal) {
            let table = kind.target.as_ref().map_or(&mut dependencies, |target| {
                targets.entry(target.to_string()).or_default().entry("dependencies").or_default()
            });
            table.insert(dep.name.clone(), value.clone().into());
        }
    }
    shader_manifest.insert("dependencies".into(), dependencies.into());
    shader_manifest.insert("target".into(), toml::Value::try_from(targets)?);
    let mut shader_features =
        package.features.keys().map(|name| (name.clone(), toml::Value::Array(Vec::new()))).collect::<toml::Table>();
    shader_features
        .insert("default".into(), features.into_iter().filter(|name| name != "default").collect::<Vec<_>>().into());
    shader_manifest.insert("features".into(), shader_features.into());
    let workspace = metadata.workspace_root.join("Cargo.toml");
    let mut workspace_manifest: toml::Table = fs::read_to_string(&workspace)?.parse()?;
    if let Some(mut patch) = workspace_manifest.remove("patch") {
        for (_, deps) in patch.as_table_mut().context("patch must be a table")? {
            for (_, dep) in deps.as_table_mut().context("patch source must be a table")? {
                if let Some(path) = dep.get_mut("path") {
                    *path = metadata
                        .workspace_root
                        .join(path.as_str().context("patch path must be a string")?)
                        .as_str()
                        .into();
                }
            }
        }
        shader_manifest.insert("patch".into(), patch);
    }
    println!("cargo:rerun-if-changed={workspace}");
    write_if_changed(source_crate.join("Cargo.toml").as_std_path(), &toml::to_string(&shader_manifest)?)?;
    write_if_changed(source_crate.join("lib.rs").as_std_path(), &generated.source)?;
    let lock = metadata.workspace_root.join("Cargo.lock");
    println!("cargo:rerun-if-changed={lock}");
    println!("cargo:rerun-if-changed={}", package.manifest_path);
    write_if_changed(source_crate.join("Cargo.lock").as_std_path(), &fs::read_to_string(lock)?)?;
    let mut builder = SpirvBuilder::new(&source_crate, "spirv-unknown-vulkan1.4")
        .deny_warnings(true)
        .target_dir_path(cache.join("target"))
        .spirv_metadata(SpirvMetadata::None);
    builder.build_script.dependency_info = Some(true);
    let build = builder.build().context("Rust-GPU shader build failed")?;
    let ModuleResult::SingleModule(module) = build.module else {
        bail!("Rust-GPU unexpectedly produced multiple shader modules");
    };
    let options =
        spv::Options { adjust_coordinate_space: false, strict_capabilities: true, block_ctx_dump_prefix: None };
    let reflected = spv::parse_u8_slice(&fs::read(module)?, &options)?;
    // Packed half floats use core WGSL operations without requiring the f16 extension.
    let mut validator =
        Validator::new(ValidationFlags::all(), Capabilities::default() | Capabilities::SHADER_FLOAT16_IN_FLOAT32);
    let info = validator.validate(&reflected)?;
    // Tint rejects Naga's rounded decimal spelling of the largest finite float.
    let wgsl =
        write_string(&reflected, &info, WriterFlags::empty())?.replace(&format!("{}f", f32::MAX), "0x1.fffffep+127f");
    // Preserve SPIR-V derivatives when WGSL cannot prove control-flow uniformity.
    let wgsl = format!("diagnostic(warning, derivative_uniformity);\n{wgsl}");
    let translated = wgsl::parse_str(&wgsl)?;
    validator.validate(&translated)?;
    for entry in generated.shaders.iter().flat_map(|shader| [shader.entry.value(), shader.vertex_entry()]) {
        if !translated.entry_points.iter().any(|other| other.name == entry) {
            bail!("shader entry {entry} was not exported");
        }
    }
    let output = env::var("OUT_DIR")?;
    let output = Path::new(&output);
    let entries = generated.shaders.iter().map(syntax::shader::Shader::metadata);
    write_if_changed(
        &output.join("isthmus.manifest.rs"),
        &quote!({
            use ::#isthmus::{Blend, __private::ShaderEntry};
            &[#(#entries),*]
        })
        .to_string(),
    )?;
    write_if_changed(&output.join("isthmus.wgsl"), &wgsl)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    if !fs::read_to_string(path).is_ok_and(|current| current == contents) {
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}
