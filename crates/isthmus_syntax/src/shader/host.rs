//! Host expansion of `shader!`: records one draw with the captured values.
use super::{
    Shader,
    capture::{Binding, CaptureKind},
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

impl Shader {
    pub fn host(&self, isthmus: &TokenStream) -> TokenStream {
        let (frame, fragment, blend, name) = (&self.frame, &self.fragment, &self.blend, &self.name);
        let variant = &blend.segments.last().unwrap().ident;
        let bindings = self.captures.iter().map(|Binding { name, ty, .. }| quote!(let #name: #ty = #name;));
        let views = self.captures.iter().filter(|capture| capture.kind == CaptureKind::Data).map(|capture| {
            let name = &capture.name;
            quote!(let #name = #isthmus::ShaderData::resolve(
                #name, #isthmus::Resources::data(&*__isthmus_frame.resources)
            );)
        });
        let fields = self.captures.iter().filter(|capture| capture.kind != CaptureKind::Image).map(|capture| {
            let name = &capture.name;
            if capture.kind == CaptureKind::Slice {
                quote!(#name: __isthmus_frame.capture_slice(#name))
            } else {
                quote!(#name)
            }
        });
        let images = self.images();

        let payload = self.payload(isthmus, &format_ident!("__IsthmusPayload"));
        let stage = self.stage(isthmus);
        quote!({
            const _: () = assert!(matches!(#blend, #isthmus::Blend::#variant));
            #(#bindings)*
            let __isthmus_frame: &mut #isthmus::Frame<'_, Program> = (#frame).reborrow();
            let __vertices = {
                #(#views)*
                __isthmus_frame.prepare(#stage, #fragment)
            };
            #payload
            const __SHADER: usize = #isthmus::__private::shader_index(<Program as #isthmus::Program>::SHADERS, #name);
            if __vertices >= 3 {
                let __payload = __IsthmusPayload { #(#fields),* };
                let __images: &[&#isthmus::Image] = &[#(&#images),*];
                __isthmus_frame.record(__SHADER, __vertices, __payload, __images);
            }
        })
    }
}
