//! The typed values a shader draw uploads.
use strum_macros::EnumIs;
use syn::{Ident, Type};

/// One `let name = draw.upload::<Type>(value);`.
pub(super) struct Upload {
    pub(super) name: Ident,
    pub(super) ty: Type,
    pub(super) kind: UploadKind,
}

/// How an upload reaches the GPU: in the draw's record, as a bound image, or as a range of the frame's words.
#[derive(Clone, Copy, EnumIs)]
pub(super) enum UploadKind {
    Data,
    Image,
    Slice,
}

impl Upload {
    pub(super) fn new(name: Ident, ty: Type) -> syn::Result<Self> {
        if let Type::Reference(reference) = &ty
            && reference.mutability.is_some()
        {
            return Err(syn::Error::new_spanned(&ty, "shader uploads must be immutable"));
        }
        let image =
            |ty: &Type| matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Image"));
        let kind = match &ty {
            Type::Reference(reference) if matches!(&*reference.elem, Type::Slice(_)) => UploadKind::Slice,
            Type::Reference(reference) if image(&reference.elem) => UploadKind::Image,
            ty if image(ty) => UploadKind::Image,
            _ => UploadKind::Data,
        };
        Ok(Self { name, ty, kind })
    }
}
