#![allow(dead_code, reason = "the build script and proc macro use opposite sides of the shared shader interface")]

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

#[path = "../../../isthmus/src/bindings.rs"]
pub mod bindings;

pub fn program(isthmus: &TokenStream2) -> TokenStream2 {
    quote! {
        #[derive(Clone, Copy)]
        pub struct Program;
        pub type Fragment = #isthmus::Fragment<Program>;
        pub type TextFragment<'a> = #isthmus::TextFragment<'a, Program>;
        pub type TriangleFragment = #isthmus::TriangleFragment<Program>;
    }
}

mod shader;
pub use shader::Shader;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Quad,
    Text,
    Triangle,
}

fn fragment_entry(
    isthmus: &TokenStream2,
    entry_name: &syn::LitStr,
    kind: InputKind,
    payload: bool,
    images: bool,
    shader_input: &syn::Type,
    shader_name: &TokenStream2,
    body: &TokenStream2,
) -> TokenStream2 {
    let draws_binding = bindings::DRAWS;
    let payload_binding = bindings::PAYLOAD;
    let placed_binding = bindings::PLACED_GLYPHS;
    let glyphs_binding = bindings::GLYPHS;
    let curves_binding = bindings::CURVES;
    let globals_binding = bindings::GLOBALS;
    let frames_binding = bindings::FRAMES;
    let image_binding = bindings::IMAGE;
    let sampler_binding = bindings::SAMPLER;
    let (geometry, input) = if kind == InputKind::Triangle {
        (quote!(#isthmus::Triangle), quote!(#isthmus::TriangleFragment))
    } else {
        (quote!(#isthmus::Quad), quote!(#isthmus::Fragment))
    };
    let binding = if kind == InputKind::Text { quote!(fragment) } else { quote!(#shader_name: #shader_input) };
    let text_resources = (kind == InputKind::Text).then(|| {
        quote! {
            #[spirv(storage_buffer, descriptor_set = 0, binding = #placed_binding)]
            placed_glyphs: &[#isthmus::geometry::text::PlacedGlyph],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #glyphs_binding)]
            glyphs: &[#isthmus::geometry::text::Glyph],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #curves_binding)]
            curves: &[#isthmus::geometry::text::Curve],
        }
    });
    let payload_resource = payload
        .then(|| quote!(#[spirv(storage_buffer, descriptor_set = 0, binding = #payload_binding)] payload: &[u32],));
    let image_resources = images.then(|| {
        quote! {
            #[spirv(descriptor_set = 1, binding = #image_binding)]
            __isthmus_image: &#isthmus::spirv_std::image::Image2d,
            #[spirv(descriptor_set = 1, binding = #sampler_binding)]
            __isthmus_sampler: &#isthmus::spirv_std::Sampler,
        }
    });
    quote! {
        #[#isthmus::spirv_std::spirv(fragment(entry_point_name = #entry_name))]
        pub fn fragment(
            #[spirv(location = 0)]
            pixel: #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)]
            draw_index: u32,
            #[spirv(storage_buffer, descriptor_set = 0, binding = #draws_binding)]
            draws: &[#isthmus::__private::DrawRecord],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #globals_binding)]
            globals: &[u32],
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frames_binding)]
            frames: &#isthmus::__private::PushBlock,
            #payload_resource
            #text_resources
            #image_resources
            #[spirv(location = 0)]
            out_color: &mut #isthmus::glam::Vec4,
        ) {
            let draw = draws[draw_index as usize];
            let frame = *frames;
            // SAFETY: The frame globals use the generated shader's declared layout.
            let #binding = #input::new(
                pixel, #geometry::from_data(draw.geometry), frame.time,
                unsafe { #isthmus::__private::load(globals, 0) },
            );
            #body
        }
    }
}

pub fn vertex(isthmus: &TokenStream2, triangle: bool) -> TokenStream2 {
    let draws_binding = bindings::DRAWS;
    let frames_binding = bindings::FRAMES;
    let entry = if triangle { "isthmus_triangle" } else { "isthmus_quad" };
    let name = quote::format_ident!("{entry}");
    let geometry = if triangle { quote!(#isthmus::Triangle) } else { quote!(#isthmus::Quad) };
    quote! {
        #[#isthmus::spirv_std::spirv(vertex(entry_point_name = #entry))]
        pub fn #name(
            #[spirv(vertex_index)]
            vertex: u32,
            #[spirv(instance_index)]
            draw_index: u32,
            #[spirv(storage_buffer, descriptor_set = 0, binding = #frames_binding)]
            frame: &#isthmus::__private::PushBlock,
            #[spirv(storage_buffer, descriptor_set = 0, binding = #draws_binding)]
            draws: &[#isthmus::__private::DrawRecord],
            #[spirv(position)]
            out_position: &mut #isthmus::glam::Vec4,
            #[spirv(location = 0)]
            out_pixel: &mut #isthmus::glam::Vec2,
            #[spirv(location = 1, flat)]
            out_draw_index: &mut u32,
        ) {
            let draw = draws[draw_index as usize];
            let pixel = #geometry::from_data(draw.geometry).vertex(vertex);
            let ndc = pixel / frame.screen_size * 2.0 - 1.0;
            *out_position = #isthmus::glam::vec4(ndc.x, -ndc.y, 0.0, 1.0);
            *out_pixel = pixel;
            *out_draw_index = draw_index;
        }
    }
}
