use super::{InputKind, fragment_entry};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Ident, Pat, PatType, Type, parse::Parser, parse_quote, punctuated::Punctuated};

#[derive(Clone)]
pub struct Capture {
    pub name: Ident,
    source: Type,
    kind: CaptureKind,
    mutability: Option<syn::Token![mut]>,
}

#[derive(Clone, Copy)]
enum CaptureKind {
    Plain,
    Bool,
    Image,
}

impl Capture {
    pub fn new(input: &PatType) -> syn::Result<Self> {
        let Pat::Ident(name) = input.pat.as_ref() else {
            return Err(syn::Error::new_spanned(&input.pat, "shader inputs require identifiers"));
        };
        if name.by_ref.is_some() || name.subpat.is_some() {
            return Err(syn::Error::new_spanned(name, "shader inputs require a plain identifier, optionally mut"));
        }
        let source = input.ty.as_ref().clone();
        let kind = if matches!(&source, Type::Path(path) if path.path.is_ident("bool")) {
            CaptureKind::Bool
        } else if matches!(&source, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Image"))
        {
            CaptureKind::Image
        } else {
            CaptureKind::Plain
        };
        Ok(Self { name: name.ident.clone(), source, kind, mutability: name.mutability })
    }

    pub const fn is_image(&self) -> bool {
        matches!(self.kind, CaptureKind::Image)
    }

    fn storage(&self) -> Type {
        match self.kind {
            CaptureKind::Bool => parse_quote!(u32),
            CaptureKind::Plain | CaptureKind::Image => self.source.clone(),
        }
    }

    fn argument(&self, isthmus: &TokenStream) -> TokenStream {
        let name = &self.name;
        match self.kind {
            CaptureKind::Plain => quote!(instance.#name),
            CaptureKind::Bool => quote!(instance.#name != 0),
            CaptureKind::Image => quote!(#isthmus::__private::ShaderImage::new(__isthmus_image, *__isthmus_sampler)),
        }
    }

    pub fn encode(&self, value: &TokenStream) -> TokenStream {
        match self.kind {
            CaptureKind::Plain | CaptureKind::Image => quote!(#value),
            CaptureKind::Bool => quote!(u32::from(#value)),
        }
    }
}

pub struct Shader {
    pub declaration: syn::ExprClosure,
    input: Capture,
    captures: Vec<Capture>,
    entry: syn::LitStr,
    kind: InputKind,
    blend: Ident,
    options: Option<syn::Path>,
}

impl Shader {
    pub fn parse(tokens: TokenStream, file: &str, line: usize, column: usize) -> syn::Result<Self> {
        let args = Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated.parse2(tokens)?;
        let mut args = args.into_iter();
        let first =
            args.next().ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "expected a shader closure"))?;
        let (options, expression) = if let syn::Expr::Path(path) = first {
            let expression = args
                .next()
                .ok_or_else(|| syn::Error::new_spanned(&path, "expected a shader closure after blend mode"))?;
            (Some(path.path), expression)
        } else {
            (None, first)
        };
        if args.next().is_some() {
            return Err(syn::Error::new_spanned(expression, "expected a blend mode and one shader closure"));
        }
        let blend = options
            .as_ref()
            .and_then(|path| path.segments.last())
            .map_or_else(|| format_ident!("Over"), |segment| segment.ident.clone());
        if !matches!(blend.to_string().as_str(), "Over" | "Add" | "Replace") {
            return Err(syn::Error::new_spanned(blend, "expected Blend::Over, Blend::Add or Blend::Replace"));
        }
        let syn::Expr::Closure(closure) = expression else {
            return Err(syn::Error::new_spanned(expression, "expected a shader closure"));
        };
        closure.modifiers.require_empty()?;
        if closure.asyncness.is_some() || closure.constness.is_some() || closure.capture.is_some() {
            return Err(syn::Error::new_spanned(
                closure,
                "shader declarations cannot be async, static, or move closures",
            ));
        }
        let mut inputs = closure.inputs.iter().map(|input| match input {
            Pat::Type(input) if matches!(&*input.pat, Pat::Ident(_)) => Ok(input.clone()),
            _ => Err(syn::Error::new_spanned(input, "shader inputs require a name and explicit type")),
        });
        let input =
            inputs.next().transpose()?.ok_or_else(|| syn::Error::new_spanned(&closure, "expected a fragment input"))?;
        let text = matches!(&*input.ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "TextFragment"));
        let triangle = matches!(&*input.ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "TriangleFragment"));
        let captures = inputs.map(|input| Capture::new(&input?)).collect::<syn::Result<Vec<_>>>()?;
        if captures.iter().filter(|capture| capture.is_image()).count() > 1 {
            return Err(syn::Error::new_spanned(
                &closure,
                "a shader may capture one Image; multiple textures are not supported yet",
            ));
        }
        let file = file.replace('\\', "/");
        let file =
            file.rsplit_once("/src/").map_or_else(|| file.strip_prefix("src/").unwrap_or(&file), |(_, suffix)| suffix);
        let hash = file
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3));
        let entry = syn::LitStr::new(&format!("isthmus_{hash:x}_{line}_{column}"), input.colon_token.span);
        let input = Capture::new(&input)?;
        let kind = if text {
            InputKind::Text
        } else if triangle {
            InputKind::Triangle
        } else {
            InputKind::Quad
        };
        Ok(Self { declaration: closure, input, captures, entry, kind, blend, options })
    }

    pub fn metadata(&self, web: bool) -> TokenStream {
        let name = if web { format!("{}_", self.entry.value()) } else { self.entry.value() };
        let blend = &self.blend;
        let primitive = if self.is_triangle() { format_ident!("Triangle") } else { format_ident!("Quad") };
        quote!(ShaderEntry {
            name: #name,
            blend: Blend::#blend,
            primitive: Primitive::#primitive,
        })
    }

    pub fn entry(&self) -> String {
        self.entry.value()
    }

    pub fn is_triangle(&self) -> bool {
        self.kind == InputKind::Triangle
    }

    fn payload(&self, isthmus: &TokenStream) -> TokenStream {
        let line = (self.kind == InputKind::Text).then(|| quote!(__isthmus_line: #isthmus::geometry::text::Line,));
        let fields = self.captures.iter().filter(|capture| !capture.is_image()).map(|capture| {
            let name = &capture.name;
            let storage = capture.storage();
            quote!(#name: #storage)
        });
        quote! {
            #[repr(C)]
            #[derive(Clone, Copy, #isthmus::ShaderData)]
            struct __IsthmusPayload { #line #(#fields),* }
        }
    }

    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let declaration = &self.declaration;
        let input_type = &self.input.source;
        let mut interface_type = input_type.clone();
        if (self.kind == InputKind::Text)
            && let Type::Path(path) = &mut interface_type
        {
            let segment = path.path.segments.last_mut().expect("shader input has a type name");
            if matches!(segment.arguments, syn::PathArguments::None) {
                segment.arguments = syn::PathArguments::AngleBracketed(parse_quote!(<'static>));
            } else if let syn::PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                if let Some(syn::GenericArgument::Lifetime(lifetime)) = arguments.args.first_mut() {
                    *lifetime = parse_quote!('static);
                } else {
                    arguments.args.insert(0, parse_quote!('static));
                }
            }
        }
        let types = self.captures.iter().map(|capture| &capture.source);
        let payload = self.payload(isthmus);
        let program = quote!(<#interface_type as #isthmus::__private::ShaderInput>::Program);
        let geometry = quote!(<#interface_type as #isthmus::__private::ShaderInput>::Geometry);
        let entry = &self.entry;
        let web_entry = syn::LitStr::new(&format!("{}_", entry.value()), entry.span());
        let bindings =
            self.captures.iter().filter(|capture| !matches!(capture.kind, CaptureKind::Plain)).map(|capture| {
                let name = &capture.name;
                let ty = &capture.source;
                if capture.is_image() { quote!(let #name: &#ty = &#name;) } else { quote!(let #name: #ty = #name;) }
            });
        let fields = self.captures.iter().filter(|capture| !capture.is_image()).map(|capture| {
            let name = &capture.name;
            let value = capture.encode(&quote!(#name));
            quote!(#name: #value)
        });
        let line = (self.kind == InputKind::Text).then(|| quote!(__isthmus_line: __isthmus_geometry,));
        let blend = &self.blend;
        let options = self.options.as_ref().map(|options| quote! {
            const _: () = assert!(matches!(#options, #isthmus::Blend::#blend), "blend mode differs from generated pipeline");
        });
        let image = self.captures.iter().find(|capture| capture.is_image()).map_or_else(
            || quote!(None),
            |capture| {
                let name = &capture.name;
                quote!(Some(__isthmus_frame.__image(#name)))
            },
        );
        quote!({
            #options
            let _: fn(#input_type, #(#types),*) -> #isthmus::glam::Vec4 = #declaration;
            #payload
            // SAFETY: Both interfaces are generated from this declaration and its checked entry.
            unsafe impl #isthmus::__private::ShaderSpec for __IsthmusPayload {
                type Program = #program;
                type Geometry = #geometry;
                const INDEX: usize = #isthmus::__private::shader_index(
                    <#program as #isthmus::Program>::SHADERS,
                    if cfg!(target_arch = "wasm32") { #web_entry } else { #entry },
                );
            }
            const _: usize = <__IsthmusPayload as #isthmus::__private::ShaderSpec>::INDEX;
            #(#bindings)*
            move |__isthmus_frame: &mut #isthmus::Frame<'_, #program>, __isthmus_geometry| {
                (__IsthmusPayload { #line #(#fields),* }, #image)
            }
        })
    }

    pub fn gpu(&self, isthmus: &TokenStream) -> TokenStream {
        let payload = self.payload(isthmus);
        let name = format_ident!("{}", self.entry.value());
        let input_name = &self.input.name;
        let input_type = &self.input.source;
        let mutable = self.input.mutability;
        let input_binding = quote!(#mutable #input_name);
        let has_payload = (self.kind == InputKind::Text) || self.captures.iter().any(|capture| !capture.is_image());
        let load = has_payload.then(|| {
            quote! {
                // SAFETY: CPU and GPU payload layouts come from the same shader declaration.
                let instance = unsafe { #isthmus::__private::load::<__IsthmusPayload>(payload, draw.payload) };
            }
        });
        let text_input = (self.kind == InputKind::Text).then(|| {
            quote! {
                let #input_name: #input_type = #isthmus::TextFragment::new(
                    fragment, instance.__isthmus_line, placed_glyphs, glyphs, curves,
                );
            }
        });
        let body = match &*self.declaration.body {
            syn::Expr::Block(block) => block.block.to_token_stream(),
            expression => quote!({ #expression }),
        };
        let parameters = self.captures.iter().map(|capture| {
            let name = &capture.name;
            let mutable = capture.mutability;
            let ty = if capture.is_image() {
                quote!(#isthmus::__private::ShaderImage<'_>)
            } else {
                capture.source.to_token_stream()
            };
            quote!(#mutable #name: #ty)
        });
        let arguments = self.captures.iter().map(|capture| capture.argument(isthmus));
        let entry = fragment_entry(
            isthmus,
            &self.entry,
            self.kind,
            has_payload,
            self.captures.iter().any(Capture::is_image),
            input_type,
            &input_name.to_token_stream(),
            &quote! {
                #load
                #text_input
                let color = shade(#input_name, #(#arguments),*);
                *out_color = color.truncate().extend(1.0) * color.w;
            },
        );
        quote! {
            pub mod #name {
                use super::*;
                #payload
                #[inline(always)]
                fn shade(#input_binding: #input_type, #(#parameters),*) -> #isthmus::glam::Vec4 #body
                #entry
            }
        }
    }
}
