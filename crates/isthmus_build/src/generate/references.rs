use super::items::{flatten, item_name, shader_derives};
use isthmus_syntax::shader::{self, Chain};
use std::{collections::HashSet, mem};
use syn::{
    spanned::Spanned as _,
    visit::{self, Visit},
};

/// A variable in scope, with the type its `let` or parameter declares, if it does.
pub(super) type Local = (String, Option<syn::Type>);

/// A shader draw found in the source, with the variables in scope where it sits.
pub(super) struct Draw {
    pub(super) chain: Chain,
    pub(super) position: (usize, usize),
    pub(super) locals: Vec<Local>,
}

#[derive(Default)]
pub(super) struct References {
    pub(super) paths: Vec<syn::Path>,
    pub(super) methods: HashSet<String>,
    locals: Vec<Local>,
    pub(super) draws: Vec<Draw>,
    /// Shader draws that were recognized but are malformed, with the line they start on.
    pub(super) errors: Vec<((usize, usize), String)>,
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

    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {}

    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if i.path.get_ident().is_none_or(|name| self.locals.iter().all(|(local, _)| name != local.as_str())) {
            self.visit_path(&i.path);
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
        self.scoped(|this| {
            this.locals.extend(i.stmts.iter().filter_map(|statement| match statement {
                syn::Stmt::Item(item) => item_name(item).map(|name| (name, None)),
                _ => None,
            }));
            for statement in &i.stmts {
                match shader::parse(statement) {
                    Ok(Some(chain)) => this.draws.push(Draw {
                        chain,
                        position: position(statement.span().start()),
                        locals: this.locals.clone(),
                    }),
                    Ok(None) => {}
                    Err(error) => this.errors.push((position(error.span().start()), error.to_string())),
                }
                this.visit_stmt(statement);
            }
        });
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
        match (&i.pat, &i.init) {
            (syn::Pat::Ident(name), Some(init)) => {
                let ty = self.infer(&init.expr);
                self.locals.push((name.ident.to_string(), ty));
            }
            _ => self.visit_pat(&i.pat),
        }
    }

    fn visit_pat_type(&mut self, i: &'ast syn::PatType) {
        if self.bind_typed(&i.pat, &i.ty) {
            self.visit_type(&i.ty);
        } else {
            visit::visit_pat_type(self, i);
        }
    }

    fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
        self.locals.push((i.ident.to_string(), None));
        visit::visit_pat_ident(self, i);
    }

    fn visit_expr_closure(&mut self, i: &'ast syn::ExprClosure) {
        self.scoped(|this| visit::visit_expr_closure(this, i));
    }

    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        self.visit_expr(&i.expr);
        self.scoped(|this| {
            this.visit_pat(&i.pat);
            this.visit_block(&i.body);
        });
    }

    fn visit_arm(&mut self, i: &'ast syn::Arm) {
        self.scoped(|this| visit::visit_arm(this, i));
    }

    fn visit_expr_let(&mut self, i: &'ast syn::ExprLet) {
        self.visit_expr(&i.expr);
        self.visit_pat(&i.pat);
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        self.scoped(|this| {
            this.visit_expr(&i.cond);
            this.visit_block(&i.then_branch);
        });
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

impl References {
    /// The type an initializer provably has, without an annotation: a cast, a suffixed literal, a struct literal,
    /// or a typed local or reference to one. Anything else needs the `let` to say.
    fn infer(&self, expr: &syn::Expr) -> Option<syn::Type> {
        use syn::{Expr, Lit};
        match expr {
            Expr::Cast(cast) => Some((*cast.ty).clone()),
            Expr::Paren(inner) => self.infer(&inner.expr),
            Expr::Group(inner) => self.infer(&inner.expr),
            Expr::Lit(literal) => {
                let suffix = match &literal.lit {
                    Lit::Bool(_) => "bool",
                    Lit::Int(int) => int.suffix(),
                    Lit::Float(float) => float.suffix(),
                    _ => return None,
                };
                if suffix.is_empty() { None } else { syn::parse_str(suffix).ok() }
            }
            Expr::Path(path) => {
                let name = path.path.get_ident()?.to_string();
                self.locals.iter().rev().find(|(local, _)| *local == name)?.1.clone()
            }
            Expr::Reference(reference) if reference.mutability.is_none() => {
                let ty = self.infer(&reference.expr)?;
                Some(syn::parse_quote!(&#ty))
            }
            Expr::Struct(literal) if literal.qself.is_none() && !literal.path.is_ident("Self") => {
                let path = &literal.path;
                Some(syn::parse_quote!(#path))
            }
            _ => None,
        }
    }

    /// Declares the variables of a pattern with the types its annotation gives them, for a plain name or a
    /// tuple of names and a tuple type. Returns whether the pattern had that form.
    fn bind_typed(&mut self, pattern: &syn::Pat, ty: &syn::Type) -> bool {
        match (pattern, ty) {
            (syn::Pat::Ident(name), _) => {
                self.locals.push((name.ident.to_string(), Some(ty.clone())));
                true
            }
            (syn::Pat::Tuple(names), syn::Type::Tuple(types)) if names.elems.len() == types.elems.len() => {
                names.elems.iter().zip(&types.elems).all(|(name, ty)| self.bind_typed(name, ty))
            }
            _ => false,
        }
    }
}

/// A source position as a one-based line and column.
const fn position(start: proc_macro2::LineColumn) -> (usize, usize) {
    (start.line, start.column + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens as _;

    #[test]
    fn types_are_inferred_only_when_provable() {
        let mut references = References::default();
        references.locals.push(("known".into(), Some(syn::parse_quote!(f32))));
        references.locals.push(("unknown".into(), None));
        let infer = |expr: &str| {
            references.infer(&syn::parse_str(expr).expect("an expression")).map(|ty| ty.to_token_stream().to_string())
        };
        assert_eq!(infer("count as u32").as_deref(), Some("u32"));
        assert_eq!(infer("(1.5 as f32)").as_deref(), Some("f32"));
        assert_eq!(infer("2.0f32").as_deref(), Some("f32"));
        assert_eq!(infer("true").as_deref(), Some("bool"));
        assert_eq!(infer("known").as_deref(), Some("f32"));
        assert_eq!(infer("&known").as_deref(), Some("& f32"));
        assert_eq!(infer("Rect { min, max }").as_deref(), Some("Rect"));
        for unprovable in ["1.0", "7", "unknown", "missing", "&unknown", "make()", "value.field", "known + 1.0"] {
            assert_eq!(infer(unprovable), None, "{unprovable} must not be guessed");
        }
    }
}
