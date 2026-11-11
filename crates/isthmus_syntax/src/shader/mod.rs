//! A `shader!` declaration: its parsed captures, stages and options.
use capture::{Binding, CaptureKind};
use proc_macro2::TokenStream;
use quote::quote;
pub use slices::SliceIndices;
use std::mem;
use syn::{
    Expr, Ident, Lit, Pat, Token, parse::Parser as _, parse_quote, punctuated::Punctuated, spanned::Spanned as _,
};

mod capture;
mod gpu;
mod host;
mod slices;

pub struct Shader {
    pub declaration: syn::ExprClosure,
    frame: Expr,
    stage: Expr,
    fragment: syn::ExprClosure,
    captures: Vec<Binding>,
    /// The name given in the `shader!` call, which labels the shader on the GPU and in logs.
    pub name: syn::LitStr,
    /// The unique identifier of its fragment entry point, derived from the name.
    pub entry: syn::LitStr,
    blend: syn::Path,
    format: Option<syn::Path>,
}

fn argument(expr: &mut Expr, name: &str) -> syn::Result<Option<Expr>> {
    if let Expr::MethodCall(call) = expr
        && call.method == name
    {
        if call.args.len() != 1 || call.turbofish.is_some() {
            return Err(syn::Error::new_spanned(call, "expected one argument"));
        }
        let value = call.args.pop().unwrap();
        *expr = *call.receiver.clone();
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

fn closure(expr: &mut Expr, parameters: usize) -> syn::Result<&mut syn::ExprClosure> {
    let span = expr.span();
    match expr {
        Expr::Closure(closure)
            if closure.asyncness.is_none()
                && closure.constness.is_none()
                && closure.inputs.len() == parameters
                && closure.inputs.iter().all(|pat| !matches!(pat, Pat::Type(_))) =>
        {
            Ok(closure)
        }
        _ => Err(syn::Error::new(span, format!("expected a closure with {parameters} inferred parameters"))),
    }
}

impl Shader {
    pub fn parse(tokens: TokenStream, isthmus: &TokenStream) -> syn::Result<Self> {
        let arguments = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(tokens)?;
        if arguments.len() != 4 {
            return Err(syn::Error::new_spanned(
                arguments,
                "expected shader!(\"name\", frame, |frame, name: Type| primitive, |frame, fragment| color)",
            ));
        }
        let mut arguments = arguments.into_iter();
        let name = match arguments.next().unwrap() {
            Expr::Lit(syn::ExprLit { lit: Lit::Str(name), .. }) if !name.value().trim().is_empty() => name,
            other => return Err(syn::Error::new_spanned(other, "expected a shader name string first")),
        };
        let mut frame = arguments.next().unwrap();
        let mut stage = arguments.next().unwrap();
        let mut fragment = arguments.next().unwrap();
        let fragment = closure(&mut fragment, 2)?.clone();
        let Expr::Closure(factory) = &mut stage else {
            return Err(syn::Error::new_spanned(stage, "expected |frame, name: Type| primitive"));
        };
        let mut inputs = mem::take(&mut factory.inputs).into_iter();
        let input = inputs.next().ok_or_else(|| syn::Error::new_spanned(&*factory, "expected a frame parameter"))?;
        let captures = inputs.map(Binding::parse).collect::<syn::Result<Vec<_>>>()?;
        factory.inputs.push(input);
        closure(&mut stage, 1)?;
        let blend = argument(&mut frame, "blend")?.unwrap_or_else(|| parse_quote!(#isthmus::Blend::Over));
        let Expr::Path(blend) = blend else {
            return Err(syn::Error::new_spanned(blend, "expected a Blend variant"));
        };
        let format = argument(&mut frame, "format")?
            .map(|format| match format {
                Expr::Path(path) => Ok(path.path),
                other => Err(syn::Error::new_spanned(other, "expected a TextureFormat variant")),
            })
            .transpose()?;
        if let Expr::MethodCall(call) = &frame
            && ["format", "blend"].iter().any(|name| call.method == *name)
        {
            return Err(syn::Error::new_spanned(call, "frame options must occur once, in format/blend order"));
        }
        let inputs = captures.iter().map(|Binding { name, ty, .. }| quote!(#name: #ty));
        let declaration = parse_quote!(|#(#inputs),*| {
            let _: Program;
            #stage;
            #fragment;
        });
        // Entry points are identifiers, so the name is reduced to letters and digits.
        let identifier: String = name
            .value()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect();
        let entry = syn::LitStr::new(&format!("isthmus_{identifier}_fragment"), name.span());
        Ok(Self { declaration, frame, stage, fragment, captures, name, entry, blend: blend.path, format })
    }

    pub fn vertex_entry(&self) -> String {
        self.entry.value().strip_suffix("_fragment").map_or_else(String::new, |stem| format!("{stem}_vertex"))
    }

    fn images(&self) -> impl Iterator<Item = &Ident> {
        self.captures.iter().filter(|capture| capture.kind == CaptureKind::Image).map(|capture| &capture.name)
    }

    pub fn metadata(&self, isthmus: &TokenStream) -> TokenStream {
        let (name, entry, vertex) = (&self.name, &self.entry, self.vertex_entry());
        let blend = &self.blend.segments.last().unwrap().ident;
        let images = self.images().count();
        let format = self.format.as_ref().map_or_else(
            || quote!(None),
            |format| {
                let variant = &format.segments.last().unwrap().ident;
                quote!({ use #isthmus::wgpu::TextureFormat; Some(TextureFormat::#variant) })
            },
        );
        quote!(ShaderEntry {
            name: #name, blend: Blend::#blend, images: #images, format: #format,
            code: [
                include_str!(concat!(env!("OUT_DIR"), "/", #vertex, ".wgsl")),
                include_str!(concat!(env!("OUT_DIR"), "/", #entry, ".wgsl")),
            ],
        })
    }

    fn payload(&self, isthmus: &TokenStream, name: &Ident) -> TokenStream {
        let fields = self.captures.iter().filter(|capture| capture.kind != CaptureKind::Image).map(|capture| {
            let (name, ty) = (&capture.name, &capture.ty);
            let ty = if capture.kind == CaptureKind::Slice { quote!([u32; 2]) } else { quote!(#ty) };
            quote!(#name: #ty)
        });
        quote! {
            #[derive(Clone, Copy, #isthmus::ShaderData)]
            struct #name { #(#fields),* }
        }
    }

    fn stage(&self, isthmus: &TokenStream) -> Expr {
        let mut stage = self.stage.clone();
        let factory = closure(&mut stage, 1).unwrap();
        let input = factory.inputs.first().unwrap();
        let typed: syn::ExprClosure = parse_quote!(|#input: #isthmus::ShaderFrame<Program>| {});
        factory.inputs = typed.inputs;
        stage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutable_captures_are_rejected_at_the_shader_boundary() {
        for ty in [quote!(&mut Image), quote!(&mut [u32]), quote!(&mut Vec4)] {
            let shader = Shader::parse(quote!("test", frame, |_, value: #ty| rect, |_, _| color), &quote!(::isthmus));
            assert!(shader.is_err_and(|error| error.to_string() == "shader captures must be immutable"));
        }
    }

    #[test]
    fn names_are_required_and_become_entry_identifiers() {
        let parse = |tokens| Shader::parse(tokens, &quote!(::isthmus));
        assert!(parse(quote!(frame, |_, rect: Rect| rect, |_, _| color)).is_err());
        assert!(parse(quote!("  ", frame, |_, rect: Rect| rect, |_, _| color)).is_err());
        let shader = parse(quote!("Track text", frame, |_, rect: Rect| rect, |_, _| color)).unwrap();
        assert_eq!(shader.entry.value(), "isthmus_track_text_fragment");
        assert_eq!(shader.vertex_entry(), "isthmus_track_text_vertex");
        assert_eq!(shader.name.value(), "Track text");
    }
}
