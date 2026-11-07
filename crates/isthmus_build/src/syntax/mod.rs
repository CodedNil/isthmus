#![expect(dead_code, reason = "the build script and proc macro use opposite sides of the shared shader interface")]

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse::Parser, punctuated::Punctuated};

#[path = "../../../isthmus/src/bindings.rs"]
pub mod bindings;

pub fn program_types(tokens: TokenStream2) -> syn::Result<(syn::Type, syn::Type)> {
    let types = Punctuated::<syn::Type, syn::Token![,]>::parse_terminated.parse2(tokens)?;
    if types.len() > 2 {
        return Err(syn::Error::new_spanned(types, "expected globals and optional resources type"));
    }
    let mut types = types.into_iter();
    Ok((types.next().unwrap_or_else(|| syn::parse_quote!(())), types.next().unwrap_or_else(|| syn::parse_quote!(()))))
}

pub fn program() -> TokenStream2 {
    quote! {
        /// Shader program declared by this module.
        #[derive(Clone, Copy)]
        pub struct Program;
    }
}

pub mod shader;

fn image_name(name: &syn::Ident) -> syn::Ident {
    format_ident!("__isthmus_image_{name}")
}

fn shader_entry(
    isthmus: &TokenStream2,
    entry_name: &syn::LitStr,
    vertex: bool,
    images: &[&syn::Ident],
    payload: &syn::Ident,
    body: &TokenStream2,
) -> TokenStream2 {
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
    let frame_binding = bindings::FRAME;
    let resources_binding = bindings::RESOURCES;
    let image_resources = images.iter().enumerate().map(|(index, name)| {
        let image_binding = index as u32 + 1;
        let image = image_name(name);
        quote! {
            #[spirv(descriptor_set = 1, binding = #image_binding)]
            #image: &#isthmus::spirv_std::image::Image2d,
        }
    });
    let sampler = (!images.is_empty()).then(|| {
        quote! {
            #[spirv(descriptor_set = 1, binding = 0)] __isthmus_sampler: &#isthmus::spirv_std::Sampler,
        }
    });
    quote! {
        #[#isthmus::spirv_std::spirv(#stage(entry_point_name = #entry_name))]
        pub fn #name(
            #interface
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frame_binding)]
            __payload: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #resources_binding)]
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
