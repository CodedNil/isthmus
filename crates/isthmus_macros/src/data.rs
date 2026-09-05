use crate::isthmus_path;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn derive(input: &DeriveInput) -> proc_macro2::TokenStream {
    if !input.attrs.iter().any(
        |attribute| matches!(&attribute.meta, syn::Meta::List(list) if attribute.path().is_ident("repr") && list.tokens.to_string() == "C"),
    ) {
        return syn::Error::new_spanned(input, "ShaderData requires #[repr(C)]").to_compile_error();
    }
    let name = &input.ident;
    let isthmus = isthmus_path();
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(input, "ShaderData requires a struct").to_compile_error();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(input, "ShaderData requires named fields").to_compile_error();
    };
    if let Some(field) = fields
        .named
        .iter()
        .find(|field| field.attrs.iter().any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")))
    {
        return syn::Error::new_spanned(
            field,
            "ShaderData fields must have the same layout on every target; conditional fields are not supported",
        )
        .to_compile_error();
    }
    let types = fields.named.iter().map(|field| &field.ty).collect::<Vec<_>>();
    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(&input.generics, "ShaderData requires a concrete struct").to_compile_error();
    }
    quote! {
        const _: () = {
            assert!(core::mem::size_of::<#name>() == 0 #(+ core::mem::size_of::<#types>())*, "ShaderData cannot contain padding");
            assert!(core::mem::align_of::<#name>() <= 4, "ShaderData alignment cannot exceed four bytes");
            assert!(core::mem::size_of::<#name>().is_multiple_of(4), "ShaderData size must be a multiple of four bytes");
        };
        #[cfg(not(target_arch = "spirv"))]
        unsafe impl #isthmus::__private::bytemuck::Zeroable for #name where #(#types: #isthmus::ShaderData,)* {}
        #[cfg(not(target_arch = "spirv"))]
        unsafe impl #isthmus::__private::bytemuck::Pod for #name where #(#types: #isthmus::ShaderData,)* {}
        unsafe impl #isthmus::ShaderData for #name where #(#types: #isthmus::ShaderData,)* {}
    }
}
