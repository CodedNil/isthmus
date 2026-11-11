//! Typed captures of a `shader!` closure.
use syn::{Ident, Pat, Type};

/// One `name: Type` capture of a shader closure.
pub(super) struct Binding {
    pub(super) name: Ident,
    pub(super) ty: Type,
    pub(super) kind: CaptureKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureKind {
    Data,
    Image,
    Slice,
}

impl Binding {
    pub(super) fn parse(pat: Pat) -> syn::Result<Self> {
        if let Pat::Type(pat) = &pat
            && let Pat::Ident(name) = &*pat.pat
            && name.mutability.is_none()
            && name.by_ref.is_none()
            && name.subpat.is_none()
        {
            if let Type::Reference(reference) = &*pat.ty
                && reference.mutability.is_some()
            {
                return Err(syn::Error::new_spanned(&pat.ty, "shader captures must be immutable"));
            }
            let name = name.ident.clone();
            let image = |ty: &Type| matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Image"));
            let kind = match &*pat.ty {
                Type::Reference(reference) if matches!(&*reference.elem, Type::Slice(_)) => CaptureKind::Slice,
                Type::Reference(reference) if image(&reference.elem) => CaptureKind::Image,
                ty if image(ty) => CaptureKind::Image,
                _ => CaptureKind::Data,
            };
            return Ok(Self { name, ty: (*pat.ty).clone(), kind });
        }
        Err(syn::Error::new_spanned(pat, "expected a typed capture: name: Type"))
    }
}
