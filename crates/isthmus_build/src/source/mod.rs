mod references;
use crate::syntax::{Shader, program};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use references::{DefaultFields, References};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use syn::{Item, UseTree, punctuated::Punctuated, visit::Visit};

pub struct Generated {
    pub files: BTreeMap<PathBuf, String>,
    pub shaders: Vec<Shader>,
}

pub fn generate(path: &Path) -> Result<Generated, String> {
    let mut entries = Vec::new();
    let mut graph = Graph::default();
    graph.load(path, None, None)?;
    for scope in 0..graph.modules.len() {
        let mut shaders = References::default();
        for item in &graph.modules[scope].items {
            shaders.visit_item(item);
        }
        for (declaration, location, outer_locals) in shaders.declarations {
            let shader = Shader::parse(
                declaration.tokens.clone(),
                &graph.modules[scope].file.to_string_lossy(),
                location.line,
                location.column,
            )
            .map_err(|error| format!("{}:{}: {error}", graph.modules[scope].file.display(), location.line))?;
            let mut refs = References::default();
            refs.visit_expr_closure(&shader.declaration);
            if let Some(path) = refs
                .paths
                .iter()
                .find(|path| path.get_ident().is_some_and(|name| outer_locals.contains(&name.to_string())))
            {
                return Err(format!(
                    "{}:{}: shader reference {} belongs to the surrounding function; pass values as typed captures and place shared helper items at module scope",
                    graph.modules[scope].file.display(),
                    location.line,
                    path.to_token_stream()
                ));
            }
            graph.follow(scope, refs);
            graph.modules[scope].shaders.push(shader.gpu(&quote!(::isthmus)));
            entries.push(shader);
        }
    }
    loop {
        while let Some((scope, index)) = graph.pending.pop_front() {
            let mut refs = References::default();
            refs.visit_item(&graph.modules[scope].items[index]);
            graph.follow(scope, refs);
        }
        let mut additions = Vec::new();
        for scope in 0..graph.modules.len() {
            for (index, item) in graph.modules[scope].items.iter().enumerate() {
                let Item::Impl(item) = item else { continue };
                let syn::Type::Path(ty) = &*item.self_ty else { continue };
                let Some(name) = ty.path.get_ident() else { continue };
                let Some(target) = graph.modules[scope].names.get(&name.to_string()) else { continue };
                if !graph.modules[scope].selected.contains(target) {
                    continue;
                }
                for (member, declaration) in item.items.iter().enumerate() {
                    if (item.trait_.is_some()
                        || member_name(declaration).is_some_and(|name| graph.methods.contains(&name)))
                        && !graph.modules[scope].impl_members.contains(&(index, member))
                    {
                        additions.push((scope, index, member));
                    }
                }
            }
        }
        if additions.is_empty() {
            break;
        }
        for (scope, index, member) in additions {
            graph.modules[scope].impl_members.insert((index, member));
            let Item::Impl(item) = &graph.modules[scope].items[index] else { continue };
            let mut refs = References::default();
            refs.visit_impl_item(&item.items[member]);
            if let Some((path, _)) = &item.trait_ {
                refs.paths.push(path.clone());
            }
            graph.follow(scope, refs);
        }
    }
    let mut files = BTreeMap::new();
    let items = graph.emit(0, Path::new("render"), &mut files);
    files.insert(PathBuf::from("render/mod.rs"), items);
    let mut defaults = DefaultFields(false);
    for items in files.values() {
        defaults.visit_file(&syn::parse2(items.clone()).map_err(|error| error.to_string())?);
    }
    let feature = defaults.0.then(|| quote!(#![feature(default_field_values)]));
    files.insert(PathBuf::from("lib.rs"), quote! {
        #![no_std]
        #feature
        #![allow(dead_code, unused_imports, reason = "shader extraction conservatively retains shared methods and trait imports")]
        pub mod render;
    });
    let files = files
        .into_iter()
        .map(|(path, source)| Ok((path, format(&source.to_string())?)))
        .collect::<Result<_, String>>()?;
    Ok(Generated { files, shaders: entries })
}

pub fn format(source: &str) -> Result<String, String> {
    let mut formatter = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout", "--config", "skip_children=true"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start rustfmt for generated shaders: {error}"))?;
    formatter
        .stdin
        .take()
        .ok_or("rustfmt stdin is unavailable")?
        .write_all(source.as_bytes())
        .map_err(|error| format!("failed to send generated shaders to rustfmt: {error}"))?;
    let output =
        formatter.wait_with_output().map_err(|error| format!("failed to format generated shaders: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

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

#[derive(Default)]
struct Graph {
    modules: Vec<Module>,
    pending: VecDeque<(usize, usize)>,
    methods: HashSet<String>,
}

impl Graph {
    fn load(&mut self, file: &Path, parent: Option<usize>, inline: Option<Vec<Item>>) -> Result<usize, String> {
        let items = if let Some(items) = inline {
            items
        } else {
            println!("cargo:rerun-if-changed={}", file.display());
            let text = fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
            syn::parse_file(&text).map_err(|error| format!("{}: {error}", file.display()))?.items
        };
        let mut expanded = Vec::new();
        for item in items {
            if let Item::Macro(declaration) = &item
                && declaration.mac.path.segments.last().is_some_and(|segment| segment.ident == "program")
            {
                let globals: syn::Type = if declaration.mac.tokens.is_empty() {
                    syn::parse_quote!(())
                } else {
                    syn::parse2(declaration.mac.tokens.clone()).map_err(|error| error.to_string())?
                };
                let shared = program(&quote!(::isthmus));
                let file: syn::File = syn::parse2(quote! {
                    #shared
                    unsafe impl ::isthmus::Program for Program { type Globals = #globals; }
                })
                .map_err(|error| error.to_string())?;
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
                    self.load(file, Some(scope), Some(items.clone()))?
                } else {
                    let parent = file.parent().ok_or("shader source needs a parent directory")?;
                    let explicit = module.attrs.iter().find_map(|attribute| {
                        if attribute.path().is_ident("path")
                            && let syn::Meta::NameValue(value) = &attribute.meta
                            && let syn::Expr::Lit(value) = &value.value
                            && let syn::Lit::Str(value) = &value.lit
                        {
                            Some(value.value())
                        } else {
                            None
                        }
                    });
                    let child = explicit.map_or_else(
                        || {
                            let directory =
                                if file.ends_with("mod.rs") { parent.to_path_buf() } else { file.with_extension("") };
                            let child = directory.join(format!("{}.rs", module.ident));
                            if child.exists() { child } else { directory.join(module.ident.to_string()).join("mod.rs") }
                        },
                        |path| parent.join(path),
                    );
                    self.load(&child, Some(scope), None)?
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
            let parts = path.segments.iter().map(|segment| segment.ident.to_string()).collect::<Vec<_>>();
            if let Some((scope, index)) = self.resolve(scope, &parts, &mut HashSet::new())
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
            "crate" if rest.first().is_some_and(|name| name == "render") => return self.resolve(0, &rest[1..], seen),
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
            let path = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .chain(rest.iter().cloned())
                .collect::<Vec<_>>();
            return self.resolve(scope, &path, seen);
        }
        for glob in self.modules[scope].globs.clone() {
            let path = glob
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .chain(parts.iter().cloned())
                .collect::<Vec<_>>();
            if let Some(target) = self.resolve(scope, &path, seen) {
                return Some(target);
            }
        }
        None
    }

    fn emit(&self, scope: usize, directory: &Path, files: &mut BTreeMap<PathBuf, TokenStream>) -> TokenStream {
        let module = &self.modules[scope];
        let mut output = TokenStream::new();
        for (name, child) in &module.children {
            let path = directory.join(name.trim_start_matches("r#"));
            let content = self.emit(*child, &path, files);
            if !content.is_empty() {
                files.insert(path.join("mod.rs"), content);
                let name = syn::Ident::new(name, proc_macro2::Span::call_site());
                let origin = format!("Source: {}", self.modules[*child].file.display());
                output.extend(quote!(#[doc = #origin] pub mod #name;));
            }
        }
        for &index in &module.selected {
            let mut item = module.items[index].clone();
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

fn item_name(item: &Item) -> Option<String> {
    Some(
        match item {
            Item::Const(item) => &item.ident,
            Item::Static(item) => &item.ident,
            Item::Fn(item) => &item.sig.ident,
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::Union(item) => &item.ident,
            _ => return None,
        }
        .to_string(),
    )
}

fn member_name(item: &syn::ImplItem) -> Option<String> {
    Some(
        match item {
            syn::ImplItem::Fn(item) => &item.sig.ident,
            syn::ImplItem::Const(item) => &item.ident,
            syn::ImplItem::Type(item) => &item.ident,
            _ => return None,
        }
        .to_string(),
    )
}

fn flatten(tree: &UseTree, mut prefix: Vec<syn::Ident>, output: &mut Vec<(String, syn::Path)>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.clone());
            flatten(&path.tree, prefix, output);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                flatten(tree, prefix.clone(), output);
            }
        }
        UseTree::Name(name) => {
            if name.ident != "self" {
                prefix.push(name.ident.clone());
            }
            if let Some(name) = prefix.last() {
                output.push((name.to_string(), syn::parse_quote!(#(#prefix)::*)));
            }
        }
        UseTree::Rename(rename) => {
            if rename.ident != "self" {
                prefix.push(rename.ident.clone());
            }
            output.push((rename.rename.to_string(), syn::parse_quote!(#(#prefix)::*)));
        }
        UseTree::Glob(_) => output.push((String::from("*"), syn::parse_quote!(#(#prefix)::*))),
    }
}

fn shader_derives(attribute: &syn::Attribute) -> Option<Punctuated<syn::Path, syn::Token![,]>> {
    if !attribute.path().is_ident("derive") {
        return None;
    }
    Some(
        attribute
            .parse_args_with(Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
            .ok()?
            .into_iter()
            .filter(|path| {
                path.segments
                    .last()
                    .is_none_or(|segment| segment.ident != "Deserialize" && segment.ident != "Serialize")
            })
            .collect(),
    )
}
