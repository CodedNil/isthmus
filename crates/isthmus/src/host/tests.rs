//! GPU regression tests, run against a headless renderer.
use super::{
    buffer::{Memory, UploadBuffer},
    error::RenderError,
    renderer::Renderer,
};
use crate::{Program, TextureFormat, glam::Vec2, program::ShaderEntry};
use std::{rc::Rc, time::Duration};

#[derive(Clone, Copy)]
struct EmptyProgram;

impl Program for EmptyProgram {
    type Globals = ();

    const SHADERS: &'static [ShaderEntry] = &[];
}

fn headless() -> Renderer<EmptyProgram> {
    Renderer::headless(TextureFormat::Rgba8Unorm).unwrap()
}

#[test]
fn storage_growth_stops_at_the_binding_limit_without_poisoning_the_device() {
    let renderer = headless();
    let memory = Memory::new(Rc::clone(&renderer.gpu.device)).unwrap();
    let mut buffer = UploadBuffer::new(&memory, "bounded storage regression", true).unwrap();
    buffer.limit = 96;
    buffer.upload(&[1; 23]).unwrap();
    assert_eq!(buffer.address_range().size, 96);
    assert!(matches!(buffer.upload(&[2; 25]), Err(RenderError::ResourceLimit { .. })));
    buffer.upload(&[3]).unwrap();
    buffer.upload(&[4; 23]).unwrap();
}

#[test]
fn failed_device_stops_recording_until_recovery() {
    let mut renderer = headless();
    renderer.gpu.error = Some(RenderError::DeviceLost("test device loss".into()));
    let error = renderer.render(|_| panic!("lost device must not record another frame")).unwrap_err();
    assert!(matches!(error, RenderError::DeviceLost(message) if message == "test device loss"));
    renderer.recover().unwrap();
    renderer.render(|_| {}).unwrap();
    renderer.wait(Duration::from_secs(5)).unwrap();
}

#[test]
fn foreign_images_fail_without_poisoning_later_passes_or_recovery() {
    let mut owner = headless();
    let (_, foreign) = owner.add_image([1, 1]);
    let mut renderer = headless();
    let (surface, _) = renderer.add_image([1, 1]);
    let alias = renderer.alias_image(surface).unwrap();

    let error = renderer
        .render(|render| {
            render.surface("target", surface, Vec2::ONE, (), |frame| {
                frame.gpu.emit(frame.surface, 0, &[], &[&foreign]);
            });
        })
        .unwrap_err();
    assert!(matches!(error, RenderError::MissingImage));

    let render_passes = |renderer: &mut Renderer<EmptyProgram>| {
        renderer
            .render(|render| {
                render.surface("target", surface, Vec2::ONE, (), |_| {});
                render.surface("alias", alias, Vec2::ONE, (), |_| {});
            })
            .unwrap();
        renderer.wait(Duration::from_secs(5)).unwrap();
    };
    render_passes(&mut renderer);
    renderer.recover().unwrap();
    render_passes(&mut renderer);
}
