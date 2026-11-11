//! Small helpers for reading items and `use` trees out of the parsed source.
use proc_macro2::TokenTree;
use std::path::{Path, PathBuf};
use syn::{Item, UseTree, punctuated::Punctuated};

pub(super) fn segments(path: &syn::Path) -> impl Iterator<Item = String> + '_ {
    path.segments.iter().map(|segment| segment.ident.to_string())
}

pub(super) fn item_name(item: &Item) -> Option<String> {
    if let Item::Macro(item) = item
        && item.mac.path.segments.last().is_some_and(|segment| segment.ident == "bitflags")
    {
        let mut tokens = item.mac.tokens.clone().into_iter();
        while let Some(token) = tokens.next() {
            if matches!(token, TokenTree::Ident(name) if name == "struct") {
                return match tokens.next() {
                    Some(TokenTree::Ident(name)) => Some(name.to_string()),
                    _ => None,
                };
            }
        }
    }
    Some(
        match item {
            Item::Const(item) => &item.ident,
            Item::Static(item) => &item.ident,
            Item::Fn(item) => &item.sig.ident,
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::Union(item) => &item.ident,
            _ => return None,
        }
        .to_string(),
    )
}

pub(super) fn flatten(tree: &UseTree, mut prefix: Vec<syn::Ident>, output: &mut Vec<(String, syn::Path)>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.clone());
            flatten(&path.tree, prefix, output);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                flatten(tree, prefix.clone(), output);
            }
        }
        UseTree::Name(name) => {
            if name.ident != "self" {
                prefix.push(name.ident.clone());
            }
            if let Some(name) = prefix.last() {
                output.push((name.to_string(), syn::parse_quote!(#(#prefix)::*)));
            }
        }
        UseTree::Rename(rename) => {
            if rename.ident != "self" {
                prefix.push(rename.ident.clone());
            }
            output.push((rename.rename.to_string(), syn::parse_quote!(#(#prefix)::*)));
        }
        UseTree::Glob(_) => output.push((String::from("*"), syn::parse_quote!(#(#prefix)::*))),
    }
}

pub(super) fn shader_derives(attribute: &syn::Attribute) -> Option<Punctuated<syn::Path, syn::Token![,]>> {
    if !attribute.path().is_ident("derive") {
        return None;
    }
    Some(
        attribute
            .parse_args_with(Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
            .ok()?
            .into_iter()
            .filter(|path| {
                path.segments
                    .last()
                    .is_none_or(|segment| segment.ident != "Deserialize" && segment.ident != "Serialize")
            })
            .collect(),
    )
}

pub(super) fn module_directory(file: &Path) -> PathBuf {
    if ["mod.rs", "lib.rs", "main.rs"].iter().any(|name| file.ends_with(name)) {
        file.parent().unwrap().to_path_buf()
    } else {
        file.with_extension("")
    }
}
