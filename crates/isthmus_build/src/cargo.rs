//! What Cargo tells the build script about the crate whose shaders are being extracted.
use anyhow::{Context as _, Result};
use cargo_metadata::{CargoOpt, DependencyKind, Metadata, MetadataCommand, Package, camino::Utf8Path};
use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
};

/// Passes a directive to Cargo, which reads them from the build script's stdout.
#[expect(clippy::print_stdout, reason = "Cargo build-script directives are printed to stdout")]
pub fn directive(directive: fmt::Arguments<'_>) {
    println!("cargo:{directive}");
}

/// Writes `contents` unless the file already holds them, so unchanged output does not trigger rebuilds.
pub fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    if !fs::read_to_string(path).is_ok_and(|current| current == contents) {
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

/// The crate being built, with its resolved dependencies.
pub struct Consumer {
    pub directory: PathBuf,
    pub metadata: Metadata,
    pub package: Package,
    command: MetadataCommand,
}

impl Consumer {
    /// Finds the crate Cargo is running this build script for.
    pub fn discover() -> Result<Self> {
        let directory = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
        let mut command = MetadataCommand::new();
        command.manifest_path(directory.join("Cargo.toml"));
        let metadata = command.clone().no_deps().exec()?;
        let package = metadata
            .packages
            .iter()
            .find(|package| package.manifest_path.parent().map(Utf8Path::as_std_path) == Some(directory.as_path()))
            .context("shader consumer is absent from Cargo metadata")?
            .clone();
        Ok(Self { directory, metadata, package, command })
    }

    /// The name the consumer gives the runtime crate, which Cargo allows it to rename.
    pub fn isthmus_name(&self) -> Result<String> {
        let dependency = self
            .package
            .dependencies
            .iter()
            .find(|dep| dep.name == "isthmus" && dep.kind == DependencyKind::Normal)
            .context("shader consumer must depend on isthmus")?;
        Ok(dependency.rename.as_deref().unwrap_or("isthmus").replace('-', "_"))
    }

    /// The consumer's module path for a source file, relative to its `src` directory.
    pub fn module_path(&self, source: &Path) -> Result<Vec<String>> {
        let module = source.strip_prefix(self.directory.join("src"))?.with_extension("");
        let mut modules: Vec<_> = module.iter().map(|part| part.to_string_lossy().into_owned()).collect();
        if modules.last().is_some_and(|name| matches!(name.as_str(), "mod" | "lib" | "main")) {
            modules.pop();
        }
        Ok(modules)
    }

    /// Features Cargo enabled for the consumer in this build.
    pub fn enabled_features(&self) -> Vec<String> {
        self.package
            .features
            .keys()
            .filter(|feature| {
                env::var_os(format!("CARGO_FEATURE_{}", feature.as_str().replace('-', "_").to_uppercase())).is_some()
            })
            .cloned()
            .collect()
    }

    /// Resolves the dependency graph with exactly the enabled features.
    pub fn resolve(&self, features: &[String]) -> Result<Metadata> {
        Ok(self
            .command
            .clone()
            .features(CargoOpt::NoDefaultFeatures)
            .features(CargoOpt::SomeFeatures(features.to_vec()))
            .exec()?)
    }

    /// Where the shader crate and its build artifacts are cached.
    pub fn cache(&self) -> PathBuf {
        self.metadata.target_directory.join("isthmus").join(self.package.name.as_str()).into_std_path_buf()
    }
}
