//! A shader draw found in source, and the GPU entry points built from it.
pub use chain::{Chain, parse};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
pub use slices::SliceIndices;
use syn::{ExprClosure, Ident, Path, Type, parse_quote};
use upload::Upload;

mod chain;
mod gpu;
mod slices;
mod upload;

pub struct Shader {
    /// The name given to `frame.shader(..)`, which keys the shader and labels it on the GPU and in logs.
    pub name: String,
    uploads: Vec<Upload>,
    shape: ExprClosure,
    paint: ExprClosure,
    blend: Ident,
    format: Option<Ident>,
    /// The identifier of its fragment entry point, derived from the name.
    entry: String,
}

impl Shader {
    /// A shader from its chain, given the type of each variable the chain sends.
    pub fn new(chain: Chain, types: Vec<Type>) -> syn::Result<Self> {
        let Chain { name, with, shape, paint, blend, format } = chain;
        for (closure, parameters, shape_of) in
            [(&shape, 1, "|frame| primitive"), (&paint, 2, "|frame, fragment| color")]
        {
            if closure.inputs.len() != parameters {
                return Err(syn::Error::new_spanned(closure, format!("expected a closure of the form {shape_of}")));
            }
        }
        let uploads =
            with.into_iter().zip(types).map(|(name, ty)| Upload::new(name, ty)).collect::<syn::Result<_>>()?;
        // Entry points are identifiers, so the name is reduced to letters and digits.
        let identifier: String =
            name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
        let entry = format!("isthmus_{identifier}");
        Ok(Self { name, uploads, shape, paint, blend, format, entry })
    }

