//! Keep generated Rust files below SPIR-V's `OpSource` instruction-size limit.
//! Rust-GPU 0.11 can truncate a large source and then panic while reconstructing
//! diagnostic spans. Physical modules preserve lexical scopes and literal text.
use crate::cargo::write_if_changed;
use anyhow::{Context as _, Result};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use syn::{AttrStyle, File, Item, token::Semi};

pub fn write(path: &Path, source: &str) -> Result<()> {
    let root = path.parent().context("shader crate directory")?;
    let mut written = BTreeSet::new();
    write_file(path, root, syn::parse_file(source).context("parse generated shader crate")?, &mut written)?;
    remove_stale(root, &written)
}

/// Files from earlier layouts would otherwise shadow or conflict with current modules.
fn remove_stale(directory: &Path, written: &BTreeSet<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            remove_stale(&path, written)?;
            // Only succeeds once the directory is empty.
            drop(fs::remove_dir(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") && !written.contains(&path) {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Child modules of `path` live in `children`, following Rust's own file layout.
fn write_file(path: &Path, children: &Path, mut file: File, written: &mut BTreeSet<PathBuf>) -> Result<()> {
    for item in &mut file.items {
        let Item::Mod(module) = item else {
            continue;
        };
        let Some((_, items)) = module.content.take() else {
            continue;
        };
        let mut inner = Vec::new();
        module.attrs.retain(|attribute| {
            if matches!(attribute.style, AttrStyle::Inner(_)) {
                inner.push(attribute.clone());
                false
            } else {
                !attribute.path().is_ident("path")
            }
        });
        let child = children.join(format!("{}.rs", module.ident));
        let file = File { shebang: None, frontmatter: None, attrs: inner, items };
        write_file(&child, &children.join(module.ident.to_string()), file, written)?;
        module.semi = Some(Semi::default());
    }
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    written.insert(path.to_path_buf());
    write_if_changed(path, &prettyplease::unparse(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        process::{self, Command},
    };

    #[test]
    fn splitting_preserves_nested_scope_inner_attributes_and_raw_strings() {
        let directory = env::temp_dir().join(format!("isthmus-module-test-{}", process::id()));
        fs::create_dir_all(&directory).expect("test directory");
        let root = directory.join("fixture.rs");
        let source = r##"
mod parent {
    #![allow(dead_code)]
    const NUMBER: u32 = 7;
    mod child {
        pub(super) fn number() -> u32 { super::NUMBER }
        pub(super) fn text() -> &'static str { r#"first
    preserve this indentation"# }
    }
    pub fn verify() {
        assert_eq!(child::number(), 7);
        assert_eq!(child::text(), "first\n    preserve this indentation");
    }
}
fn main() { parent::verify(); }
"##;
        write(&root, source).expect("split fixture");
        let parent = fs::read_to_string(directory.join("parent.rs")).expect("parent source");
        assert!(parent.contains("#![allow(dead_code)]"), "module inner attributes moved incorrectly");
        let binary = directory.join("fixture");
        let output =
            Command::new("rustc").arg(&root).arg("--edition=2024").arg("-o").arg(&binary).output().expect("rustc");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(Command::new(binary).status().expect("fixture execution").success(), "module semantics changed");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
