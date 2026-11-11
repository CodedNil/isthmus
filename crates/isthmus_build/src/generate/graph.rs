//! The consumer crate's modules and the reachability walk that selects the code shaders need.
use super::{
    items::{flatten, item_name, module_directory, segments, shader_derives},
    references::References,
};
use crate::cargo::directive;
use isthmus_syntax::{
    program::Declaration,
    shader::{Shader, SliceIndices},
};
use proc_macro2::TokenStream;
use quote::{ToTokens as _, quote};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};
use syn::{Item, visit::Visit as _, visit_mut::VisitMut as _};

/// One source file (or inline module) of the consumer crate, with its items and name resolution state.
#[derive(Default)]
struct Module {
    file: PathBuf,
    parent: Option<usize>,
    items: Vec<Item>,
    names: BTreeMap<String, usize>,
    children: BTreeMap<String, usize>,
    imports: BTreeMap<String, syn::Path>,
    globs: Vec<syn::Path>,
    traits: Vec<syn::Path>,
    used_imports: BTreeSet<String>,
    selected: BTreeSet<usize>,
    impl_members: BTreeSet<(usize, usize)>,
    shaders: Vec<TokenStream>,
}

/// The consumer's module tree, and which items the shaders reach.
#[derive(Default)]
pub(super) struct Graph {
    root: Vec<String>,
    isthmus: TokenStream,
    modules: Vec<Module>,
    pending: VecDeque<(usize, usize)>,
    methods: HashSet<String>,
}

impl Graph {
    /// Loads the module tree rooted at `path`, which the consumer reaches through the `root` module path.
    pub(super) fn load_crate(path: &Path, root: Vec<String>, isthmus: &TokenStream) -> Result<Self, String> {
        let mut graph = Self { root, isthmus: isthmus.clone(), ..Self::default() };
        graph.load(path, &module_directory(path), None, None)?;
        Ok(graph)
    }

    /// Finds every `shader!` declaration and selects the items each one reaches.
    pub(super) fn collect_shaders(&mut self) -> Result<Vec<Shader>, String> {
        let mut entries: Vec<Shader> = Vec::new();
        for scope in 0..self.modules.len() {
            let mut shaders = References::default();
            for item in &self.modules[scope].items {
                shaders.visit_item(item);
            }
            for (declaration, location, outer_locals) in shaders.declarations {
                let file = self.modules[scope].file.display();
                let shader = Shader::parse(declaration.tokens.clone(), &self.isthmus)
                    .map_err(|error| format!("{file}:{}: {error}", location.line))?;
                if entries.iter().any(|other| other.entry.value() == shader.entry.value()) {
                    return Err(format!(
                        "{file}:{}: shader name {} is already used; names must be unique within a crate",
                        location.line,
                        shader.name.value()
                    ));
                }
                let mut refs = References::default();
                refs.visit_expr_closure(&shader.declaration);
                if let Some(path) = refs
                    .paths
                    .iter()
                    .find(|path| path.get_ident().is_some_and(|name| outer_locals.contains(&name.to_string())))
                {
                    return Err(format!(
                        "{file}:{}: shader reference {} belongs to the surrounding function; pass values as typed captures and place shared helper items at module scope",
                        location.line,
                        path.to_token_stream()
                    ));
                }
                self.follow(scope, refs);
                let gpu = shader.gpu(&self.isthmus);
                self.modules[scope].shaders.push(gpu);
                entries.push(shader);
            }
        }
        Ok(entries)
    }

    /// Follows references from the selected items until no further item or impl member is reached.
    pub(super) fn close_selection(&mut self) {
        loop {
            while let Some((scope, index)) = self.pending.pop_front() {
                let mut refs = References::default();
                refs.visit_item(&self.modules[scope].items[index]);
                self.follow(scope, refs);
            }
            let additions = self.unselected_impl_members();
            if additions.is_empty() {
                break;
            }
            for (scope, index, member) in additions {
                self.modules[scope].impl_members.insert((index, member));
                let Item::Impl(item) = &self.modules[scope].items[index] else { continue };
                let mut refs = References::default();
                refs.visit_generics(&item.generics);
                refs.visit_impl_item(&item.items[member]);
                if let Some((path, _)) = &item.trait_ {
                    refs.paths.push(path.clone());
                }
                self.follow(scope, refs);
            }
        }
    }

