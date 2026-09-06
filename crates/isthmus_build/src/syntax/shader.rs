use super::{fragment_entry, vertex};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Ident, Pat, PatType, Type, parse::Parser, parse_quote, punctuated::Punctuated, visit_mut::VisitMut};

pub struct Capture {
    pub name: Ident,
    source: Type,
    image: bool,
    buffer: bool,
    mutability: Option<syn::Token![mut]>,
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
        let image = matches!(&source, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Image"));
        let buffer = matches!(&source, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Buffer"));
        Ok(Self { name: name.ident.clone(), source, image, buffer, mutability: name.mutability })
    }

    fn argument(&self, isthmus: &TokenStream) -> TokenStream {
        let name = &self.name;
        if self.image {
            let image = format_ident!("__isthmus_image_{name}");
            let sampler = format_ident!("__isthmus_sampler_{name}");
            quote!(#isthmus::__private::ShaderImage::new(#image, *#sampler))
        } else if self.buffer {
            quote!(#isthmus::Buffer::from_words(payload, instance.#name))
        } else {
            quote!(instance.#name)
        }
    }
}

pub struct Shader {
    pub declaration: syn::ExprClosure,
    input: Capture,
    captures: Vec<Capture>,
    entry: syn::LitStr,
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
        let captures = inputs.map(|input| Capture::new(&input?)).collect::<syn::Result<Vec<_>>>()?;
        let file = file.replace('\\', "/");
        let file =
            file.rsplit_once("/src/").map_or_else(|| file.strip_prefix("src/").unwrap_or(&file), |(_, suffix)| suffix);
        let hash = file
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3));
        let entry = syn::LitStr::new(&format!("isthmus_{hash:x}_{line}_{column}_fragment"), input.colon_token.span);
        let input = Capture::new(&input)?;
        Ok(Self { declaration: closure, input, captures, entry, blend, options })
    }

    pub fn metadata(&self) -> TokenStream {
        let name = &self.entry;
        let blend = &self.blend;
        let vertex = self.vertex_entry();
        let images = self.captures.iter().filter(|capture| capture.image).count();
        quote!(ShaderEntry {
            name: #name,
            blend: Blend::#blend,
            vertex: #vertex,
            images: #images,
        })
    }

    pub fn entry(&self) -> String {
        self.entry.value()
    }

    fn vertex_entry(&self) -> syn::LitStr {
        syn::LitStr::new(&self.entry.value().replace("_fragment", "_vertex"), self.entry.span())
    }

    fn interface(&self) -> Type {
        let mut ty = self.input.source.clone();
        StaticLifetimes.visit_type_mut(&mut ty);
        ty
    }

    fn payload(&self, isthmus: &TokenStream, payload: &Ident, sample: &Ident) -> TokenStream {
        let interface = self.interface();
        let fields = self.captures.iter().filter(|capture| !capture.image).map(|capture| {
            let name = &capture.name;
            let storage = if capture.buffer { quote!([u32; 2]) } else { capture.source.to_token_stream() };
            quote!(#name: #storage)
        });
        quote! {
            #[allow(non_camel_case_types)]
            type #sample = <#interface as #isthmus::__private::ShaderInput<'static>>::Sample;
            #[allow(non_camel_case_types)]
            #[derive(Clone, Copy, #isthmus::ShaderData)]
            struct #payload {
                __geometry: <#sample as #isthmus::geometry::GeometrySample<'static>>::Payload,
                #(#fields),*
            }
        }
    }

    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let declaration = &self.declaration;
        let input_type = &self.input.source;
        let interface_type = self.interface();
        let types = self.captures.iter().map(|capture| &capture.source);
        let payload = self.payload(isthmus, &format_ident!("__IsthmusPayload"), &format_ident!("__IsthmusSample"));
        let program = quote!(<#interface_type as #isthmus::__private::ShaderInput<'static>>::Program);
        let sample = quote!(<#interface_type as #isthmus::__private::ShaderInput<'static>>::Sample);
        let entry = &self.entry;
        let bindings = self.captures.iter().filter(|capture| capture.image).map(|capture| {
            let name = &capture.name;
            let ty = &capture.source;
            quote!(let #name: &#ty = &#name;)
        });
        let fields = self.captures.iter().filter(|capture| !capture.image).map(|capture| {
            let name = &capture.name;
            if capture.buffer { quote!(#name: __isthmus_gpu.capture_buffer(#name)) } else { quote!(#name) }
        });
        let blend = &self.blend;
        let options = self.options.as_ref().map(|options| quote! {
            const _: () = assert!(matches!(#options, #isthmus::Blend::#blend), "blend mode differs from generated pipeline");
        });
        let images: Vec<_> =
            self.captures.iter().filter(|capture| capture.image).map(|capture| &capture.name).collect();
        let image = if images.is_empty() { quote!(None) } else { quote!(Some(__isthmus_gpu.images(&[#(#images),*]))) };
        quote!({
            #options
            let _: fn(#input_type, #(#types),*) -> #isthmus::glam::Vec4 = #declaration;
            #payload
            // SAFETY: Both interfaces are generated from this declaration and its checked entry.
            unsafe impl #isthmus::__private::ShaderSpec for __IsthmusPayload {
                type Program = #program;
                type Sample = #sample;
                const INDEX: usize = #isthmus::__private::shader_index(
                    <#program as #isthmus::Program>::SHADERS,
                    #entry,
                );
            }
            const _: usize = <__IsthmusPayload as #isthmus::__private::ShaderSpec>::INDEX;
            #(#bindings)*
            move |__isthmus_gpu: &mut #isthmus::__private::Gpu, __isthmus_geometry| {
                (__IsthmusPayload { __geometry: __isthmus_geometry, #(#fields),* }, #image)
            }
        })
    }

    pub fn gpu(&self, isthmus: &TokenStream) -> TokenStream {
        let payload_name = format_ident!("{}Payload", self.entry.value());
        let sample_name = format_ident!("{}Sample", self.entry.value());
        let shade = format_ident!("{}_shade", self.entry.value());
        let payload = self.payload(isthmus, &payload_name, &sample_name);
        let input_name = &self.input.name;
        let input_type = &self.input.source;
        let mutable = self.input.mutability;
        let input_binding = quote!(#mutable #input_name);
        let body = match &*self.declaration.body {
            syn::Expr::Block(block) => block.block.to_token_stream(),
            expression => quote!({ #expression }),
        };
        let parameters = self.captures.iter().map(|capture| {
            let name = &capture.name;
            let mutable = capture.mutability;
            let ty = if capture.image {
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
            &self
                .captures
                .iter()
                .filter(|capture| capture.image)
                .map(|capture| capture.name.clone())
                .collect::<Vec<_>>(),
            &payload_name,
            &input_name.to_token_stream(),
            &quote! {
                let color = #shade(#input_name, #(#arguments),*);
                *out_color = color.truncate().extend(1.0) * color.w;
            },
        );
        let vertex = vertex(isthmus, &self.vertex_entry(), &sample_name);
        quote! {
            #payload
            fn #shade(#input_binding: #input_type, #(#parameters),*) -> #isthmus::glam::Vec4 #body
            #entry
            #vertex
        }
    }
}

struct StaticLifetimes;

impl VisitMut for StaticLifetimes {
    fn visit_lifetime_mut(&mut self, i: &mut syn::Lifetime) {
        *i = parse_quote!('static);
    }
}
