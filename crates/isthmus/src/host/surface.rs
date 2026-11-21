//! Draw runs and presentation surfaces.
use super::{
    error::{RenderError, SetupError},
    images::{TextureFormat, view},
    setup::{Device, NativeSurface},
};
use crate::Image;
use ash::vk::{self, TaggedStructure as _};
use smallvec::SmallVec;
use std::{mem, ops::Range, rc::Rc};

pub const FRAMES: usize = 2;

pub(super) struct Paint {
    pub shader: usize,
    pub draws: Range<u32>,
}

pub struct SurfaceTarget {
    pub(super) target: Target,
    pub(super) paints: Vec<Paint>,
    pub(super) frame: u32,
    pub(super) name: &'static str,
}

pub(super) enum Target {
    Window(WindowTarget),
    Image(Image),
}

impl SurfaceTarget {
    pub(super) const fn new(target: Target) -> Self {
        Self { target, paints: Vec::new(), frame: 0, name: "" }
    }

    pub(super) const fn size(&self) -> [u32; 2] {
        match &self.target {
            Target::Window(window) => window.size,
            Target::Image(image) => image.size(),
        }
    }
}

pub struct WindowTarget {
    swap: Option<Swapchain>,
    pub surface: NativeSurface,
    pub size: [u32; 2],
    dirty: bool,
    format: vk::SurfaceFormatKHR,
}

impl WindowTarget {
    pub fn new(surface: NativeSurface, size: [u32; 2], format: vk::SurfaceFormatKHR) -> Self {
        Self { swap: None, surface, size: size.map(|dimension| dimension.max(1)), dirty: true, format }
    }

    pub fn resize(&mut self, size: [u32; 2]) {
        if !size.contains(&0) && self.size != size {
            self.size = size;
            self.dirty = true;
        }
    }

    pub fn reset(&mut self, format: vk::SurfaceFormatKHR) {
        self.format = format;
        self.swap = None;
        self.dirty = true;
    }

    pub fn prepare(&mut self, device: &Rc<Device>) -> Result<(), RenderError> {
        if self.dirty {
            // Release any retired image and stop presenting before replacing the swapchain.
            self.swap = None;
            self.swap = Swapchain::new(device, &self.surface, self.size, self.format)?;
            self.dirty = self.swap.is_none();
        }
        Ok(())
    }

    pub fn acquire(&mut self, device: &Rc<Device>, frame: usize) -> Result<Option<Acquired>, RenderError> {
        for _ in 0..2 {
            let result = self.acquire_once(frame)?;
            if result.is_some() || !self.dirty || self.swap.is_none() {
                return Ok(result);
            }
            self.prepare(device)?;
        }
        Ok(None)
    }

    fn acquire_once(&mut self, frame: usize) -> Result<Option<Acquired>, RenderError> {
        let Some(swap) = &mut self.swap else { return Ok(None) };
        let acquire = swap.acquire[frame];
        let loader = swap.device.swapchain.as_ref().unwrap();
        // SAFETY: the frame fence was waited before reusing its acquisition semaphore.
        let (index, suboptimal) =
            match unsafe { loader.acquire_next_image(swap.raw, 16_000_000, acquire, vk::Fence::null()) } {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.dirty = true;
                    return Ok(None);
                }
                Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
        self.dirty |= suboptimal;
        swap.acquired = true;
        let image = &mut swap.images[index as usize];
        Ok(Some(Acquired {
            index,
            image: image.raw,
            view: image.view,
            previous: mem::replace(&mut image.layout, vk::ImageLayout::PRESENT_SRC_KHR),
            size: swap.size,
            acquire,
            present: image.present,
        }))
    }

    pub fn present(&mut self, index: u32) -> Result<(), RenderError> {
        let swap = self.swap.as_mut().unwrap();
        let device = &swap.device.raw;
        let image = &mut swap.images[index as usize];
        let waits = [image.present];
        let swaps = [swap.raw];
        let indices = [index];
        let present = vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&swaps).image_indices(&indices);
        let fences = [image.fence];
        let mut fence_info = vk::SwapchainPresentFenceInfoKHR::default().fences(&fences);
        let present = if swap.device.present_fences {
            if image.pending {
                // SAFETY: this fence belongs to the last successful presentation of this image.
                unsafe { device.wait_for_fences(&fences, true, u64::MAX) }?;
                // SAFETY: the last presentation fence has signalled and can be reused.
                unsafe { device.reset_fences(&fences) }?;
                image.pending = false;
            }
            // SAFETY: the enabled extension takes one live unsignalled fence for this swapchain.
            unsafe { present.extend(&mut fence_info) }
        } else {
            present
        };
        let loader = swap.device.swapchain.as_ref().unwrap();
        // SAFETY: the image was acquired and the rendering submission signals the wait semaphore.
        match unsafe { loader.queue_present(swap.device.queue, &present) } {
            Ok(suboptimal) => {
                image.pending = swap.device.present_fences;
                swap.acquired = false;
                self.dirty |= suboptimal;
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.dirty = true,
            Err(error) => {
                self.dirty = true;
                return Err(error.into());
            }
        }
        Ok(())
    }
}

pub struct Acquired {
    pub index: u32,
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub previous: vk::ImageLayout,
    pub size: vk::Extent2D,
    pub acquire: vk::Semaphore,
    pub present: vk::Semaphore,
}

