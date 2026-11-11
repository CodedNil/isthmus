//! Generates the standalone shader crate: the code reachable from `shader!` declarations.
use graph::Graph;
use isthmus_syntax::shader::Shader;
use items::flatten;
use proc_macro2::TokenStream;
use quote::quote;
use std::{collections::BTreeSet, path::Path};
use syn::visit::{self, Visit};

mod graph;
mod items;
mod loops;
mod prelude;
mod references;
pub mod split;

/// The generated shader crate and what it needs from the build.
pub struct Generated {
    pub source: String,
    pub shaders: Vec<Shader>,
    /// Crates the generated source names, which the shader crate must depend on.
    pub identifiers: BTreeSet<String>,
}

/// Generates the shader crate for the source module at `path`, which the consumer reaches as `root`.
pub fn generate(path: &Path, root: Vec<String>, isthmus: &TokenStream) -> Result<Generated, String> {
    let mut graph = Graph::load_crate(path, root, isthmus)?;
    let shaders = graph.collect_shaders()?;
    graph.close_selection();
    let mut module = graph.emit(0);
    for name in graph.root().iter().rev() {
        let name = syn::Ident::new(name, proc_macro2::Span::call_site());
        module = quote!(pub mod #name { #module });
    }
    let mut source = syn::parse2({
        let prelude = prelude::prelude();
        quote! { #prelude #module }
    })
    .map_err(|error| error.to_string())?;
    loops::lower(&mut source);
    let mut dependencies = Dependencies::default();
    dependencies.visit_file(&source);
    Ok(Generated { source: prettyplease::unparse(&source), shaders, identifiers: dependencies.0 })
}

#[derive(Default)]
struct Dependencies(BTreeSet<String>);
impl<'ast> Visit<'ast> for Dependencies {
    fn visit_path(&mut self, i: &'ast syn::Path) {
        if i.leading_colon.is_some() || i.segments.len() > 1 {
            self.0.insert(i.segments[0].ident.to_string());
        }
        visit::visit_path(self, i);
    }

    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        let mut paths = Vec::new();
        flatten(&i.tree, Vec::new(), &mut paths);
        self.0.extend(paths.into_iter().map(|(_, path)| path.segments[0].ident.to_string()));
    }
}