    /// Members of impls on selected types that are trait items, or are called by name from selected code.
    fn unselected_impl_members(&self) -> Vec<(usize, usize, usize)> {
        let mut additions = Vec::new();
        for (scope, module) in self.modules.iter().enumerate() {
            for (index, item) in module.items.iter().enumerate() {
                let Item::Impl(item) = item else { continue };
                let syn::Type::Path(ty) = &*item.self_ty else { continue };
                let Some(name) = ty.path.segments.last() else { continue };
                let Some(target) = module.names.get(&name.ident.to_string()) else { continue };
                if !module.selected.contains(target) {
                    continue;
                }
                for (member, declaration) in item.items.iter().enumerate() {
                    let name = match declaration {
                        syn::ImplItem::Fn(item) => Some(&item.sig.ident),
                        syn::ImplItem::Const(item) => Some(&item.ident),
                        syn::ImplItem::Type(item) => Some(&item.ident),
                        _ => None,
                    };
                    if (item.trait_.is_some() || name.is_some_and(|name| self.methods.contains(&name.to_string())))
                        && !module.impl_members.contains(&(index, member))
                    {
                        additions.push((scope, index, member));
                    }
                }
            }
        }
        additions
    }

    fn load(
        &mut self,
        file: &Path,
        directory: &Path,
        parent: Option<usize>,
        inline: Option<Vec<Item>>,
    ) -> Result<usize, String> {
        let items = if let Some(items) = inline {
            items
        } else {
            directive(format_args!("rerun-if-changed={}", file.display()));
            let text = fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
            syn::parse_file(&text).map_err(|error| format!("{}: {error}", file.display()))?.items
        };
        let mut expanded = Vec::new();
        for item in items {
            if let Item::Macro(declaration) = &item
                && declaration.mac.path.segments.last().is_some_and(|segment| segment.ident == "program")
            {
                let program = Declaration::parse(declaration.mac.tokens.clone()).map_err(|error| error.to_string())?;
                let file: syn::File = syn::parse2(program.gpu(&self.isthmus)).map_err(|error| error.to_string())?;
                expanded.extend(file.items);
            } else {
                expanded.push(item);
            }
        }
        let items = expanded;
        let scope = self.modules.len();
        self.modules.push(Module { file: file.to_path_buf(), parent, ..Module::default() });
        for (index, item) in items.iter().enumerate() {
            if let Some(name) = item_name(item) {
                self.modules[scope].names.insert(name, index);
            }
            if let Item::Use(import) = item {
                let mut flattened = Vec::new();
                flatten(&import.tree, Vec::new(), &mut flattened);
                for (alias, path) in flattened {
                    if alias == "_" {
                        self.modules[scope].traits.push(path);
                    } else if alias == "*" {
                        self.modules[scope].globs.push(path);
                    } else {
                        self.modules[scope].imports.insert(alias, path);
                    }
                }
            }
            if let Item::Mod(module) = item {
                let child = if let Some((_, items)) = &module.content {
                    self.load(file, &directory.join(module.ident.to_string()), Some(scope), Some(items.clone()))?
                } else {
                    let child = directory.join(format!("{}.rs", module.ident));
                    let child =
                        if child.exists() { child } else { directory.join(module.ident.to_string()).join("mod.rs") };
                    self.load(&child, &module_directory(&child), Some(scope), None)?
                };
                self.modules[scope].children.insert(module.ident.to_string(), child);
            }
        }
        self.modules[scope].items = items;
        Ok(scope)
    }

    fn follow(&mut self, scope: usize, refs: References) {
        self.methods.extend(refs.methods);
        for path in refs.paths {
            if let Some((scope, index)) = self.resolve(scope, &segments(&path).collect::<Vec<_>>(), &mut HashSet::new())
                && self.modules[scope].selected.insert(index)
            {
                self.pending.push_back((scope, index));
            }
        }
    }

