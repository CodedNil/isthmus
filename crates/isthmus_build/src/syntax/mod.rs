#![expect(dead_code, reason = "the build script and proc macro use opposite sides of the shared shader interface")]

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;

#[path = "../../../isthmus/src/bindings.rs"]
pub mod bindings;

pub fn program_types(tokens: TokenStream2) -> syn::Result<(syn::Type, syn::Type)> {
    use syn::parse::Parser;
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

fn shader_entry(
    isthmus: &TokenStream2,
    entry_name: &syn::LitStr,
    vertex: bool,
    images: &[&syn::Ident],
    payload: &syn::Ident,
    varyings: &TokenStream2,
    body: &TokenStream2,
) -> TokenStream2 {
    let stage = if vertex { format_ident!("vertex") } else { format_ident!("fragment") };
    let interface = if vertex {
        quote! {
            #[spirv(vertex_index)] index: u32,
            #[spirv(instance_index)] draw_index: u32,
            #[spirv(position)] out_position: &mut #isthmus::glam::Vec4,
            #[spirv(location = 0)] out_uv: &mut #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)] out_draw_index: &mut u32,
        }
    } else {
        quote! {
            #[spirv(frag_coord)] pixel: #isthmus::glam::Vec4,
            #[spirv(location = 0)] uv: #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)] draw_index: u32,
            #[spirv(location = 0)] out_color: &mut #isthmus::glam::Vec4,
        }
    };
    let name = format_ident!("{}", entry_name.value());
    let draws_binding = bindings::DRAWS;
    let payload_binding = bindings::PAYLOAD;
    let transient_binding = bindings::TRANSIENT;
    let persistent_binding = bindings::PERSISTENT;
    let frames_binding = bindings::FRAMES;
    let image_resources = images.iter().enumerate().map(|(index, name)| {
        let image_binding = index as u32 * 2;
        let sampler_binding = image_binding + 1;
        let image = format_ident!("__isthmus_image_{name}");
        let sampler = format_ident!("__isthmus_sampler_{name}");
        quote! {
            #[spirv(descriptor_set = 1, binding = #image_binding)]
            #image: &#isthmus::spirv_std::image::Image2d,
            #[spirv(descriptor_set = 1, binding = #sampler_binding)]
            #sampler: &#isthmus::spirv_std::Sampler,
        }
    });
    quote! {
        #[#isthmus::spirv_std::spirv(#stage(entry_point_name = #entry_name))]
        pub fn #name(
            #interface
            #varyings
            #[spirv(storage_buffer, descriptor_set = 0, binding = #draws_binding)]
            draws: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frames_binding)]
            frame: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #payload_binding)]
            payload: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #transient_binding)]
            _transient: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #persistent_binding)]
            _persistent: &[u32],
            #(#image_resources)*
        ) {
            type FrameData = #isthmus::__private::FrameData<<Program as #isthmus::Program>::Globals>;
            // SAFETY: Renderer draw ranges and frame uploads use these generated codecs.
            let (draw, frame) = unsafe {
                (<u32 as #isthmus::ShaderData>::read_unchecked(draws, draw_index as usize),
                 <FrameData as #isthmus::ShaderData>::read_unchecked(frame, 0))
            };
            // SAFETY: Draw offsets address complete payloads encoded by this generated shader interface.
            let _instance = unsafe {
                <#payload as #isthmus::ShaderData>::read_unchecked(payload, draw as usize)
            };
            #body
        }
    }
}
