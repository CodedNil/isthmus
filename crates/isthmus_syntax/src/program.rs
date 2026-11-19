//! The `program!(Globals)` declaration.
use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse::Parser as _, punctuated::Punctuated};

/// Types a program shares between its shaders.
pub struct Declaration {
    globals: syn::Type,
}

impl Declaration {
    /// Parses the optional `Globals` type, which defaults to `()`.
    pub fn parse(tokens: TokenStream) -> syn::Result<Self> {
        let types = Punctuated::<syn::Type, syn::Token![,]>::parse_terminated.parse2(tokens)?;
        if types.len() > 1 {
            return Err(syn::Error::new_spanned(types, "expected an optional globals type"));
        }
        Ok(Self { globals: types.into_iter().next().unwrap_or_else(|| syn::parse_quote!(())) })
    }

    /// The marker type both the host and the shader crate hang their `Program` impl on.
    fn marker() -> TokenStream {
        quote! {
            /// Shader program declared by this module.
            #[derive(Clone, Copy)]
            pub struct Program;
        }
    }

    /// Host expansion: the program with its compiled shader table, and aliases for drawing with it.
    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let (marker, globals) = (Self::marker(), &self.globals);
        quote! {
            #marker
            impl #isthmus::Program for Program {
                type Globals = #globals;
                const SHADERS: &'static [#isthmus::__private::ShaderEntry] =
                    include!(concat!(env!("OUT_DIR"), "/isthmus.manifest.rs"));
            }
            /// Drawing context for this shader program.
            pub type Frame<'draw> = #isthmus::Frame<'draw, Program>;
            /// Renderer for this shader program.
            pub type Renderer = #isthmus::Renderer<Program>;
        }
    }

    /// Shader-crate expansion: only what the GPU code needs to name the program's globals.
    pub fn gpu(&self, isthmus: &TokenStream) -> TokenStream {
        let (marker, globals) = (Self::marker(), &self.globals);
        quote! {
            #marker
            impl #isthmus::Program for Program { type Globals = #globals; }
        }
    }
}
