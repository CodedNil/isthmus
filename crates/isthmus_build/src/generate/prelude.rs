//! Boilerplate every generated shader crate starts with.
use proc_macro2::TokenStream;
use quote::quote;

/// Crate attributes and the `ShaderSlice` type that slice captures are rewritten to use.
pub(super) fn prelude() -> TokenStream {
    quote! {
        #![no_std]
        #![feature(default_field_values)]
        #![allow(dead_code, unused_imports, unused_features, reason = "shared shader code may not use every retained method, import, or enabled language feature")]
        extern crate isthmus;
        #[derive(Clone, Copy)]
        pub struct ShaderSlice<'a, T: isthmus::ShaderData> {
            words: &'a [u32],
            range: [u32; 2],
            marker: core::marker::PhantomData<T>,
        }
        impl<'a, T: isthmus::ShaderData> ShaderSlice<'a, T> {
            fn from_words(words: &'a [u32], range: [u32; 2]) -> Self {
                Self { words, range, marker: core::marker::PhantomData }
            }
            fn len(self) -> usize { self.range[1] as usize }
            fn load(self, index: usize) -> T {
                if index >= self.len() { T::ZERO }
                else { T::read(self.words, self.range[0] as usize + index * T::WORDS) }
            }
        }
    }
}