struct SwapImage {
    raw: vk::Image,
    view: vk::ImageView,
    present: vk::Semaphore,
    layout: vk::ImageLayout,
    fence: vk::Fence,
    pending: bool,
}

struct Swapchain {
    device: Rc<Device>,
    raw: vk::SwapchainKHR,
    size: vk::Extent2D,
    images: Vec<SwapImage>,
    acquire: [vk::Semaphore; FRAMES],
    acquired: bool,
}

impl Swapchain {
    fn new(
        device: &Rc<Device>,
        surface: &NativeSurface,
        size: [u32; 2],
        format: vk::SurfaceFormatKHR,
    ) -> Result<Option<Self>, RenderError> {
        let loader = device.swapchain.as_ref().ok_or_else(|| RenderError::Gpu("presentation is disabled".into()))?;
        // SAFETY: the surface and physical device belong to the same live instance.
        let caps =
            unsafe { surface.vulkan.surface.get_physical_device_surface_capabilities(device.physical, surface.raw) }?;
        let size = if caps.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: size[0].clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: size[1].clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        } else {
            caps.current_extent
        };
        if size.width == 0 || size.height == 0 {
            return Ok(None);
        }
        let alpha = [
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|&alpha| caps.supported_composite_alpha.contains(alpha))
        .ok_or_else(|| RenderError::Gpu("surface has no supported compositing mode".into()))?;
        let count =
            (caps.min_image_count + 1).min(if caps.max_image_count == 0 { u32::MAX } else { caps.max_image_count });
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface.raw)
            .min_image_count(count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(size)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true);
        // SAFETY: all surface properties were queried from this physical device.
        let raw = unsafe { loader.create_swapchain(&info, None) }?;
        let mut swap = Self {
            device: Rc::clone(device),
            raw,
            size,
            images: Vec::new(),
            acquire: [vk::Semaphore::null(); FRAMES],
            acquired: false,
        };
        for semaphore in &mut swap.acquire {
            // SAFETY: the semaphore is created on the live swapchain device and owned by swap.
            *semaphore = unsafe { device.raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?;
        }
        // SAFETY: this swapchain belongs to the loader's live device.
        let images = unsafe { loader.get_swapchain_images(raw) }?;
        for raw in images {
            swap.images.push(SwapImage {
                raw,
                view: vk::ImageView::null(),
                present: vk::Semaphore::null(),
                layout: vk::ImageLayout::UNDEFINED,
                fence: vk::Fence::null(),
                pending: false,
            });
            let image = swap.images.last_mut().unwrap();
            image.view = view(&device.raw, raw, format.format)?;
            // SAFETY: the presentation semaphore belongs to this live device and is owned by swap.
            image.present = unsafe { device.raw.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?;
            if device.present_fences {
                // SAFETY: the new unsignalled fence is owned by this swapchain image.
                image.fence = unsafe { device.raw.create_fence(&vk::FenceCreateInfo::default(), None) }?;
            }
        }
        Ok(Some(swap))
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        let device = &self.device.raw;
        let fences =
            self.images.iter().filter(|image| image.pending).map(|image| image.fence).collect::<SmallVec<[_; 4]>>();
        // SAFETY: only fences from successful presentations are collected, and they are owned by this swapchain.
        let finished = fences.is_empty() || unsafe { device.wait_for_fences(&fences, true, u64::MAX) }.is_ok();
        if !self.device.present_fences || self.acquired || !finished {
            // Acquisition or presentation failure can leave work outside any present fence.
            let _idle = self.device.idle();
        }
        for image in &self.images {
            // SAFETY: the queue no longer uses this image's presentation fence.
            unsafe {
                device.destroy_fence(image.fence, None);
            }
            // SAFETY: the queue has stopped using the swapchain and its presentation semaphores.
            unsafe {
                device.destroy_semaphore(image.present, None);
            }
            // SAFETY: no pending submission can use the swapchain image view.
            unsafe {
                device.destroy_image_view(image.view, None);
            }
        }
        for &semaphore in &self.acquire {
            // SAFETY: all acquisitions using this semaphore have completed.
            unsafe {
                device.destroy_semaphore(semaphore, None);
            }
        }
        // SAFETY: the swapchain is idle and its image views have been destroyed.
        unsafe {
            self.device.swapchain.as_ref().unwrap().destroy_swapchain(self.raw, None);
        }
    }
}

pub fn surface_format(
    device: &Device,
    surface: &NativeSurface,
    preferred: Option<TextureFormat>,
) -> Result<(TextureFormat, vk::SurfaceFormatKHR), SetupError> {
    let loader = &surface.vulkan.surface;
    // SAFETY: the surface and physical device belong to the same live instance.
    if !unsafe { loader.get_physical_device_surface_support(device.physical, device.family, surface.raw) }? {
        return Err(SetupError::UnsupportedSurface);
    }
    // SAFETY: the surface and physical device belong to the same live instance.
    let formats = unsafe { loader.get_physical_device_surface_formats(device.physical, surface.raw) }?;
    for format in
        [TextureFormat::Bgra8Unorm, TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Srgb, TextureFormat::Rgba8Srgb]
    {
        if preferred.is_none_or(|preferred| preferred == format)
            && let Some(&surface_format) = formats.iter().find(|entry| entry.format == format.raw())
        {
            return Ok((format, surface_format));
        }
    }
    Err(SetupError::UnsupportedSurface)
}

slotmap::new_key_type! {
    /// Identifies a presentation surface owned by its renderer.
    pub struct SurfaceHandle;
}
