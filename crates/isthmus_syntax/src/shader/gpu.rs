//! GPU expansion of `shader!`: the vertex and fragment entry points built into the shader crate.
use super::{
    Shader,
    capture::{Binding, CaptureKind},
    slices::SliceIndices,
};
use isthmus_abi as abi;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::{Ident, Type, visit_mut::VisitMut as _};

impl Shader {
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
        let vertex = entry_point(isthmus, &vertex_name, true, &images, &payload_name, &quote! {
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
        let fragment = entry_point(isthmus, &self.entry, false, &images, &payload_name, &quote! {
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

/// Name of the entry-point parameter carrying a captured image.
pub(super) fn image_name(name: &Ident) -> Ident {
    format_ident!("__isthmus_image_{name}")
}

/// One vertex or fragment entry point, wrapping `body` with the shared binding interface.
fn entry_point(
    isthmus: &TokenStream,
    entry_name: &syn::LitStr,
    vertex: bool,
    images: &[&Ident],
    payload: &Ident,
    body: &TokenStream,
) -> TokenStream {
    let stage = if vertex { format_ident!("vertex") } else { format_ident!("fragment") };
    let interface = if vertex {
        quote! {
            #[spirv(vertex_index)] __index: u32,
            #[spirv(instance_index)] __draw_index: u32,
            #[spirv(position)] __out_position: &mut #isthmus::glam::Vec4,
            #[spirv(location = 0)] __out_uv: &mut #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)] __out_draw_index: &mut u32,
        }
    } else {
        quote! {
            #[spirv(frag_coord)] __pixel: #isthmus::glam::Vec4,
            #[spirv(location = 0)] __uv: #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)] __draw_index: u32,
            #[spirv(location = 0)] __out_color: &mut #isthmus::glam::Vec4,
        }
    };
    let name = format_ident!("{}", entry_name.value());
    let (frame_set, frame_binding, resources_binding) = (abi::FRAME_SET, abi::FRAME_BINDING, abi::RESOURCES_BINDING);
    let (image_set, sampler_binding) = (abi::IMAGE_SET, abi::SAMPLER_BINDING);
    let image_resources = images.iter().zip(abi::FIRST_IMAGE_BINDING..).map(|(name, binding)| {
        let image = image_name(name);
        quote! {
            #[spirv(descriptor_set = #image_set, binding = #binding)]
            #image: &#isthmus::spirv_std::image::Image2d,
        }
    });
    let sampler = (!images.is_empty()).then(|| {
        quote! {
            #[spirv(descriptor_set = #image_set, binding = #sampler_binding)]
            __isthmus_sampler: &#isthmus::spirv_std::Sampler,
        }
    });
    quote! {
        #[#isthmus::spirv_std::spirv(#stage(entry_point_name = #entry_name))]
        pub fn #name(
            #interface
            #[spirv(storage_buffer, descriptor_set = #frame_set, binding = #frame_binding)]
            __payload: &[u32],
            #[spirv(storage_buffer, descriptor_set = #frame_set, binding = #resources_binding)]
            __resources: &[u32],
            #sampler
            #(#image_resources)*
        ) {
            type __FrameData = #isthmus::ShaderFrame<Program>;
            let __draw = __draw_index as usize * (<#payload as #isthmus::ShaderData>::WORDS + 1);
            // SAFETY: The recorder aligns complete payloads and their frame offsets to this stride.
            let (__frame_data, __instance) = unsafe {
                let frame = <u32 as #isthmus::ShaderData>::read_unchecked(__payload, __draw) as usize;
                (<__FrameData as #isthmus::ShaderData>::read_unchecked(__payload, frame),
                 <#payload as #isthmus::ShaderData>::read_unchecked(__payload, __draw + 1))
            };
            #body
        }
    }
}
