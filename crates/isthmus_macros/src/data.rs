use crate::isthmus_path;
use quote::quote;
use syn::{Data, DeriveInput, Member};

pub fn derive(input: &DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let isthmus = isthmus_path();
    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(input, "ShaderData requires a struct").to_compile_error();
    };
    for attribute in data.fields.iter().flat_map(|field| &field.attrs) {
        let message = match attribute.path().get_ident().map(ToString::to_string).as_deref() {
            Some("shader_data") => "shader_data storage options belong on the struct",
            Some("cfg" | "cfg_attr") => "ShaderData fields must be identical on every target",
            _ => continue,
        };
        return syn::Error::new_spanned(attribute, message).to_compile_error();
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
    let mut view: Option<syn::Type> = None;
    for attribute in input.attrs.iter().filter(|attribute| attribute.path().is_ident("shader_data")) {
        if let Err(error) = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("view") && view.is_none() {
                view = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if !meta.path.is_ident("unorm16") || normalized {
                return Err(meta.error("expected a unique unorm16 or view option"));
            }
            normalized = true;
            Ok(())
        }) {
            return error.to_compile_error();
        }
    }
    let resolve = view.as_ref().map_or_else(
        || quote!(type View<'a> = Self; fn resolve(self, _: #isthmus::ResourceData<'_>) -> Self { self }),
        |view| {
            quote! {
                type View<'a> = #view;
                fn resolve(self, resources: #isthmus::ResourceData<'_>) -> Self::View<'_> {
                    Self::View::new(self, resources)
                }
            }
        },
    );
    let fields: Vec<_> = data
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| (field.ident.clone().map_or_else(|| Member::Unnamed(index.into()), Member::Named), field))
        .collect();
    for group in fields.chunks(if normalized { 2 } else { 1 }) {
        let (first, field) = &group[0];
        let ty = &field.ty;
        let codec = if normalized { quote!(#isthmus::Unorm16x2) } else { quote!(#ty) };
        let value = if normalized {
            let second = group.get(1).map_or_else(|| quote!(0.0), |(name, _)| quote!(self.#name));
            quote!(#codec::from_vec2(#isthmus::glam::Vec2::new(self.#first, #second)))
        } else {
            quote!(self.#first)
        };
        writes.push(quote!(<#codec as #isthmus::ShaderData>::write(#value, words, offset + #offset);));
        for ((name, field), component) in group.iter().zip([quote!(x), quote!(y)]) {
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
            #resolve
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
