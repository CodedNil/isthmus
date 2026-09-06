#![allow(dead_code, reason = "the build script and proc macro use opposite sides of the shared shader interface")]

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

#[path = "../../../isthmus/src/bindings.rs"]
pub mod bindings;

pub fn program(isthmus: &TokenStream2) -> TokenStream2 {
    quote! {
        /// Shader program declared by this module.
        #[derive(Clone, Copy)]
        pub struct Program;
        /// Fragment coordinates and queries for the specified geometry in this program.
        pub type Fragment<'a, G> = #isthmus::Fragment<'a, Program, G>;
    }
}

mod shader;
pub use shader::Shader;

fn fragment_entry(
    isthmus: &TokenStream2,
    entry_name: &syn::LitStr,
    images: &[syn::Ident],
    payload: &syn::Ident,
    shader_name: &TokenStream2,
    body: &TokenStream2,
) -> TokenStream2 {
    let name = format_ident!("{}", entry_name.value());
    let draws_binding = bindings::DRAWS;
    let payload_binding = bindings::PAYLOAD;
    let placed_binding = bindings::PLACED_GLYPHS;
    let outlines_binding = bindings::OUTLINES;
    let globals_binding = bindings::GLOBALS;
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
        #[#isthmus::spirv_std::spirv(fragment(entry_point_name = #entry_name))]
        pub fn #name(
            #[spirv(location = 0)]
            pixel: #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)]
            draw_index: u32,
            #[spirv(storage_buffer, descriptor_set = 0, binding = #draws_binding)]
            draws: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #globals_binding)]
            globals: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frames_binding)]
            frame: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #payload_binding)]
            payload: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #placed_binding)]
            placed_glyphs: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #outlines_binding)]
            outlines: &[u32],
            #(#image_resources)*
            #[spirv(location = 0)]
            out_color: &mut #isthmus::glam::Vec4,
        ) {
            // SAFETY: Renderer draw ranges and frame uploads use these generated codecs.
            let (draw, frame) = unsafe {
                (#isthmus::__private::load_unchecked::<#isthmus::__private::DrawRecord>(draws, draw_index),
                 #isthmus::__private::load_unchecked::<#isthmus::__private::FrameData>(frame, 0))
            };
            // SAFETY: Draw offsets address complete payloads encoded by this generated shader interface.
            let instance = unsafe {
                <#payload as #isthmus::ShaderData>::read_unchecked(payload, draw.payload as usize)
            };
            let #shader_name = #isthmus::Fragment::new(
                pixel, draw.geometry, instance.__geometry, frame.time,
                // SAFETY: The renderer uploads this program's complete globals before drawing.
                unsafe { #isthmus::__private::load_unchecked(globals, 0) },
                #isthmus::geometry::text::TextResources { placed_glyphs, outlines },
            );
            #body
        }
    }
}

pub fn vertex(isthmus: &TokenStream2, entry: &syn::LitStr, sample: &syn::Ident) -> TokenStream2 {
    let name = format_ident!("{}", entry.value());
    let draws_binding = bindings::DRAWS;
    let frames_binding = bindings::FRAMES;
    quote! {
        #[#isthmus::spirv_std::spirv(vertex(entry_point_name = #entry))]
        pub fn #name(
            #[spirv(vertex_index)]
            vertex: u32,
            #[spirv(instance_index)]
            draw_index: u32,
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frames_binding)]
            frame: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #draws_binding)]
            draws: &[u32],
            #[spirv(position)]
            out_position: &mut #isthmus::glam::Vec4,
            #[spirv(location = 0)]
            out_pixel: &mut #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)]
            out_draw_index: &mut u32,
        ) {
            // SAFETY: Renderer draw ranges and frame uploads use these generated codecs.
            let (draw, frame) = unsafe {
                (#isthmus::__private::load_unchecked::<#isthmus::__private::DrawRecord>(draws, draw_index),
                 #isthmus::__private::load_unchecked::<#isthmus::__private::FrameData>(frame, 0))
            };
            use #isthmus::geometry::Raster as _;
            type Raster = <#sample as #isthmus::geometry::FragmentGeometry<'static>>::Raster;
            let pixel = Raster::from_data(draw.geometry).vertex(vertex);
            let ndc = pixel / frame.screen_size * 2.0 - 1.0;
            *out_position = #isthmus::glam::vec4(ndc.x, -ndc.y, 0.0, 1.0);
            *out_pixel = pixel;
            *out_draw_index = draw_index;
        }
    }
}
