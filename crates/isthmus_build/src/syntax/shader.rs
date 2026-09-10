use super::{image_names, shader_entry};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, Ident, Pat, Stmt, Type, parse_quote, spanned::Spanned, token::Move};

struct Binding {
    name: Ident,
    ty: Type,
    value: Expr,
    kind: CaptureKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Data,
    Image,
    Buffer,
}

impl Binding {
    fn parse(local: syn::Local) -> syn::Result<Self> {
        if let Pat::Type(pat) = &local.pat
            && let Pat::Ident(name) = &*pat.pat
            && name.mutability.is_none()
            && name.by_ref.is_none()
            && name.subpat.is_none()
            && local.init.as_ref().is_none_or(|init| init.diverge.is_none())
            && local.attrs.is_empty()
        {
            let name = name.ident.clone();
            let value = local.init.as_ref().map_or_else(|| parse_quote!(#name), |init| (*init.expr).clone());
            let ty = if let Type::Reference(reference) = &*pat.ty { &*reference.elem } else { &*pat.ty };
            let kind = match ty {
                Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Image") => CaptureKind::Image,
                Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Buffer") => {
                    CaptureKind::Buffer
                }
                _ => CaptureKind::Data,
            };
            return Ok(Self { name, ty: (*pat.ty).clone(), value, kind });
        }
        Err(syn::Error::new_spanned(local, "expected let name: Type; or let name: Type = value;"))
    }
}

struct Output {
    name: Ident,
    ty: Type,
    flat: bool,
}

pub struct Shader {
    pub declaration: syn::ExprClosure,
    frame: Expr,
    stage: Expr,
    fragment: syn::ExprClosure,
    captures: Vec<Binding>,
    outputs: Vec<Output>,
    pub entry: syn::LitStr,
    blend: syn::Path,
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

fn custom(stage: &mut Expr) -> syn::Result<Option<&mut syn::ExprClosure>> {
    if let Expr::Closure(factory) = stage {
        return custom(&mut factory.body);
    }
    if let Expr::Block(block) = stage {
        return match block.block.stmts.last_mut() {
            Some(Stmt::Expr(tail, None)) => custom(tail),
            _ => Ok(None),
        };
    }
    if let Expr::Call(call) = stage
        && let Expr::Path(path) = &*call.func
        && path.path.segments.last().is_some_and(|segment| segment.ident == "vertices")
    {
        if call.args.len() != 2 {
            return Err(syn::Error::new_spanned(call, "expected vertices(count, |vertex| { ... })"));
        }
        return closure(call.args.last_mut().unwrap(), 1).map(Some);
    }
    Ok(None)
}

impl Shader {
    pub fn parse(
        tokens: TokenStream,
        file: &str,
        line: usize,
        column: usize,
        isthmus: &TokenStream,
    ) -> syn::Result<Self> {
        let mut frame: Expr = syn::parse2(tokens)?;
        let mut fragment = argument(&mut frame, "fragment")?
            .ok_or_else(|| syn::Error::new_spanned(&frame, "end the shader with .fragment(|frame, fragment| color)"))?;
        let fragment = closure(&mut fragment, 2)?.clone();
        let mut stage = argument(&mut frame, "primitive")?
            .ok_or_else(|| syn::Error::new_spanned(&frame, "select .primitive(shape) or .primitive(|frame| shape)"))?;
        if !matches!(stage, Expr::Closure(_)) {
            stage = parse_quote!(|_| #stage);
        }
        closure(&mut stage, 1)?;
        let mut captures = Vec::new();
        if let Some(upload) = argument(&mut frame, "upload")? {
            let Expr::Block(block) = upload else {
                return Err(syn::Error::new_spanned(upload, "expected { let name: Type = value; }"));
            };
            for statement in block.block.stmts {
                let Stmt::Local(local) = statement else {
                    return Err(syn::Error::new_spanned(statement, "uploads must be typed let declarations"));
                };
                captures.push(Binding::parse(local)?);
            }
        }
        let blend = argument(&mut frame, "blend")?.unwrap_or_else(|| parse_quote!(#isthmus::Blend::Over));
        let Expr::Path(blend) = blend else {
            return Err(syn::Error::new_spanned(blend, "expected Blend::Over, Blend::Add or Blend::Replace"));
        };
        if let Expr::MethodCall(call) = &frame
            && ["blend", "upload", "primitive", "fragment"].iter().any(|name| call.method == *name)
        {
            return Err(syn::Error::new_spanned(
                call,
                "shader operations must occur once, in blend/upload/primitive/fragment order",
            ));
        }
        let mut outputs = Vec::new();
        if let Some(vertex) = custom(&mut stage)? {
            let statements = match &*vertex.body {
                Expr::Block(body) => body.block.stmts.as_slice(),
                _ => &[],
            };
            for statement in statements {
                let Stmt::Local(local) = statement else { continue };
                if let Pat::Type(pat) = &local.pat
                    && let Type::Path(path) = &*pat.ty
                    && let Some(marker) = path.path.segments.last()
                    && (marker.ident == "Flat" || marker.ident == "Smooth")
                {
                    let binding = Binding::parse(local.clone())?;
                    let syn::PathArguments::AngleBracketed(args) = &marker.arguments else {
                        return Err(syn::Error::new_spanned(marker, "specify the output type"));
                    };
                    let Some(syn::GenericArgument::Type(ty)) = args.args.first().filter(|_| args.args.len() == 1)
                    else {
                        return Err(syn::Error::new_spanned(args, "specify one shader output type"));
                    };
                    if outputs.len() == 14 {
                        return Err(syn::Error::new_spanned(
                            binding.name,
                            "vertex outputs must fit 14 stage locations",
                        ));
                    }
                    outputs.push(Output { name: binding.name, ty: ty.clone(), flat: marker.ident == "Flat" });
                }
            }
        }
        let inputs = captures.iter().map(|Binding { name, ty, .. }| quote!(#name: #ty));
        let declaration = parse_quote!(|#(#inputs),*| {
            let _: Program;
            #stage;
            #fragment;
        });
        let file = file.replace('\\', "/");
        let file =
            file.rsplit_once("/src/").map_or_else(|| file.strip_prefix("src/").unwrap_or(&file), |(_, suffix)| suffix);
        let hash = file
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3));
        let entry =
            syn::LitStr::new(&format!("isthmus_{hash:x}_{line}_{column}_fragment"), proc_macro2::Span::call_site());
        Ok(Self { declaration, frame, stage, fragment, captures, outputs, entry, blend: blend.path })
    }

    pub fn vertex_entry(&self) -> String {
        self.entry.value().replace("_fragment", "_vertex")
    }

    fn images(&self) -> impl Iterator<Item = &Ident> {
        self.captures.iter().filter(|capture| capture.kind == CaptureKind::Image).map(|capture| &capture.name)
    }

    pub fn metadata(&self) -> TokenStream {
        let (name, vertex) = (&self.entry, self.vertex_entry());
        let blend = &self.blend.segments.last().unwrap().ident;
        let images = self.images().count();
        quote!(ShaderEntry {
            name: #name, blend: Blend::#blend, images: #images,
            code: [
                include_str!(concat!(env!("OUT_DIR"), "/", #vertex, ".wgsl")),
                include_str!(concat!(env!("OUT_DIR"), "/", #name, ".wgsl")),
            ],
        })
    }

    fn payload(&self, isthmus: &TokenStream, name: &Ident) -> TokenStream {
        let fields = self.captures.iter().filter(|capture| capture.kind != CaptureKind::Image).map(|capture| {
            let (name, ty) = (&capture.name, &capture.ty);
            let ty = if capture.kind == CaptureKind::Buffer { quote!([u32; 2]) } else { quote!(#ty) };
            quote!(#name: #ty)
        });
        quote! {
            #[derive(Clone, Copy, #isthmus::ShaderData)]
            struct #name { #(#fields),* }
        }
    }

    fn stage(&self, isthmus: &TokenStream, data: &Ident) -> (TokenStream, Expr) {
        let mut stage = self.stage.clone();
        if let Some(vertex) = custom(&mut stage).unwrap() {
            let input = vertex.inputs.first().unwrap();
            let typed: syn::ExprClosure = parse_quote!(|#input: #isthmus::VertexInput<Program>| {});
            vertex.inputs = typed.inputs;
            vertex.capture = Some(Move::default());
            if !self.outputs.is_empty()
                && let Expr::Block(body) = &mut *vertex.body
            {
                let tail = body.block.stmts.pop().unwrap();
                let fields = self.outputs.iter().map(|Output { name, .. }| quote!(#name: #name.0));
                let tail = match tail {
                    Stmt::Expr(expr, None) => expr,
                    other => parse_quote!({ #other }),
                };
                body.block.stmts.push(parse_quote!(let __vertex = #tail;));
                body.block.stmts.push(Stmt::Expr(parse_quote!(__vertex.with(#data { #(#fields),* })), None));
            }
        }
        let factory = closure(&mut stage, 1).unwrap();
        let input = factory.inputs.first().unwrap();
        let typed: syn::ExprClosure = parse_quote!(|#input: #isthmus::ShaderFrame<Program>| {});
        factory.inputs = typed.inputs;
        let fields = self.outputs.iter().map(|Output { name, ty, .. }| quote!(pub #name: #ty));
        let declaration = if self.outputs.is_empty() {
            quote!()
        } else {
            quote!(#[derive(Clone, Copy)] struct #data { #(#fields),* })
        };
        (declaration, stage)
    }

    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let (frame, fragment, blend, entry) = (&self.frame, &self.fragment, &self.blend, &self.entry);
        let variant = &blend.segments.last().unwrap().ident;
        let bindings = self.captures.iter().map(|Binding { name, ty, value, .. }| quote!(let #name: #ty = #value;));
        let views = self.captures.iter().filter(|capture| capture.kind == CaptureKind::Data).map(|capture| {
            let name = &capture.name;
            quote!(let #name = #isthmus::ShaderData::resolve(
                #name, #isthmus::Resources::data(&*__isthmus_frame.resources)
            );)
        });
        let fields = self.captures.iter().filter(|capture| capture.kind != CaptureKind::Image).map(|capture| {
            let name = &capture.name;
            if capture.kind == CaptureKind::Buffer {
                quote!(#name: __isthmus_frame.capture_buffer(#name))
            } else {
                quote!(#name)
            }
        });
        let images = self.images();

        let payload = self.payload(isthmus, &format_ident!("__IsthmusPayload"));
        let (outputs, stage) = self.stage(isthmus, &format_ident!("__IsthmusOutputs"));
        quote!({
            const _: () = assert!(matches!(#blend, #isthmus::Blend::#variant));
            #(#bindings)*
            let __isthmus_frame: &mut #isthmus::Frame<'_, Program> = (#frame).reborrow();
            #outputs
            let __vertices = {
                #(#views)*
                __isthmus_frame.prepare(#stage, #fragment)
            };
            #payload
            const __SHADER: usize = #isthmus::__private::shader_index(<Program as #isthmus::Program>::SHADERS, #entry);
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
        let data = format_ident!("IsthmusOutputs{suffix}");
        let (outputs, stage) = self.stage(isthmus, &data);
        let payload = self.payload(isthmus, &payload_name);
        let frame_input = self.fragment.inputs.first().unwrap();
        let input = &self.fragment.inputs[1];
        let body = &self.fragment.body;
        let bindings = self.captures.iter().map(|Binding { name, ty, kind, .. }| {
            let value = match kind {
                CaptureKind::Image => {
                    let (image, sampler) = image_names(name);
                    quote!(#isthmus::__private::ShaderImage::new(#image, *#sampler))
                }
                CaptureKind::Buffer => quote!(#isthmus::Buffer::from_words(payload, _instance.#name)),
                CaptureKind::Data => quote!(#isthmus::ShaderData::resolve(_instance.#name, #isthmus::ResourceData {
                    transient: _transient, persistent: _persistent,
                })),
            };
            let annotation = (*kind == CaptureKind::Buffer).then(|| quote!(: #ty));
            // Captures can be used by only one stage; both stages share this setup.
            quote!(let #name #annotation = #value; let _ = &#name;)
        });
        let setup = quote! {
            type __Program = Program;
            #(#bindings)*
            let __frame = #isthmus::ShaderFrame::<__Program> {
                time: frame.time, screen_size: frame.screen_size,
                pixel_size: frame.pixel_scale.max_element(), globals: frame.globals,
            };
            let __stage = (#stage)(__frame);
        };
        let interface = |vertex: bool| {
            let fields = self.outputs.iter().enumerate().map(|(index, output)| {
                let name = format_ident!("__varying_{}", output.name);
                let ty = &output.ty;
                let ty = if vertex { quote!(&mut #ty) } else { quote!(#ty) };
                let location = index as u32 + 2;
                let flat = output.flat.then(|| quote!(, flat));
                quote!(#[spirv(location = #location #flat)] #name: #ty,)
            });
            quote!(#(#fields)*)
        };
        let (writes, reads): (Vec<_>, Vec<_>) = self
            .outputs
            .iter()
            .map(|Output { name, .. }| {
                let varying = format_ident!("__varying_{name}");
                (quote!(*#varying = vertex.outputs.#name;), quote!(#name: #varying))
            })
            .unzip();
        let read = if self.outputs.is_empty() { quote!(()) } else { quote!(#data { #(#reads),* }) };
        let images = self.images().collect::<Vec<_>>();
        let vertex_name = syn::LitStr::new(&self.vertex_entry(), self.entry.span());
        let vertex = shader_entry(isthmus, &vertex_name, true, &images, &payload_name, &interface(true), &quote! {
            #setup
            let vertex = #isthmus::Primitive::vertex(__stage, #isthmus::VertexInput::<__Program> {
                index, frame: __frame,
            });
            *out_position = vertex.position;
            *out_uv = vertex.uv;
            *out_draw_index = draw_index;
            #(#writes)*
        });
        let fragment = shader_entry(isthmus, &self.entry, false, &images, &payload_name, &interface(false), &quote! {
            #setup
            let __fragment = #isthmus::Fragment {
                pixel: #isthmus::glam::vec2(pixel.x, pixel.y) * frame.pixel_scale,
                uv,
                sample: (),
                coverage: 1.0,
            };
            let (__sample, __coverage) = #isthmus::Primitive::<__Program>::sample(__stage, __fragment, #read);
            let #input = #isthmus::Fragment {
                pixel: __fragment.pixel, uv: __fragment.uv, sample: __sample, coverage: __coverage,
            };
            let #frame_input = __frame;
            let color: #isthmus::glam::Vec4 = (|| #body)();
            if __coverage <= 0.0 { #isthmus::spirv_std::arch::kill(); }
            let alpha = color.w * __coverage;
            *out_color = (color.truncate() * alpha).extend(alpha);
        });
        quote!(#payload #outputs #vertex #fragment)
    }
}
