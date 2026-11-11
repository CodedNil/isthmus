//! The `Cargo.toml` of the generated shader crate: the consumer's shader-facing dependencies, pinned.
use crate::cargo::Consumer;
use anyhow::{Context as _, Result, bail};
use cargo_metadata::{DependencyKind, Package};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

/// Renders the manifest for a shader crate that uses the dependencies named by `identifiers`.
pub fn render(consumer: &Consumer, identifiers: &BTreeSet<String>) -> Result<String> {
    let features = consumer.enabled_features();
    let resolved = consumer.resolve(&features)?;
    let package = &consumer.package;
    let nodes = &resolved.resolve.as_ref().context("Cargo did not resolve shader dependencies")?.nodes;
    let node = nodes.iter().find(|node| node.id == package.id).context("shader consumer was not resolved")?;
    let mut manifest: toml::Table = format!(
        "[package]\nname = {:?}\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\npath = \"lib.rs\"\n[workspace]\n",
        format!("{}-shader", package.name),
    )
    .parse()?;
    let mut dependencies = toml::Table::new();
    let mut targets = BTreeMap::<String, BTreeMap<&str, toml::Table>>::new();
    for dep in node.deps.iter().filter(|dep| {
        identifiers.contains(&dep.name) && dep.dep_kinds.iter().any(|kind| kind.kind == DependencyKind::Normal)
    }) {
        let requested = package
            .dependencies
            .iter()
            .find(|direct| {
                direct.kind == DependencyKind::Normal && direct.rename.as_deref().unwrap_or(&direct.name) == dep.name
            })
            .with_context(|| format!("shader dependency {} is absent from the consumer manifest", dep.name))?;
        let resolved = &resolved[&dep.pkg];
        let mut value = toml::toml! {
            package = (resolved.name.as_str())
            default-features = false
            features = (requested.features.clone())
        };
        locate(&mut value, resolved)?;
        for kind in dep.dep_kinds.iter().filter(|kind| kind.kind == DependencyKind::Normal) {
            let table = kind.target.as_ref().map_or(&mut dependencies, |target| {
                targets.entry(target.to_string()).or_default().entry("dependencies").or_default()
            });
            table.insert(dep.name.clone(), value.clone().into());
        }
    }
    manifest.insert("dependencies".into(), dependencies.into());
    manifest.insert("target".into(), toml::Value::try_from(targets)?);
    manifest.insert("features".into(), feature_table(package, features).into());
    if let Some(patch) = patches(consumer)? {
        manifest.insert("patch".into(), patch);
    }
    Ok(toml::to_string(&manifest)?)
}

/// Points a shader dependency at the same source Cargo resolved for the consumer.
fn locate(value: &mut toml::Table, package: &Package) -> Result<()> {
    match package.source.as_ref().map(|source| source.repr.as_str()) {
        None => {
            let directory = package.manifest_path.parent().context("manifest has a directory")?;
            value.insert("path".into(), directory.as_str().into());
        }
        Some(source) if let Some(index) = source.strip_prefix("registry+") => {
            value.insert("registry-index".into(), index.into());
            value.insert("version".into(), format!("={}", package.version).into());
        }
        Some(source) if let Some(git) = source.strip_prefix("git+") => {
            let git = git.split('#').next().context("git source has a URL")?;
            let (url, reference) = git.split_once('?').unwrap_or((git, ""));
            value.insert("git".into(), url.into());
            value.extend(
                form_urlencoded::parse(reference.as_bytes())
                    .map(|(key, value)| (key.into_owned(), value.into_owned().into())),
            );
        }
        Some(source) => bail!("unsupported shader dependency source: {source}"),
    }
    Ok(())
}

/// Declares every consumer feature, enabling by default only those Cargo enabled for this build.
fn feature_table(package: &Package, enabled: Vec<String>) -> toml::Table {
    let mut features =
        package.features.keys().map(|name| (name.clone(), toml::Value::Array(Vec::new()))).collect::<toml::Table>();
    features.insert("default".into(), enabled.into_iter().filter(|name| name != "default").collect::<Vec<_>>().into());
    features
}

/// The workspace's `[patch]` tables, with relative paths made absolute for the cached crate.
fn patches(consumer: &Consumer) -> Result<Option<toml::Value>> {
    let root = &consumer.metadata.workspace_root;
    let mut workspace: toml::Table = fs::read_to_string(root.join("Cargo.toml"))?.parse()?;
    let Some(mut patch) = workspace.remove("patch") else { return Ok(None) };
    for (_, deps) in patch.as_table_mut().context("patch must be a table")? {
        for (_, dep) in deps.as_table_mut().context("patch source must be a table")? {
            if let Some(path) = dep.get_mut("path") {
                *path = root.join(path.as_str().context("patch path must be a string")?).as_str().into();
            }
        }
    }
    Ok(Some(patch))
}
