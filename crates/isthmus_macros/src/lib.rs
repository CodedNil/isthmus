use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

mod buffer;
#[path = "../../isthmus_build/src/syntax/mod.rs"]
mod syntax;

fn isthmus_path() -> TokenStream2 {
    match crate_name("isthmus") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            quote!(::#name)
        }
        Err(_) => quote!(::isthmus),
    }
}

#[proc_macro_derive(ShaderData)]
pub fn derive_shader_data(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    buffer::derive(&input).into()
}

/// Declares the shader program owned by the surrounding module.
#[proc_macro]
pub fn program(input: TokenStream) -> TokenStream {
    let globals: syn::Type =
        if input.is_empty() { syn::parse_quote!(()) } else { syn::parse_macro_input!(input as syn::Type) };
    let isthmus = isthmus_path();
    let shared = syntax::program(&isthmus);
    quote! {
        #shared
        // SAFETY: The build generates this program's metadata and validates its shader module together.
        unsafe impl #isthmus::Program for Program {
            type Globals = #globals;
            const SHADERS: &'static [#isthmus::__private::ShaderEntry] =
                include!(concat!(env!("OUT_DIR"), "/isthmus.manifest.rs"));
            const CODE: &'static [u8] = {
                #[cfg(target_arch = "wasm32")]
                { include_bytes!(concat!(env!("OUT_DIR"), "/isthmus.wgsl")) }
                #[cfg(not(target_arch = "wasm32"))]
                { include_bytes!(concat!(env!("OUT_DIR"), "/isthmus.spv")) }
            };
        }
        pub type Frame<'a> = #isthmus::Frame<'a, Program>;
        pub type Renderer = #isthmus::Renderer<Program>;
    }
    .into()
}

/// Declares GPU code and typed CPU captures for a geometry or text paint.
#[proc_macro]
pub fn shader(input: TokenStream) -> TokenStream {
    let span = proc_macro2::Span::call_site();
    let location = span.start();
    let file = proc_macro::Span::call_site().file();
    syntax::Shader::parse(input.into(), &file, location.line, location.column)
        .map_or_else(|error| error.to_compile_error(), |shader| shader.host(&isthmus_path()))
        .into()
}
