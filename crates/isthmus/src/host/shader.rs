//! The draw builder: `frame.shader("name")`, then the values to send, a shape and a paint.
use super::{arena::Arena, error::RenderError, frame::Frame, gpu::Gpu, surface::SurfaceTarget};
use crate::{
    Blend, Fragment, Image, Primitive, Program, ShaderData, ShaderFrame,
    glam::{Vec2, Vec4},
};
use core::{marker::PhantomData, mem};
use smallvec::SmallVec;

/// A value that can be sent to a shader draw: any shader data, an image, a slice, or a tuple of those.
pub trait Upload<'draw>: Copy {
    #[doc(hidden)]
    fn upload(self, record: &mut Record<'draw>);
}

/// The record being built for a draw: the values it sends, and access to the frame's shared words.
pub struct Record<'draw> {
    gpu: &'draw mut Gpu,
    /// Space for the region, then each value in order.
    payload: Vec<u32>,
    images: SmallVec<[&'draw Image; 2]>,
}

impl Record<'_> {
    /// Adds a value to the draw's record, for implementations of [`Upload`] outside this crate.
    #[doc(hidden)]
    pub fn push(&mut self, value: impl ShaderData) {
        value.append(&mut self.payload);
    }

    /// The frame's words, which every draw can read: records, slices, and anything resident such as text outlines.
    #[doc(hidden)]
    pub const fn arena(&mut self) -> &mut Arena {
        &mut self.gpu.arena
    }
}

impl<'draw, T: ShaderData> Upload<'draw> for T {
    fn upload(self, record: &mut Record<'draw>) {
        record.push(self);
    }
}

impl<'draw, 'value: 'draw> Upload<'draw> for &'value Image {
    fn upload(self, record: &mut Record<'draw>) {
        record.images.push(self);
    }
}

impl<'draw, T: ShaderData> Upload<'draw> for &[T] {
    fn upload(self, record: &mut Record<'draw>) {
        let words = &mut record.arena().words;
        let range = [
            u32::try_from(words.len()).expect("frame words exceed u32"),
            u32::try_from(self.len()).expect("slice exceeds u32"),
        ];
        for &value in self {
            value.append(words);
        }
        record.push(range);
    }
}

/// Implements [`Upload`] for tuples of every length up to the number of names, peeling one name per step.
macro_rules! tuples {
    () => {};
    ($head:ident $($tail:ident)*) => {
        tuples!($($tail)*);
        #[expect(non_snake_case, reason = "the macro names each value after its type parameter")]
        impl<'draw, $head: Upload<'draw>, $($tail: Upload<'draw>),*> Upload<'draw> for ($head, $($tail,)*) {
            fn upload(self, record: &mut Record<'draw>) {
                let ($head, $($tail,)*) = self;
                $head.upload(record);
                $($tail.upload(record);)*
            }
        }
    };
}
tuples!(U T S R Q P O N M L K J I H G F E D C B A);

/// A shader draw being described: send it values, give it a [`shape`](Self::shape), then paint it.
///
/// The name keys the shader that the build extracted, and labels it on the GPU and in logs.
pub struct Shader<'draw, P: Program> {
    surface: &'draw mut SurfaceTarget,
    state: ShaderFrame<P>,
    name: &'static str,
    blend: Blend,
    format: Option<crate::TextureFormat>,
    record: Record<'draw>,
}

impl<P: Program> Frame<'_, P> {
    /// Starts a draw of the shader `name`.
    pub fn shader(&mut self, name: &'static str) -> Shader<'_, P> {
        let mut payload = mem::take(&mut self.gpu.scratch);
        payload.clear();
        payload.resize(<[Vec2; 4]>::WORDS, 0);
        let record = Record { gpu: &mut *self.gpu, payload, images: SmallVec::new() };
        Shader { surface: &mut *self.surface, state: self.state, name, blend: Blend::Over, format: None, record }
    }
}

impl<'draw, P: Program> Shader<'draw, P> {
    /// How the shader's colour combines with the target. The build reads this from the source.
    pub const fn blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    /// Draws to offscreen images of this format instead of the presentation surface. The build reads this too.
    pub const fn format(mut self, format: crate::TextureFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Sends values to the GPU: the variables the shape and paint closures use, as a tuple in any order.
    ///
    /// The build checks that this lists exactly the local variables the closures capture, and reads each one's
    /// type from its `let` annotation, so the GPU knows it as well.
    pub fn with(mut self, values: impl Upload<'draw>) -> Self {
        values.upload(&mut self.record);
        self
    }

    /// Places a draw using host-only geometry. Paint receives full coverage and no prepared sample.
    pub fn bounds(self, bounds: impl Primitive<Sample = ()>) -> Shaped<'draw, P, ()> {
        let region = bounds.region(self.state.pixel_size());
        Shaped { shader: self, region, primitive: PhantomData }
    }

    /// The geometry. The CPU evaluates it to place the draw, and the GPU evaluates it to cover each pixel.
    pub fn shape<S: Primitive>(self, shape: impl FnOnce(ShaderFrame<P>) -> S) -> Shaped<'draw, P, S> {
        let region = shape(self.state).region(self.state.pixel_size());
        Shaped { shader: self, region, primitive: PhantomData }
    }
}

/// A shader draw with its geometry, ready to be painted.
pub struct Shaped<'draw, P: Program, S> {
    shader: Shader<'draw, P>,
    region: Option<[Vec2; 4]>,
    primitive: PhantomData<S>,
}

impl<P: Program, S: Primitive> Shaped<'_, P, S> {
    /// The colour of each pixel the shape covers. The GPU runs this once per pixel; the CPU only checks it.
    pub fn paint(self, _: impl FnOnce(ShaderFrame<P>, Fragment<S::Sample>) -> Vec4) {
        let Shader { surface, name, blend, format, mut record, .. } = self.shader;
        let Some(region) = self.region else {
            record.gpu.scratch = record.payload;
            return;
        };
        let Ok(index) = P::SHADERS.binary_search_by_key(&name, |entry| entry.name) else {
            record.gpu.fail(RenderError::UnknownShader(name));
            return;
        };
        if P::SHADERS[index].images != record.images.len() {
            record.gpu.fail(RenderError::Gpu(format!("wrong image count for {name:?}")));
            return;
        }
        debug_assert!(
            P::SHADERS[index].blend == blend && P::SHADERS[index].format == format,
            "shader {name:?} must declare the same blend and format literally, so the build can see them"
        );
        region.write(&mut record.payload, 0);
        record.gpu.emit(surface, index, &record.payload, &record.images);
        record.gpu.scratch = record.payload;
    }
}
