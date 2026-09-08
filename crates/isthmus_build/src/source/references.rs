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
    locals: Vec<String>,
    pub(super) declarations: Vec<(syn::Macro, proc_macro2::LineColumn, HashSet<String>)>,
}

impl References {
    fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
        let scope = self.locals.len();
        visit(self);
        self.locals.truncate(scope);
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
            self.declarations.push((i.clone(), i.path.span().start(), self.locals.iter().cloned().collect()));
        }
    }

    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}

    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if i.path.get_ident().is_none_or(|name| !self.locals.contains(&name.to_string())) {
            self.visit_path(&i.path);
        } else {
            for segment in &i.path.segments {
                self.visit_path_arguments(&segment.arguments);
            }
        }
        if i.path.segments.len() > 1 {
            self.methods.insert(i.path.segments.last().expect("path has segments").ident.to_string());
        }
        if let Some(qself) = &i.qself {
            self.visit_qself(qself);
        }
    }

    fn visit_path(&mut self, i: &'ast syn::Path) {
        self.paths.push(i.clone());
        visit::visit_path(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        self.methods.insert(i.method.to_string());
        visit::visit_expr_method_call(self, i);
    }

    fn visit_block(&mut self, i: &'ast syn::Block) {
        let scope = self.locals.len();
        for statement in &i.stmts {
            if let syn::Stmt::Item(item) = statement
                && let Some(name) = item_name(item)
            {
                self.locals.push(name);
            }
        }
        visit::visit_block(self, i);
        self.locals.truncate(scope);
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

    fn visit_local(&mut self, i: &'ast syn::Local) {
        if let Some(init) = &i.init {
            self.visit_local_init(init);
        }
        self.visit_pat(&i.pat);
    }

    fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
        self.locals.push(i.ident.to_string());
        visit::visit_pat_ident(self, i);
    }

    fn visit_expr_closure(&mut self, i: &'ast syn::ExprClosure) {
        self.scoped(|this| visit::visit_expr_closure(this, i));
    }

    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        self.visit_expr(&i.expr);
        let scope = self.locals.len();
        self.visit_pat(&i.pat);
        self.visit_block(&i.body);
        self.locals.truncate(scope);
    }

    fn visit_arm(&mut self, i: &'ast syn::Arm) {
        self.scoped(|this| visit::visit_arm(this, i));
    }

    fn visit_expr_let(&mut self, i: &'ast syn::ExprLet) {
        self.visit_expr(&i.expr);
        self.visit_pat(&i.pat);
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        let scope = self.locals.len();
        self.visit_expr(&i.cond);
        self.visit_block(&i.then_branch);
        self.locals.truncate(scope);
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
        self.scoped(|this| visit::visit_expr_while(this, i));
    }
}
