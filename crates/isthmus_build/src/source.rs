use crate::syntax::{Shader, program, vertex};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    fs,
    io::Write,
    mem,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use syn::{
    Item, UseTree,
    punctuated::Punctuated,
    spanned::Spanned,
    visit::{self, Visit},
};

pub struct Generated {
    pub source: String,
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
                let needed =
                    item.trait_.as_ref().is_some_and(|(path, _)| {
                        path.segments.last().is_some_and(|segment| segment.ident == "Program")
                    }) || item
                        .items
                        .iter()
                        .any(|member| member_name(member).is_some_and(|name| graph.methods.contains(&name)));
                for (member, declaration) in item.items.iter().enumerate() {
                    if (item.trait_.is_some() && needed
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
    let items = graph.emit(0);
    let vertex = vertex(&quote!(::isthmus));
    let mut defaults = DefaultFields(false);
    defaults.visit_file(&syn::parse2(items.clone()).map_err(|error| error.to_string())?);
    let feature = defaults.0.then(|| quote!(#![feature(default_field_values)]));
    let generated = quote! {
        #![no_std]
        #feature
        #![allow(dead_code, unused_imports, reason = "shader extraction conservatively retains shared methods and trait imports")]
        pub mod render { #items #vertex }
    }.to_string();
    let mut formatter = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start rustfmt for generated shaders: {error}"))?;
    formatter
        .stdin
        .take()
        .ok_or("rustfmt stdin is unavailable")?
        .write_all(generated.as_bytes())
        .map_err(|error| format!("failed to send generated shaders to rustfmt: {error}"))?;
    let output =
        formatter.wait_with_output().map_err(|error| format!("failed to format generated shaders: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(Generated { source: String::from_utf8(output.stdout).map_err(|error| error.to_string())?, shaders: entries })
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

    fn emit(&self, scope: usize) -> TokenStream {
        let module = &self.modules[scope];
        let mut output = TokenStream::new();
        for (name, child) in &module.children {
            let content = self.emit(*child);
            if !content.is_empty() {
                let name = syn::Ident::new(name, proc_macro2::Span::call_site());
                let origin = self.modules[*child].file.to_string_lossy();
                output.extend(quote!(#[doc = #origin] pub mod #name { #content }));
            }
        }
        for &index in &module.selected {
            let mut item = module.items[index].clone();
            if let Item::Struct(item) = &mut item {
                for attribute in item.attrs.iter_mut().filter(|attr| attr.path().is_ident("derive")) {
                    if let Ok(mut paths) =
                        attribute.parse_args_with(Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
                    {
                        paths = paths
                            .into_iter()
                            .filter(|path| {
                                path.segments.last().is_none_or(|segment| {
                                    segment.ident != "Deserialize" && segment.ident != "Serialize"
                                })
                            })
                            .collect();
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

#[derive(Default)]
struct References {
    paths: Vec<syn::Path>,
    methods: HashSet<String>,
    locals: Vec<HashSet<String>>,
    declarations: Vec<(syn::Macro, proc_macro2::LineColumn, HashSet<String>)>,
}

impl References {
    fn bind(&mut self, pattern: &syn::Pat) {
        if self.locals.is_empty() {
            self.locals.push(HashSet::new());
        }
        let mut names = Bindings(self.locals.last_mut().expect("a local scope was created"));
        names.visit_pat(pattern);
    }
}

impl<'ast> Visit<'ast> for References {
    fn visit_attribute(&mut self, i: &'ast syn::Attribute) {
        if i.path().is_ident("derive")
            && let Ok(paths) = i.parse_args_with(Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
        {
            self.paths.extend(paths.into_iter().filter(|path| {
                path.segments
                    .last()
                    .is_none_or(|segment| segment.ident != "Deserialize" && segment.ident != "Serialize")
            }));
        }
    }

    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        if i.path.segments.last().is_some_and(|segment| segment.ident == "shader") {
            self.declarations.push((i.clone(), i.path.span().start(), self.locals.iter().flatten().cloned().collect()));
        }
    }

    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}

    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if i.path
            .get_ident()
            .is_none_or(|name| !self.locals.iter().rev().any(|scope| scope.contains(&name.to_string())))
        {
            self.paths.push(i.path.clone());
        }
        if i.path.segments.len() > 1 {
            self.methods.insert(i.path.segments.last().expect("path has segments").ident.to_string());
        }
        visit::visit_expr_path(self, i);
    }

    fn visit_type_path(&mut self, i: &'ast syn::TypePath) {
        self.paths.push(i.path.clone());
        visit::visit_type_path(self, i);
    }

    fn visit_expr_struct(&mut self, i: &'ast syn::ExprStruct) {
        self.paths.push(i.path.clone());
        visit::visit_expr_struct(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        self.methods.insert(i.method.to_string());
        visit::visit_expr_method_call(self, i);
    }

    fn visit_block(&mut self, i: &'ast syn::Block) {
        self.locals.push(HashSet::new());
        for statement in &i.stmts {
            if let syn::Stmt::Item(item) = statement
                && let Some(name) = item_name(item)
            {
                self.locals.last_mut().expect("a block scope was created").insert(name);
            }
        }
        visit::visit_block(self, i);
        self.locals.pop();
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let outer = mem::take(&mut self.locals);
        visit::visit_item_fn(self, i);
        self.locals = outer;
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let outer = mem::take(&mut self.locals);
        visit::visit_impl_item_fn(self, i);
        self.locals = outer;
    }

    fn visit_trait_bound(&mut self, i: &'ast syn::TraitBound) {
        self.paths.push(i.path.clone());
        visit::visit_trait_bound(self, i);
    }

    fn visit_pat_struct(&mut self, i: &'ast syn::PatStruct) {
        self.paths.push(i.path.clone());
        visit::visit_pat_struct(self, i);
    }

    fn visit_pat_tuple_struct(&mut self, i: &'ast syn::PatTupleStruct) {
        self.paths.push(i.path.clone());
        visit::visit_pat_tuple_struct(self, i);
    }

    fn visit_local(&mut self, i: &'ast syn::Local) {
        if let Some(init) = &i.init {
            self.visit_local_init(init);
        }
        self.visit_pat(&i.pat);
        self.bind(&i.pat);
    }

    fn visit_fn_arg(&mut self, i: &'ast syn::FnArg) {
        visit::visit_fn_arg(self, i);
        if let syn::FnArg::Typed(input) = i {
            self.bind(&input.pat);
        }
    }

    fn visit_expr_closure(&mut self, i: &'ast syn::ExprClosure) {
        self.locals.push(HashSet::new());
        for input in &i.inputs {
            self.visit_pat(input);
            self.bind(input);
        }
        self.visit_expr(&i.body);
        self.locals.pop();
    }

    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        self.visit_expr(&i.expr);
        self.locals.push(HashSet::new());
        self.bind(&i.pat);
        self.visit_block(&i.body);
        self.locals.pop();
    }

    fn visit_arm(&mut self, i: &'ast syn::Arm) {
        self.locals.push(HashSet::new());
        self.bind(&i.pat);
        visit::visit_arm(self, i);
        self.locals.pop();
    }

    fn visit_expr_let(&mut self, i: &'ast syn::ExprLet) {
        self.visit_expr(&i.expr);
        self.visit_pat(&i.pat);
        self.bind(&i.pat);
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        self.locals.push(HashSet::new());
        self.visit_expr(&i.cond);
        self.visit_block(&i.then_branch);
        self.locals.pop();
        if let Some((_, branch)) = &i.else_branch {
            self.visit_expr(branch);
        }
    }

    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        let mut imports = Vec::new();
        flatten(&i.tree, Vec::new(), &mut imports);
        self.paths.extend(imports.into_iter().map(|(_, path)| path));
    }

    fn visit_expr_while(&mut self, i: &'ast syn::ExprWhile) {
        self.locals.push(HashSet::new());
        self.visit_expr(&i.cond);
        self.visit_block(&i.body);
        self.locals.pop();
    }
}

struct Bindings<'a>(&'a mut HashSet<String>);
impl<'ast> Visit<'ast> for Bindings<'_> {
    fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
        self.0.insert(i.ident.to_string());
        visit::visit_pat_ident(self, i);
    }
}

struct DefaultFields(bool);
impl<'ast> Visit<'ast> for DefaultFields {
    fn visit_field(&mut self, i: &'ast syn::Field) {
        self.0 |= i.default.is_some();
        visit::visit_field(self, i);
    }

    fn visit_expr_struct(&mut self, i: &'ast syn::ExprStruct) {
        self.0 |= i.dot2_token.is_some() && i.rest.is_none();
        visit::visit_expr_struct(self, i);
    }
}

#[cfg(test)]
mod tests {
    use super::generate;
    use std::{
        env, fs,
        path::Path,
        process,
        sync::atomic::{AtomicUsize, Ordering},
    };

    fn try_extract(source: &str) -> Result<String, String> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = env::temp_dir().join(format!(
            "isthmus-extract-{}-{}.rs",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, source).unwrap();
        let result = generate(&path).map(|generated| generated.source);
        fs::remove_file(path).unwrap();
        result
    }

    fn extract(source: &str) -> String {
        try_extract(source).unwrap()
    }

    #[test]
    fn rejects_implicit_and_unsupported_captures() {
        let error = try_extract(
            "const GAIN: f32 = 1.0;
            fn draw() {
                const GAIN: f32 = 2.0;
                isthmus::shader!(|fragment: Fragment| Vec4::splat(GAIN));
            }",
        )
        .unwrap_err();
        assert!(error.contains("GAIN belongs to the surrounding function"));
        let generated = extract(
            "fn draw(gain: f32) {
                isthmus::shader!(|fragment: Fragment, gain: f32| Vec4::splat(gain));
            }",
        );
        assert!(generated.contains("gain: f32"));
        let error = try_extract(
            "fn draw() {
                isthmus::shader!(|fragment: Fragment, image: Image, other: Image| Vec4::ONE);
            }",
        )
        .unwrap_err();
        assert!(error.contains("a shader may capture one Image"));
    }

    #[test]
    fn compiles_image_early_returns_and_shared_defaults_to_spirv() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output = workspace.join("target/isthmus/extraction-test");
        fs::create_dir_all(&output).unwrap();
        crate::ShaderBuild {
            name: "extraction-test".into(),
            source: workspace.join("crates/isthmus_build/tests/fixtures/render.rs"),
            isthmus: workspace.join("crates/isthmus"),
            workspace,
            output: output.join("shader.spv"),
        }
        .build()
        .unwrap();
        let manifest = fs::read_to_string(output.join("shader.manifest.rs")).unwrap();
        assert_eq!(manifest.matches("ShaderEntry {").count(), 3);
        assert!(manifest.contains("Blend :: Add") && manifest.contains("Primitive :: Triangle"));
        assert!(manifest.contains("Blend :: Replace"));
    }

    #[test]
    fn follows_aliased_helpers_without_importing_cpu_code() {
        let generated = extract(
            "
            use unavailable_cpu_crate::Cpu;
            use math::weight as amount;
            mod math {
                const SCALE: f32 = 2.0;
                pub fn weight() -> f32 { SCALE }
                fn unused() -> Cpu { panic!() }
            }
            fn draw() {
                frame.paint(quad, isthmus::shader!(|fragment: Fragment| Vec4::splat(amount())));
            }
        ",
        );
        assert!(generated.contains("fn weight"));
        assert!(generated.contains("const SCALE"));
        assert!(generated.contains("use math::weight as amount"));
        assert!(!generated.contains("unavailable_cpu_crate"));
        assert!(!generated.contains("fn unused"));
        assert!(!generated.contains("fn draw"));
    }

    #[test]
    fn resolves_block_loop_and_nested_function_scopes() {
        let generated = extract(
            "
            use unavailable_cpu_crate::local;
            use math::weight;
            use math::weight as inner_weight;
            mod math { pub fn weight() -> f32 { 2.0 } }
            fn helper() -> f32 {
                let local = 1.0;
                { let weight = 3.0; let _ = weight; }
                while let Some(weight) = None::<f32> { let _ = weight; }
                let inner_weight = 1.0;
                fn inner() -> f32 { inner_weight() }
                weight() + local + inner() + inner_weight
            }
            fn draw() {
                frame.paint(quad, isthmus::shader!(|fragment: Fragment| Vec4::splat(helper())));
            }
        ",
        );
        assert!(generated.contains("fn weight"));
        assert!(generated.contains("weight as inner_weight"));
        assert!(!generated.contains("unavailable_cpu_crate"));
    }

    #[test]
    fn follows_glob_imports_without_copying_unrelated_items() {
        let generated = extract(
            "
            use math::*;
            mod math {
                pub const USED: f32 = 1.0;
                pub const UNUSED: f32 = 2.0;
            }
            fn draw() {
                frame.paint(quad, isthmus::shader!(|fragment: Fragment| Vec4::splat(USED)));
            }
        ",
        );
        assert!(generated.contains("const USED"));
        assert!(!generated.contains("const UNUSED"));
    }
}