    fn resolve(
        &mut self,
        scope: usize,
        parts: &[String],
        seen: &mut HashSet<(usize, Vec<String>)>,
    ) -> Option<(usize, usize)> {
        if !seen.insert((scope, parts.to_vec())) {
            return None;
        }
        let (first, rest) = parts.split_first()?;
        match first.as_str() {
            "crate" if rest.starts_with(&self.root) => return self.resolve(0, &rest[self.root.len()..], seen),
            "self" => return self.resolve(scope, rest, seen),
            "super" => return self.resolve(self.modules[scope].parent?, rest, seen),
            _ => {}
        }
        if let Some(&child) = self.modules[scope].children.get(first) {
            return self.resolve(child, rest, seen);
        }
        if let Some(&index) = self.modules[scope].names.get(first) {
            return Some((scope, index));
        }
        if let Some(path) = self.modules[scope].imports.get(first).cloned() {
            self.modules[scope].used_imports.insert(first.clone());
            return self.resolve(scope, &segments(&path).chain(rest.iter().cloned()).collect::<Vec<_>>(), seen);
        }
        for glob in self.modules[scope].globs.clone() {
            if !seen.insert((scope, vec!["*".into(), glob.to_token_stream().to_string()])) {
                continue;
            }
            if let Some(target) =
                self.resolve(scope, &segments(&glob).chain(parts.iter().cloned()).collect::<Vec<_>>(), seen)
            {
                return Some(target);
            }
        }
        None
    }

    /// The module path the consumer reaches the loaded source through.
    pub(super) fn root(&self) -> &[String] {
        &self.root
    }

    /// The selected items of the module at `scope` and its children, as source for the shader crate.
    pub(super) fn emit(&self, scope: usize) -> TokenStream {
        let module = &self.modules[scope];
        let mut output = TokenStream::new();
        for (name, child) in &module.children {
            let content = self.emit(*child);
            if !content.is_empty() {
                let name = syn::Ident::new(name, proc_macro2::Span::call_site());
                let origin = format!("Source: {}", self.modules[*child].file.display());
                output.extend(quote!(#[doc = #origin] pub mod #name { #content }));
            }
        }
        for &index in &module.selected {
            let mut item = module.items[index].clone();
            if let Item::Fn(function) = &mut item {
                rewrite_slices(&mut function.sig, &mut function.block);
            }
            if let Item::Struct(item) = &mut item {
                for attribute in &mut item.attrs {
                    if let Some(paths) = shader_derives(attribute) {
                        *attribute = syn::parse_quote!(#[derive(#paths)]);
                    }
                }
            }
            output.extend(item.to_token_stream());
        }
        for (index, item) in module.items.iter().enumerate() {
            if let Item::Impl(item) = item {
                let mut item = item.clone();
                item.items = item
                    .items
                    .into_iter()
                    .enumerate()
                    .filter_map(|(member, item)| module.impl_members.contains(&(index, member)).then_some(item))
                    .collect();
                for member in &mut item.items {
                    if let syn::ImplItem::Fn(function) = member {
                        rewrite_slices(&mut function.sig, &mut function.block);
                    }
                }
                if !item.items.is_empty() {
                    output.extend(item.to_token_stream());
                }
            }
        }
        output.extend(module.shaders.iter().cloned());
        if output.is_empty() {
            return output;
        }
        let imports =
            module.imports.iter().filter(|(name, _)| module.used_imports.contains(*name)).map(|(name, path)| {
                let alias = syn::Ident::new(name, proc_macro2::Span::call_site());
                if path.segments.last().is_some_and(|segment| segment.ident == alias) {
                    quote!(use #path;)
                } else {
                    quote!(use #path as #alias;)
                }
            });
        let globs = &module.globs;
        let traits = &module.traits;
        quote! { #(#imports)* #(use #globs::*;)* #(use #traits as _;)* #output }
    }
}

fn rewrite_slices(signature: &mut syn::Signature, block: &mut syn::Block) {
    let mut names = BTreeSet::new();
    for argument in &mut signature.inputs {
        let syn::FnArg::Typed(argument) = argument else { continue };
        let syn::Type::Reference(reference) = &*argument.ty else { continue };
        let syn::Type::Slice(slice) = &*reference.elem else { continue };
        let syn::Pat::Ident(name) = &*argument.pat else { continue };
        let ident = name.ident.clone();
        let element = &slice.elem;
        *argument.ty = syn::parse_quote!(crate::ShaderSlice<'_, #element>);
        names.insert(ident.to_string());
    }
    SliceIndices(names).visit_block_mut(block);
}
