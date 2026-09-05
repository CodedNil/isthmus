use super::{flatten, item_name, shader_derives};
use std::{collections::HashSet, mem};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
};

#[derive(Default)]
pub(super) struct References {
    pub(super) paths: Vec<syn::Path>,
    pub(super) methods: HashSet<String>,
    locals: Vec<HashSet<String>>,
    pub(super) declarations: Vec<(syn::Macro, proc_macro2::LineColumn, HashSet<String>)>,
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
        if let Some(paths) = shader_derives(i) {
            self.paths.extend(paths);
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

pub(super) struct DefaultFields(pub bool);
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
