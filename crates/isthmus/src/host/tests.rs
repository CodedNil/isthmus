//! GPU regression tests, run against a headless renderer.
use super::{buffer::UploadBuffer, error::RenderError, renderer::Renderer, surface::SurfaceHandle};
use crate::{Blend, Image, Program, glam::Vec2, program::ShaderEntry};
use std::{sync::mpsc, time::Duration};

const IMAGE_VERTEX: &str = "
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
let points = array<vec2<f32>, 6>(
    vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
    vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
);
let point = points[index];
return VertexOutput(vec4(point, 0.0, 1.0), point * 0.5 + 0.5);
}
";
const IMAGE_FRAGMENT: &str = "
@group(1) @binding(0) var image_sampler: sampler;
@group(1) @binding(1) var image_texture: texture_2d<f32>;
@fragment fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
return textureSample(image_texture, image_sampler, uv);
}
";

#[derive(Clone, Copy)]
struct ImageProgram;

impl Program for ImageProgram {
    type Globals = ();
    type Resources = ();

    const SHADERS: &'static [ShaderEntry] = &[ShaderEntry {
        name: "image",
        blend: Blend::Replace,
        images: 1,
        format: None,
        code: [IMAGE_VERTEX, IMAGE_FRAGMENT],
    }];
}

#[derive(Clone, Copy)]
struct EmptyProgram;

impl Program for EmptyProgram {
    type Globals = ();
    type Resources = ();

    const SHADERS: &'static [ShaderEntry] = &[];
}

fn headless() -> Renderer<EmptyProgram> {
    pollster::block_on(Renderer::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap()
}

#[test]
fn frames_without_a_presentation_texture_still_submit_uploads() {
    let mut renderer = headless();
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("occluded upload regression"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    renderer.queue().write_buffer(&buffer, 0, &[7; 4]);
    renderer.render(|_| {}).unwrap();
    let (send, recv) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        send.send(result).unwrap();
    });
    renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(&*buffer.slice(..).get_mapped_range().unwrap(), &[7; 4]);
}

#[test]
fn storage_growth_stops_at_the_binding_limit_without_poisoning_the_device() {
    let renderer = headless();
    let (device, queue) = pollster::block_on(renderer.gpu.adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits { max_storage_buffer_binding_size: 96, ..Default::default() },
        ..Default::default()
    }))
    .unwrap();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut buffer = UploadBuffer::new(&device, "bounded storage regression");
    buffer.upload(&device, &queue, &[1; 23]).unwrap();
    assert_eq!(buffer.buffer.size(), 96);
    assert!(matches!(buffer.upload(&device, &queue, &[2; 25]), Err(RenderError::ResourceLimit { .. })));
    buffer.upload(&device, &queue, &[3]).unwrap();
    buffer.upload(&device, &queue, &[4; 23]).unwrap();
    queue.submit([]);
    assert!(pollster::block_on(scope.pop()).is_none());
}

#[test]
fn invalid_buffer_preserves_original_error_and_stops_rendering() {
    let mut renderer = headless();
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("invalid storage regression"),
        size: renderer.device().limits().max_buffer_size + 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // Reproduce the secondary error in the crash report without panicking.
    renderer.queue().write_buffer(&buffer, 0, &[0; 4]);
    for _ in 0..2 {
        let error = renderer.render(|_| panic!("failed renderer must not record another frame")).unwrap_err();
        let RenderError::Gpu(message) = error else { panic!("expected GPU validation diagnostics") };
        assert!(message.contains("Device::create_buffer"), "original allocation error was lost: {message}");
        assert!(message.contains("invalid storage regression"), "buffer label was lost: {message}");
    }
}

#[test]
fn device_loss_returns_error_instead_of_recording() {
    let mut renderer = headless();
    let error = renderer
        .render(|render| {
            render.renderer.device().destroy();
            render.renderer.device().poll(wgpu::PollType::Poll).unwrap();
        })
        .unwrap_err();
    assert!(matches!(error, RenderError::DeviceLost(_)), "expected device loss during frame: {error}");
    let error = renderer.render(|_| panic!("lost device must not record another frame")).unwrap_err();
    assert!(matches!(error, RenderError::DeviceLost(_)), "expected device loss: {error}");
}

#[test]
fn foreign_images_fail_without_poisoning_later_passes_or_recovery() {
    let mut owner =
        pollster::block_on(Renderer::<ImageProgram>::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
    let (_, foreign) = owner.add_image([1, 1]);
    let mut renderer =
        pollster::block_on(Renderer::<ImageProgram>::headless((), wgpu::TextureFormat::Rgba8Unorm)).unwrap();
    let (first, first_image) = renderer.add_image([1, 1]);
    let (second, second_image) = renderer.add_image([1, 1]);
    let source = Image::rgba8([1, 1], vec![255, 128, 0, 255]);
    let error = renderer
        .render(|render| {
            render.surface(first, Vec2::ONE, (), |mut frame| {
                frame.record(0, 6, (), &[&foreign]);
            });
        })
        .unwrap_err();
    assert!(matches!(error, RenderError::MissingImage));
    copy_through(&mut renderer, [first, second], &source, &first_image).unwrap();
    assert_eq!(pixel(&renderer, &first_image), [255, 128, 0, 255]);
    assert_eq!(pixel(&renderer, &second_image), [255, 128, 0, 255]);
    renderer.device().destroy();
    renderer.device().poll(wgpu::PollType::Poll).unwrap();
    pollster::block_on(renderer.recover()).unwrap();
    copy_through(&mut renderer, [first, second], &source, &first_image).unwrap();
    assert_eq!(pixel(&renderer, &first_image), [255, 128, 0, 255]);
    assert_eq!(pixel(&renderer, &second_image), [255, 128, 0, 255]);
}

/// Draws `source` into the first surface, then that surface's image into the second.
fn copy_through(
    renderer: &mut Renderer<ImageProgram>,
    [first, second]: [SurfaceHandle; 2],
    source: &Image,
    first_image: &Image,
) -> Result<(), RenderError> {
    renderer.render(|render| {
        render.surface(first, Vec2::ONE, (), |mut frame| frame.record(0, 6, (), &[source]));
        render.surface(second, Vec2::ONE, (), |mut frame| frame.record(0, 6, (), &[first_image]));
    })
}

fn pixel(renderer: &Renderer<ImageProgram>, image: &Image) -> [u8; 4] {
    let texture = renderer.image_view(image).unwrap().texture();
    let buffer = renderer.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("image pass regression"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device().create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
        },
        texture.size(),
    );
    renderer.queue().submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| send.send(result).unwrap());
    renderer.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    mapped[..4].try_into().unwrap()
}