    /// The uploads as typed parameters around the closures, which the build script walks for references. `program`
    /// is the path of the program the shader belongs to, which keeps it reachable.
    pub fn declaration(&self, program: &Path) -> ExprClosure {
        let (shape, paint) = (&self.shape, &self.paint);
        let inputs = self.uploads.iter().map(|Upload { name, ty, .. }| quote!(#name: #ty));
        parse_quote!(|#(#inputs),*| {
            let _: #program;
            #shape;
            #paint;
        })
    }

    /// The name of the vertex entry point, unique to this shader.
    pub fn vertex_entry(&self) -> String {
        format!("{}_vertex", self.entry)
    }

    /// The name of the fragment entry point, unique to this shader.
    pub fn fragment_entry(&self) -> String {
        format!("{}_fragment", self.entry)
    }

    fn images(&self) -> impl Iterator<Item = &Ident> {
        self.uploads.iter().filter(|upload| upload.kind.is_image()).map(|upload| &upload.name)
    }

    pub fn metadata(&self, isthmus: &TokenStream) -> TokenStream {
        let (name, blend) = (&self.name, &self.blend);
        let (vertex, fragment) = (self.vertex_entry(), self.fragment_entry());
        let images = self.images().count();
        let format = self.format.as_ref().map_or_else(
            || quote!(None),
            |format| quote!({ use #isthmus::TextureFormat; Some(TextureFormat::#format) }),
        );
        quote!(unsafe {
            ShaderEntry::new(
                #name, Blend::#blend, #images, #format,
                [concat!(#vertex, "\0"), concat!(#fragment, "\0")],
                [
                    include_bytes!(concat!(env!("OUT_DIR"), "/", #vertex, ".spv")),
                    include_bytes!(concat!(env!("OUT_DIR"), "/", #fragment, ".spv")),
                ],
            )
        })
    }

    /// The draw record's struct: the region to rasterize, then non-image uploads followed by the image table indices
    /// in capture order. Each data field is generic because a struct type cannot name an elided lifetime.
    fn payload(&self, isthmus: &TokenStream, name: &Ident) -> TokenStream {
        let sent = self.uploads.iter().enumerate().filter(|(_, upload)| !upload.kind.is_image());
        let images = self.images();
        let parameters =
            sent.clone().filter(|(_, upload)| upload.kind.is_data()).map(|(index, _)| format_ident!("T{index}"));
        let fields = sent.map(|(index, upload)| {
            let name = &upload.name;
            if upload.kind.is_slice() {
                quote!(#name: [u32; 2])
            } else {
                let parameter = format_ident!("T{index}");
                quote!(#name: #parameter)
            }
        });
        quote! {
            #[derive(Clone, Copy, #isthmus::ShaderData)]
            struct #name<#(#parameters),*> { region: [#isthmus::glam::Vec2; 4], #(#fields,)* #(#images: u32),* }
        }
    }

    /// The draw record's struct with its fields' types filled in, for use inside function bodies.
    fn payload_type(&self, isthmus: &TokenStream, name: &Ident) -> TokenStream {
        let records = self.uploads.iter().filter(|upload| upload.kind.is_data()).map(|upload| {
            let ty = &upload.ty;
            quote!(<#ty as #isthmus::Sent>::Record)
        });
        quote!(#name<#(#records),*>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::Stmt;

    fn find(statement: &TokenStream) -> syn::Result<Option<Chain>> {
        parse(&Stmt::Expr(syn::parse2(statement.clone())?, None))
    }

    #[test]
    fn the_chain_names_its_variables_and_options() {
        let chain = find(&quote! {
            context.frame.shader("Track text").blend(Blend::Add).format(TextureFormat::Rgba16Float)
                .with((center, image)).shape(|_| bounds).paint(|_, fragment| image.sample(fragment.uv))
        })
        .unwrap()
        .unwrap();
        assert_eq!(chain.name, "Track text");
        assert_eq!(chain.with.iter().map(ToString::to_string).collect::<Vec<_>>(), ["center", "image"]);
        let types = vec![parse_quote!(Vec2), parse_quote!(&Image)];
        let shader = Shader::new(chain, types).unwrap();
        assert_eq!(
            (shader.blend.to_string(), shader.format.as_ref().map(ToString::to_string)),
            ("Add".into(), Some("Rgba16Float".into()))
        );
        assert_eq!(shader.fragment_entry(), "isthmus_track_text_fragment");
        assert_eq!(shader.vertex_entry(), "isthmus_track_text_vertex");
        assert_eq!(shader.images().count(), 1);
    }

    #[test]
    fn sent_values_may_be_one_variable_or_none() {
        let sent = |chain: &TokenStream| find(chain).unwrap().unwrap().with.len();
        assert_eq!(sent(&quote!(frame.shader("a").with(x).shape(|_| r).paint(|_, _| c))), 1);
        assert_eq!(sent(&quote!(frame.shader("a").with((x,)).shape(|_| r).paint(|_, _| c))), 1);
        assert_eq!(sent(&quote!(frame.shader("a").shape(|_| r).paint(|_, _| c))), 0);
    }

    #[test]
    fn other_paint_calls_are_not_shaders() {
        assert!(find(&quote!(sample.paint(fill, outline))).unwrap().is_none());
        assert!(find(&quote!(x.shape(|_| r).paint(|_, _| c))).unwrap().is_none());
    }

    #[test]
    fn mistakes_are_reported_where_the_build_can_see_them() {
        let error = |chain: &TokenStream| find(chain).err().map(|error| error.to_string()).unwrap_or_default();
        assert!(error(&quote!(frame.shader(name).shape(|_| r).paint(|_, _| c))).contains("string literal"));
        assert!(
            error(&quote!(frame.shader("a").with(draw.time).shape(|_| r).paint(|_, _| c))).contains("plain variable")
        );
        let chain = find(&quote!(frame.shader("a").with(x).shape(|_, _| r).paint(|_, _| c))).unwrap().unwrap();
        let shader = Shader::new(chain, vec![parse_quote!(f32)]);
        assert!(shader.err().is_some_and(|error| error.to_string().contains("closure of the form")));
        let chain = find(&quote!(frame.shader("a").with(x).shape(|_| r).paint(|_, _| c))).unwrap().unwrap();
        assert!(
            Shader::new(chain, vec![parse_quote!(&mut Image)])
                .err()
                .is_some_and(|e| e.to_string().contains("immutable"))
        );
    }
}
