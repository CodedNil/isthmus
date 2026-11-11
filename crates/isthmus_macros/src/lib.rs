//! Proc-macro entry points. The expansion logic lives in `isthmus_syntax`, shared with the build script.
use isthmus_syntax::{derive, program::Declaration, shader::Shader};
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

/// How the expanded code names the runtime crate, which may be renamed or be the crate itself.
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

/// Derives a word codec with optional `unorm16` packing or `view = View<'view>` resource resolution.
#[proc_macro_derive(ShaderData, attributes(shader_data))]
pub fn derive_shader_data(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    derive::derive(&input, &isthmus_path()).into()
}

/// Declares a shader program with optional globals and host resource types: `program!(Globals, Resources)`.
#[proc_macro]
pub fn program(input: TokenStream) -> TokenStream {
    Declaration::parse(input.into())
        .map_or_else(|error| error.to_compile_error(), |program| program.host(&isthmus_path()))
        .into()
}

/// Draws inline Rust: `shader!("name", frame, |frame, name: Type| primitive, |frame, fragment| color)`.
///
/// The name must be unique within the crate. It labels the shader's GPU objects and appears in logs.
#[proc_macro]
pub fn shader(input: TokenStream) -> TokenStream {
    let isthmus = isthmus_path();
    Shader::parse(input.into(), &isthmus)
        .map_or_else(|error| error.to_compile_error(), |shader| shader.host(&isthmus))
        .into()
}
