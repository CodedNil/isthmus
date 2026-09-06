use crate::isthmus_path;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn derive(input: &DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let isthmus = isthmus_path();
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(input, "ShaderData requires a struct").to_compile_error();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(input, "ShaderData requires named fields").to_compile_error();
    };
    if let Some(field) =
        fields.named.iter().find(|field| field.attrs.iter().any(|attr| attr.path().is_ident("shader_data")))
    {
        return syn::Error::new_spanned(field, "shader_data storage options belong on the struct").to_compile_error();
    }
    if let Some(field) = fields
        .named
        .iter()
        .find(|field| field.attrs.iter().any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")))
    {
        return syn::Error::new_spanned(field, "ShaderData fields must be identical on every target")
            .to_compile_error();
    }
    let mut generics = input.generics.clone();
    for parameter in generics.type_params_mut() {
        parameter.bounds.push(syn::parse_quote!(#isthmus::ShaderData));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let mut offset = quote!(0);
    let mut reads = Vec::new();
    let mut zeros = Vec::new();
    let mut writes = Vec::new();
    let mut normalized = false;
    for attribute in input.attrs.iter().filter(|attribute| attribute.path().is_ident("shader_data")) {
        if let Err(error) = attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("unorm16") || normalized {
                return Err(meta.error("expected one unorm16 storage option"));
            }
            normalized = true;
            Ok(())
        }) {
            return error.to_compile_error();
        }
    }
    let fields = fields.named.iter().collect::<Vec<_>>();
    for group in fields.chunks(if normalized { 2 } else { 1 }) {
        let first = &group[0].ident;
        let ty = &group[0].ty;
        let codec = if normalized { quote!(#isthmus::Unorm16x2) } else { quote!(#ty) };
        let value = if normalized {
            let second = group.get(1).map_or_else(
                || quote!(0.0),
                |field| {
                    let name = &field.ident;
                    quote!(self.#name)
                },
            );
            quote!(#codec::from_vec2(#isthmus::glam::Vec2::new(self.#first, #second)))
        } else {
            quote!(self.#first)
        };
        writes.push(quote!(<#codec as #isthmus::ShaderData>::write(#value, words, offset + #offset);));
        for (field, component) in group.iter().zip([quote!(x), quote!(y)]) {
            let name = &field.ident;
            let ty = &field.ty;
            if normalized && !matches!(ty, syn::Type::Path(path) if path.path.is_ident("f32")) {
                return syn::Error::new_spanned(field, "unorm16 requires normalized f32 fields").to_compile_error();
            }
            let decode = normalized.then(|| quote!(.to_vec2().#component));
            zeros.push(quote!(#name: <#ty as #isthmus::ShaderData>::ZERO));
            reads.push(quote!(#name: <#codec as #isthmus::ShaderData>::read_unchecked(words, offset + #offset)#decode));
        }
        offset = quote!(#offset + <#codec as #isthmus::ShaderData>::WORDS);
    }
    quote! {
        impl #impl_generics #isthmus::ShaderData for #name #type_generics #where_clause {
            const WORDS: usize = #offset;
            const ZERO: Self = Self { #(#zeros),* };
            unsafe fn read_unchecked(words: &[u32], offset: usize) -> Self {
                // SAFETY: Field offsets partition the complete record guaranteed by the caller.
                unsafe { Self { #(#reads),* } }
            }
            fn write(self, words: &mut [u32], offset: usize) { #(#writes)* }
        }
    }
}
