use super::{image_name, shader_entry};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{collections::BTreeSet, mem};
use syn::{
    Expr, Ident, Lit, Pat, Token, Type,
    parse::Parser,
    parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    visit_mut::{self, VisitMut},
};

struct Binding {
    name: Ident,
    ty: Type,
    kind: CaptureKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Data,
    Image,
    Slice,
}

impl Binding {
    fn parse(pat: Pat) -> syn::Result<Self> {
        if let Pat::Type(pat) = &pat
            && let Pat::Ident(name) = &*pat.pat
            && name.mutability.is_none()
            && name.by_ref.is_none()
            && name.subpat.is_none()
        {
            if let Type::Reference(reference) = &*pat.ty
                && reference.mutability.is_some()
            {
                return Err(syn::Error::new_spanned(&pat.ty, "shader captures must be immutable"));
            }
            let name = name.ident.clone();
            let image = |ty: &Type| matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Image"));
            let kind = match &*pat.ty {
                Type::Reference(reference) if matches!(&*reference.elem, Type::Slice(_)) => CaptureKind::Slice,
                Type::Reference(reference) if image(&reference.elem) => CaptureKind::Image,
                ty if image(ty) => CaptureKind::Image,
                _ => CaptureKind::Data,
            };
            return Ok(Self { name, ty: (*pat.ty).clone(), kind });
        }
        Err(syn::Error::new_spanned(pat, "expected a typed capture: name: Type"))
    }
}

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

    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let (frame, fragment, blend, name) = (&self.frame, &self.fragment, &self.blend, &self.name);
        let variant = &blend.segments.last().unwrap().ident;
        let bindings = self.captures.iter().map(|Binding { name, ty, .. }| quote!(let #name: #ty = #name;));
        let views = self.captures.iter().filter(|capture| capture.kind == CaptureKind::Data).map(|capture| {
            let name = &capture.name;
            quote!(let #name = #isthmus::ShaderData::resolve(
                #name, #isthmus::Resources::data(&*__isthmus_frame.resources)
            );)
        });
        let fields = self.captures.iter().filter(|capture| capture.kind != CaptureKind::Image).map(|capture| {
            let name = &capture.name;
            if capture.kind == CaptureKind::Slice {
                quote!(#name: __isthmus_frame.capture_slice(#name))
            } else {
                quote!(#name)
            }
        });
        let images = self.images();

        let payload = self.payload(isthmus, &format_ident!("__IsthmusPayload"));
        let stage = self.stage(isthmus);
        quote!({
            const _: () = assert!(matches!(#blend, #isthmus::Blend::#variant));
            #(#bindings)*
            let __isthmus_frame: &mut #isthmus::Frame<'_, Program> = (#frame).reborrow();
            let __vertices = {
                #(#views)*
                __isthmus_frame.prepare(#stage, #fragment)
            };
            #payload
            const __SHADER: usize = #isthmus::__private::shader_index(<Program as #isthmus::Program>::SHADERS, #name);
            if __vertices >= 3 {
                let __payload = __IsthmusPayload { #(#fields),* };
                let __images: &[&#isthmus::Image] = &[#(&#images),*];
                __isthmus_frame.record(__SHADER, __vertices, __payload, __images);
            }
        })
    }

    pub fn gpu(&self, isthmus: &TokenStream) -> TokenStream {
        let suffix = self.entry.value().replace('_', "");
        let payload_name = format_ident!("IsthmusPayload{suffix}");
        let stage = self.stage(isthmus);
        let payload = self.payload(isthmus, &payload_name);
        let frame_input = self.fragment.inputs.first().unwrap();
        let input = &self.fragment.inputs[1];
        let mut body = (*self.fragment.body).clone();
        let mut stage = stage;
        let slices = self
            .captures
            .iter()
            .filter(|capture| capture.kind == CaptureKind::Slice)
            .map(|capture| capture.name.to_string())
            .collect::<BTreeSet<_>>();
        SliceIndices(slices.clone()).visit_expr_mut(&mut body);
        SliceIndices(slices).visit_expr_mut(&mut stage);
        let bindings = self.captures.iter().map(|Binding { name, ty, kind }| {
            let value = match kind {
                CaptureKind::Image => {
                    let image = image_name(name);
                    let value = quote!(#isthmus::Image::new(#image, *__isthmus_sampler));
                    if matches!(ty, Type::Reference(_)) { quote!(&#value) } else { value }
                }
                CaptureKind::Slice => quote!(crate::ShaderSlice::from_words(__payload, __instance.#name)),
                CaptureKind::Data => quote!(#isthmus::ShaderData::resolve(__instance.#name, __resources)),
            };
            let annotation = (*kind == CaptureKind::Slice).then(|| {
                let Type::Reference(reference) = ty else { return quote!() };
                let Type::Slice(slice) = &*reference.elem else { return quote!() };
                let element = &slice.elem;
                quote!(: crate::ShaderSlice<'_, #element>)
            });
            // Captures can be used by only one stage; both stages share this setup.
            quote!(let #name #annotation = #value; let _ = &#name;)
        });
        let setup = quote! {
            type __Program = Program;
            #(#bindings)*
            let __frame = __frame_data;
            let __stage = (#stage)(__frame);
        };
        let images = self.images().collect::<Vec<_>>();
        let vertex_name = syn::LitStr::new(&self.vertex_entry(), self.entry.span());
        let vertex = shader_entry(isthmus, &vertex_name, true, &images, &payload_name, &quote! {
            #setup
            let __vertex = #isthmus::Primitive::vertex(__stage, #isthmus::VertexInput::<__Program> {
                index: __index, frame: __frame,
            });
            *__out_position = __vertex.position;
            *__out_uv = __vertex.uv;
            *__out_draw_index = __draw_index;
        });
        let output = if self.blend.segments.last().is_some_and(|segment| segment.ident == "Raw") {
            quote! { *__out_color = __color; }
        } else {
            quote! {
                let __alpha = __color.w * __coverage;
                *__out_color = (__color.truncate() * __alpha).extend(__alpha);
            }
        };
        let fragment = shader_entry(isthmus, &self.entry, false, &images, &payload_name, &quote! {
            #setup
            let __fragment = #isthmus::Fragment {
                pixel: #isthmus::glam::vec2(__pixel.x, __pixel.y) * __frame_data.pixel_scale,
                uv: __uv,
                sample: (),
                coverage: 1.0,
            };
            let (__sample, __coverage) = #isthmus::Primitive::<__Program>::sample(__stage, __fragment);
            let #input = #isthmus::Fragment {
                pixel: __fragment.pixel, uv: __fragment.uv, sample: __sample, coverage: __coverage,
            };
            let #frame_input = __frame;
            let __color: #isthmus::glam::Vec4 = (|| #body)();
            if __coverage <= 0.0 { #isthmus::spirv_std::arch::kill(); }
            #output
        });
        quote!(#payload #vertex #fragment)
    }
}

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
