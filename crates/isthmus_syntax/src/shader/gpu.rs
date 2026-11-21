//! GPU expansion of a shader draw: the vertex and fragment entry points built into the shader crate.
use super::{
    Shader,
    slices::SliceIndices,
    upload::{Upload, UploadKind},
};
use isthmus_abi as abi;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::{Expr, Path, Type, visit_mut::VisitMut as _};

impl Shader {
    pub fn gpu(&self, isthmus: &TokenStream, program: &Path) -> TokenStream {
        let payload_name = format_ident!("IsthmusPayload{}", self.entry.replace('_', ""));
        let payload = self.payload(isthmus, &payload_name);
        let payload_type = self.payload_type(isthmus, &payload_name);
        let slices = self
            .uploads
            .iter()
            .filter(|upload| upload.kind.is_slice())
            .map(|upload| upload.name.to_string())
            .collect::<BTreeSet<_>>();
        let [shape, paint] = [&self.shape, &self.paint].map(|closure| {
            let mut closure = Expr::Closure(closure.clone());
            SliceIndices(slices.clone()).visit_expr_mut(&mut closure);
            closure
        });
        let bindings = self.uploads.iter().map(|Upload { name, ty, kind }| {
            let value = match kind {
                UploadKind::Image => {
                    let image = quote!(unsafe { __isthmus_images.index(__data.#name as usize) });
                    let value = quote!(#isthmus::Image::new(#image, *__isthmus_sampler));
                    if matches!(ty, Type::Reference(_)) { quote!(&#value) } else { value }
                }
                UploadKind::Slice => quote!(crate::ShaderSlice::from_words(__words, __data.#name)),
                UploadKind::Data => quote!(<#ty as #isthmus::Sent>::restore(__data.#name, __shared)),
            };
            let annotation = kind.is_slice().then(|| {
                let Type::Reference(reference) = ty else { return quote!() };
                let Type::Slice(slice) = &*reference.elem else { return quote!() };
                let element = &slice.elem;
                quote!(: crate::ShaderSlice<'_, #element>)
            });
            quote!(let #name #annotation = #value;)
        });
        let raw = self.blend == "Raw";
        let (vertex_name, fragment_name) = (self.vertex_entry(), self.fragment_entry());
        let (vertex_ident, fragment_ident) = (format_ident!("{vertex_name}"), format_ident!("{fragment_name}"));
        let (frame_set, frame_binding, shared_binding) = (abi::FRAME_SET, abi::FRAME_BINDING, abi::SHARED_BINDING);
        let (image_set, sampler_binding, image_binding) = (abi::IMAGE_SET, abi::SAMPLER_BINDING, abi::IMAGE_BINDING);
        let image_table = self.images().next().is_some().then(|| {
            quote! {
                #[spirv(descriptor_set = #image_set, binding = #sampler_binding)]
                __isthmus_sampler: &#isthmus::spirv_std::Sampler,
                #[spirv(descriptor_set = #image_set, binding = #image_binding)]
                __isthmus_images: &#isthmus::spirv_std::RuntimeArray<#isthmus::spirv_std::image::Image2d>,
            }
        });
        quote! {
            #payload
            #[#isthmus::spirv_std::spirv(vertex(entry_point_name = #vertex_name))]
            pub fn #vertex_ident(
                #[spirv(vertex_index)] __index: u32,
                #[spirv(instance_index)] __draw: u32,
                #[spirv(position)] __position: &mut #isthmus::glam::Vec4,
                #[spirv(location = 0)] __uv: &mut #isthmus::glam::Vec2,
                #[spirv(location = 1, flat)] __out_draw: &mut u32,
                #[spirv(storage_buffer, descriptor_set = #frame_set, binding = #frame_binding)] __words: &[u32],
            ) {
                #isthmus::__private::vertex(
                    __words, <#payload_type as #isthmus::ShaderData>::WORDS, __index, __draw,
                    __position, __uv, __out_draw,
                );
            }
            #[#isthmus::spirv_std::spirv(fragment(entry_point_name = #fragment_name))]
            pub fn #fragment_ident(
                #[spirv(frag_coord)] __coord: #isthmus::glam::Vec4,
                #[spirv(location = 0)] __uv: #isthmus::glam::Vec2,
                #[spirv(location = 1, flat)] __draw: u32,
                #[spirv(location = 0)] __out: &mut #isthmus::glam::Vec4,
                #[spirv(storage_buffer, descriptor_set = #frame_set, binding = #frame_binding)] __words: &[u32],
                #[spirv(storage_buffer, descriptor_set = #frame_set, binding = #shared_binding)] __shared: &[u32],
                #image_table
            ) {
                let (__frame, __data, __fragment) =
                    #isthmus::__private::fragment_input::<#program, #payload_type>(__words, __draw, __coord, __uv);
                #(#bindings)*
                #isthmus::__private::fragment::<#program, _>(__frame, __fragment, #shape, #paint, #raw, __out);
            }
        }
    }
}
