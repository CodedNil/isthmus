//! Recognizes a shader draw in source: `frame.shader("name").with((a, b)).shape(|frame| ..).paint(|frame, fragment| ..)`.
use syn::{Expr, ExprClosure, ExprMethodCall, Ident, Lit, Stmt, parse_quote};

/// A shader draw as written: its name, the variables it sends, its closures and its pipeline options.
pub struct Chain {
    pub name: String,
    /// The variables listed in `.with(..)`, in order.
    pub with: Vec<Ident>,
    pub shape: ExprClosure,
    pub paint: ExprClosure,
    pub blend: Ident,
    pub format: Option<Ident>,
}

/// The shader painted by `statement`, if that statement is a complete shader draw.
pub fn parse(statement: &Stmt) -> syn::Result<Option<Chain>> {
    let Stmt::Expr(expr, _) = statement else { return Ok(None) };
    let Some(paint) = method(expr, "paint") else { return Ok(None) };
    let Some(geometry) = method(&paint.receiver, "shape").or_else(|| method(&paint.receiver, "bounds")) else {
        return Ok(None);
    };
    if geometry.args.len() != 1 || paint.args.len() != 1 {
        return Ok(None);
    }
    let (Some(geometry_arg), Some(Expr::Closure(paint_closure))) = (geometry.args.first(), paint.args.first()) else {
        return Ok(None);
    };
    let shape = if geometry.method == "bounds" {
        parse_quote!(|_| ())
    } else if let Expr::Closure(closure) = geometry_arg {
        closure.clone()
    } else {
        return Ok(None);
    };
    let Some((name, with, blend, format)) = start(&geometry.receiver)? else { return Ok(None) };
    Ok(Some(Chain { name, with, shape, paint: paint_closure.clone(), blend, format }))
}

fn method<'expr>(expr: &'expr Expr, name: &str) -> Option<&'expr ExprMethodCall> {
    match expr {
        Expr::MethodCall(call) if call.method == name => Some(call),
        _ => None,
    }
}

/// The name, sent variables and pipeline options of `frame.shader("name").with(..).blend(..).format(..)`.
#[expect(clippy::type_complexity, reason = "a private helper returning the pieces of one chain")]
fn start(mut expr: &Expr) -> syn::Result<Option<(String, Vec<Ident>, Ident, Option<Ident>)>> {
    let (mut with, mut blend, mut format) = (None, None, None);
    let last = |expr: &Expr| match expr {
        Expr::Path(path) => path.path.segments.last().map(|segment| segment.ident.clone()),
        _ => None,
    };
    while let Expr::MethodCall(call) = expr {
        match (call.method.to_string().as_str(), call.args.first()) {
            ("shader", Some(Expr::Lit(literal))) if let Lit::Str(name) = &literal.lit => {
                let blend = blend.unwrap_or_else(|| Ident::new("Over", call.method.span()));
                return Ok(Some((name.value(), with.unwrap_or_default(), blend, format)));
            }
            ("shader", _) => {
                return Err(syn::Error::new_spanned(
                    call,
                    "a shader's name must be a string literal: `frame.shader(\"name\")`",
                ));
            }
            ("with", Some(values)) => with = Some(names(values)?),
            ("blend" | "format", Some(value)) => {
                let variant = last(value)
                    .ok_or_else(|| syn::Error::new_spanned(value, "write the variant out, such as `Blend::Add`"))?;
                if call.method == "blend" { blend = Some(variant) } else { format = Some(variant) }
            }
            _ => return Ok(None),
        }
        expr = &call.receiver;
    }
    Ok(None)
}

/// The variable names in `.with(a)` or `.with((a, b))`.
fn names(values: &Expr) -> syn::Result<Vec<Ident>> {
    let name = |expr: &Expr| match expr {
        Expr::Path(path) if let Some(name) = path.path.get_ident() => Ok(name.clone()),
        other => Err(syn::Error::new_spanned(
            other,
            "`with` lists plain variable names; bind other values to a typed `let` first",
        )),
    };
    match values {
        Expr::Tuple(tuple) => tuple.elems.iter().map(name).collect(),
        Expr::Paren(paren) => Ok(vec![name(&paren.expr)?]),
        other => Ok(vec![name(other)?]),
    }
}
