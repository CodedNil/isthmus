//! Rewrites slice captures into `ShaderSlice` accesses that cannot panic in a shader.
use std::collections::BTreeSet;
use syn::{
    Expr, parse_quote,
    visit_mut::{self, VisitMut},
};

/// Rewrites `for x in slice` and `slice[i]` over the named captures.
pub struct SliceIndices(pub BTreeSet<String>);

impl VisitMut for SliceIndices {
    fn visit_expr_mut(&mut self, i: &mut Expr) {
        visit_mut::visit_expr_mut(self, i);
        if let Expr::ForLoop(loop_) = i
            && let Expr::Path(path) = &*loop_.expr
            && let Some(name) = path.path.get_ident()
            && self.0.contains(&name.to_string())
        {
            let binding = &loop_.pat;
            let body = &loop_.body;
            *i = parse_quote!({
                for __isthmus_index in 0..#name.len() {
                    let __isthmus_item = #name.load(__isthmus_index);
                    let #binding = &__isthmus_item;
                    #body
                }
            });
            return;
        }
        if let Expr::Index(index) = i
            && let Expr::Path(path) = &*index.expr
            && let Some(name) = path.path.get_ident()
            && self.0.contains(&name.to_string())
        {
            let receiver = (*index.expr).clone();
            let subscript = (*index.index).clone();
            *i = parse_quote!(#receiver.load(#subscript));
        }
    }
}
