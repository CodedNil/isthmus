//! Macros for declaring typed shader programs, captures, and word codecs.
#![warn(missing_docs)]

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

mod data;
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

/// Derives a word codec with optional `unorm16` packing or `view = View<'a>` resource resolution.
#[proc_macro_derive(ShaderData, attributes(shader_data))]
pub fn derive_shader_data(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    data::derive(&input).into()
}

/// Declares a shader program with optional globals and host resource types: `program!(Globals, Resources)`.
#[proc_macro]
pub fn program(input: TokenStream) -> TokenStream {
    let (globals, resources) = match syntax::program_types(input.into()) {
        Ok(types) => types,
        Err(error) => return error.to_compile_error().into(),
    };
    let isthmus = isthmus_path();
    let shared = syntax::program();
    quote! {
        #shared
        impl #isthmus::Program for Program {
            type Globals = #globals;
            type Resources = #resources;
            const SHADERS: &'static [#isthmus::__private::ShaderEntry] =
                include!(concat!(env!("OUT_DIR"), "/isthmus.manifest.rs"));
            const CODE: &'static str = include_str!(concat!(env!("OUT_DIR"), "/isthmus.wgsl"));
        }
        /// Drawing context for this shader program.
        pub type Frame<'a> = #isthmus::Frame<'a, Program>;
        /// Renderer for this shader program.
        pub type Renderer = #isthmus::Renderer<Program>;
    }
    .into()
}

/// Draws inline Rust: `shader!(frame.upload({ typed captures }).vertex(stage).fragment(shader))`.
#[proc_macro]
pub fn shader(input: TokenStream) -> TokenStream {
    let span = proc_macro2::Span::call_site();
    let location = span.start();
    let file = proc_macro::Span::call_site().file();
    syntax::shader::Shader::parse(input.into(), &file, location.line, location.column, &isthmus_path())
        .map_or_else(|error| error.to_compile_error(), |shader| shader.host(&isthmus_path()))
        .into()
}
