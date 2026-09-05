use isthmus::{Blend, ColorExt as _, Float as _, Image, ShaderData as Payload, glam::Vec4};
use std::fs;

isthmus::program!();

#[repr(C)]
#[derive(Clone, Copy, Payload)]
struct Tone {
    gain: f32 = 0.5,
}

impl Tone {
    fn apply(self, color: Vec4) -> Vec4 {
        color * self.gain
    }

    fn cpu_only(self) -> String {
        fs::read_to_string("unused").unwrap()
    }
}

fn draw(image: Image, gain: f32) {
    isthmus::shader!(|fragment: Fragment, image: Image, mut gain: f32| {
        gain *= 0.5;
        if fragment.local.x < 0.0 {
            return image.sample(fragment.local) * gain;
        }
        Tone { .. }.apply(image.sample(fragment.local)).opacity(gain.saturate())
    });
    isthmus::shader!(Blend::Add, |fragment: TriangleFragment| {
        fragment.barycentric.extend(0.5)
    });
    isthmus::shader!(Blend::Replace, |text: TextFragment| {
        text.color(1.0)
    });
}
