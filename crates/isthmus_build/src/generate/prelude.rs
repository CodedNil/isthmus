//! Boilerplate every generated shader crate starts with.
use proc_macro2::TokenStream;
use quote::quote;

/// Crate attributes, and `ShaderSlice`, which slice captures are rewritten to use.
pub(super) fn prelude() -> TokenStream {
    quote! {
        #![no_std]
        #![feature(default_field_values)]
        #![allow(dead_code, unused_imports, unused_features, reason = "shared shader code may not use every retained method, import, or enabled language feature")]
        extern crate isthmus;
        pub use isthmus::__private::ShaderSlice;
    }
}
