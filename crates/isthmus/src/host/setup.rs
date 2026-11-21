use super::error::{RenderError, SetupError};
use ash::{
    Entry, Instance, ext, khr,
    vk::{self, TaggedStructure as _},
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    cell::Cell,
    env,
    ffi::{CStr, CString},
    ptr,
    rc::Rc,
};

/// A native window kept alive for the lifetime of its Vulkan surface.
pub trait Window: HasWindowHandle + HasDisplayHandle + 'static {}
impl<T: HasWindowHandle + HasDisplayHandle + 'static> Window for T {}

pub struct Vulkan {
    entry: Entry,
    pub instance: Instance,
    pub surface: khr::surface::Instance,
    platform: Option<&'static CStr>,
    debug_labels: bool,
    surface_maintenance: bool,
}

impl Vulkan {
    pub fn new(window: Option<&dyn Window>) -> Result<Rc<Self>, SetupError> {
        let platform = match window.map(|window| window.window_handle().map(|handle| handle.as_raw())).transpose() {
            Ok(Some(RawWindowHandle::Wayland(_))) => Some(khr::wayland_surface::NAME),
            Ok(Some(RawWindowHandle::AndroidNdk(_))) => Some(khr::android_surface::NAME),
            Ok(None) => None,
            _ => return Err(SetupError::UnsupportedSurface),
        };
        // SAFETY: ash retains the Vulkan loader for the lifetime of Entry.
        let entry = unsafe { Entry::load() }.map_err(|error| SetupError::Vulkan(error.to_string()))?;
        let mut extensions = Vec::new();
        if let Some(platform) = platform {
            extensions.extend([khr::surface::NAME.as_ptr(), platform.as_ptr()]);
        }
        // SAFETY: the loaded Vulkan entry is live and no layer filter is required.
        let available = unsafe { entry.enumerate_instance_extension_properties(None) }?;
        let supports = |name| available.iter().any(|ext| ext.extension_name_as_c_str() == Ok(name));
        let debug_labels = env::var("ISTHMUS_GPU_LABELS").as_deref() == Ok("1") && supports(ext::debug_utils::NAME);
        if debug_labels {
            extensions.push(ext::debug_utils::NAME.as_ptr());
        }
        let surface_maintenance = platform.is_some()
            && supports(khr::get_surface_capabilities2::NAME)
            && supports(khr::surface_maintenance1::NAME);
        if surface_maintenance {
            extensions
                .extend([khr::get_surface_capabilities2::NAME.as_ptr(), khr::surface_maintenance1::NAME.as_ptr()]);
        }
        let application = vk::ApplicationInfo::default().application_name(c"Isthmus").api_version(vk::API_VERSION_1_4);
        let info =
            vk::InstanceCreateInfo::default().application_info(&application).enabled_extension_names(&extensions);
        // SAFETY: the instance create info refers to live static extension names and the application info.
        let instance = unsafe { entry.create_instance(&info, None) }?;
        let surface = khr::surface::Instance::load(&entry, &instance);
        Ok(Rc::new(Self { entry, instance, surface, platform, debug_labels, surface_maintenance }))
    }

    pub fn create_surface(self: &Rc<Self>, window: impl Window) -> Result<NativeSurface, SetupError> {
        let display = window.display_handle().map_err(|error| SetupError::Vulkan(error.to_string()))?.as_raw();
        let handle = window.window_handle().map_err(|error| SetupError::Vulkan(error.to_string()))?.as_raw();
        let raw = match (display, handle) {
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle))
                if self.platform == Some(khr::wayland_surface::NAME) =>
            {
                // SAFETY: the owned window keeps both raw Wayland handles alive.
                unsafe {
                    khr::wayland_surface::Instance::load(&self.entry, &self.instance).create_wayland_surface(
                        &vk::WaylandSurfaceCreateInfoKHR::default()
                            .display(display.display.as_ptr().cast())
                            .surface(handle.surface.as_ptr().cast()),
                        None,
                    )
                }?
            }
            (RawDisplayHandle::Android(_), RawWindowHandle::AndroidNdk(handle))
                if self.platform == Some(khr::android_surface::NAME) =>
            {
                // SAFETY: the owned Android window keeps the native handle alive.
                unsafe {
                    khr::android_surface::Instance::load(&self.entry, &self.instance).create_android_surface(
                        &vk::AndroidSurfaceCreateInfoKHR::default().window(handle.a_native_window.as_ptr().cast()),
                        None,
                    )
                }?
            }
            _ => return Err(SetupError::UnsupportedSurface),
        };
        Ok(NativeSurface { raw, vulkan: Rc::clone(self), _window: Box::new(window) })
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        // SAFETY: every device and surface retains its own reference to this instance.
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

pub struct NativeSurface {
    pub raw: vk::SurfaceKHR,
    pub vulkan: Rc<Vulkan>,
    _window: Box<dyn Window>,
}

impl Drop for NativeSurface {
    fn drop(&mut self) {
        // SAFETY: the owning WindowTarget drops its swapchain before the surface.
        unsafe {
            self.vulkan.surface.destroy_surface(self.raw, None);
        }
    }
}

pub struct Device {
    pub vulkan: Rc<Vulkan>,
    pub physical: vk::PhysicalDevice,
    pub properties: vk::PhysicalDeviceProperties,
    pub raw: ash::Device,
    pub queue: vk::Queue,
    pub family: u32,
    pub max_images: u32,
    pub heap: ext::descriptor_heap::Device,
    pub heap_properties: vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>,
    pub swapchain: Option<khr::swapchain::Device>,
    pub present_fences: bool,
    debug: Option<ext::debug_utils::Device>,
    fault: Option<ext::device_fault::Device>,
    fault_reported: Cell<bool>,
}

impl Device {
    pub fn new(vulkan: Rc<Vulkan>, surface: Option<vk::SurfaceKHR>, images: usize) -> Result<Rc<Self>, SetupError> {
        let instance = &vulkan.instance;
        // SAFETY: the instance is live.
        let devices = unsafe { instance.enumerate_physical_devices() }?;
        let mut candidates = Vec::new();
        for physical in devices {
            // SAFETY: enumeration returned this physical device from the live instance.
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            if properties.api_version < vk::API_VERSION_1_4 {
                continue;
            }
            // SAFETY: this physical device belongs to the live instance.
            let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }?;
            let supports = |name| extensions.iter().any(|extension| extension.extension_name_as_c_str() == Ok(name));
            if !supports(ext::descriptor_heap::NAME) || (surface.is_some() && !supports(khr::swapchain::NAME)) {
                continue;
            }
            let mut indexing = vk::PhysicalDeviceVulkan12Features::default();
            let mut heap_features = vk::PhysicalDeviceDescriptorHeapFeaturesEXT::default();
            // SAFETY: the indexing features are initialized and live through the query.
            let features = unsafe { vk::PhysicalDeviceFeatures2::default().extend(&mut indexing) };
            // SAFETY: the heap features start with an empty chain and live through the query.
            let mut features = unsafe { features.extend(&mut heap_features) };
            // SAFETY: the chain and physical device are live.
            unsafe {
                instance.get_physical_device_features2(physical, &mut features);
            }
            if indexing.runtime_descriptor_array == 0
                || indexing.shader_sampled_image_array_non_uniform_indexing == 0
                || heap_features.descriptor_heap == 0
            {
                continue;
            }
            let mut heap_properties = vk::PhysicalDeviceDescriptorHeapPropertiesEXT::default();
            // SAFETY: the output properties are initialized and live through the query.
            let mut output = unsafe { vk::PhysicalDeviceProperties2::default().extend(&mut heap_properties) };
            // SAFETY: the physical device and output chain are live.
            unsafe {
                instance.get_physical_device_properties2(physical, &mut output);
            }
            let max_images = super::heap::Layout::max_images(&heap_properties);
            if max_images < (images as u32).max(1) {
                continue;
            }
            // SAFETY: this physical device belongs to the live instance.
            let queues = unsafe { instance.get_physical_device_queue_family_properties(physical) };
            for (family, queue) in queues.iter().enumerate() {
                let family = family as u32;
                if !queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) || queue.queue_count == 0 {
                    continue;
                }
                // SAFETY: the surface and queue family belong to this live instance.
                if let Some(surface) = surface
                    // SAFETY: this surface and queue family belong to the live instance.
                    && !unsafe { vulkan.surface.get_physical_device_surface_support(physical, family, surface) }?
                {
                    continue;
                }
                candidates.push((physical, properties, family, max_images, heap_properties, extensions));
                break;
            }
        }
        let (physical, properties, family, max_images, heap_properties, available) = candidates
            .into_iter()
            .max_by_key(|(_, properties, ..)| match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                vk::PhysicalDeviceType::CPU => 0,
                _ => 1,
            })
            .ok_or_else(|| {
                SetupError::Vulkan("need Vulkan 1.4 with descriptor heaps and sampled-image descriptor indexing".into())
            })?;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
        let mut extensions = vec![ext::descriptor_heap::NAME.as_ptr()];
        if surface.is_some() {
            extensions.push(khr::swapchain::NAME.as_ptr());
        }
        let supports = |name| available.iter().any(|ext| ext.extension_name_as_c_str() == Ok(name));
        let mut presentation = vk::PhysicalDeviceSwapchainMaintenance1FeaturesKHR::default();
        if vulkan.surface_maintenance && surface.is_some() && supports(khr::swapchain_maintenance1::NAME) {
            // SAFETY: this supported extension's feature struct is initialized and live.
            let mut query = unsafe { vk::PhysicalDeviceFeatures2::default().extend(&mut presentation) };
            // SAFETY: the physical device and feature chain are live.
            unsafe {
                instance.get_physical_device_features2(physical, &mut query);
            }
        }
        let present_fences = presentation.swapchain_maintenance1 == vk::TRUE;
        if present_fences {
            extensions.push(khr::swapchain_maintenance1::NAME.as_ptr());
        }
        let mut fault = vk::PhysicalDeviceFaultFeaturesEXT::default();
        if supports(ext::device_fault::NAME) {
            // SAFETY: this supported extension's feature struct is initialized and live.
            let mut query = unsafe { vk::PhysicalDeviceFeatures2::default().extend(&mut fault) };
            // SAFETY: the physical device and feature chain are live.
            unsafe {
                instance.get_physical_device_features2(physical, &mut query);
            }
        }
        let fault_reports = fault.device_fault == vk::TRUE;
        if fault_reports {
            extensions.push(ext::device_fault::NAME.as_ptr());
        }
        let mut fault = vk::PhysicalDeviceFaultFeaturesEXT::default().device_fault(fault_reports);
        let mut modern = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true).synchronization2(true);
        let mut indexing = vk::PhysicalDeviceVulkan12Features::default()
            .runtime_descriptor_array(true)
            .shader_sampled_image_array_non_uniform_indexing(true)
            .buffer_device_address(true)
            .vulkan_memory_model(true);
        let mut core = vk::PhysicalDeviceVulkan14Features::default().maintenance5(true);
        let mut heap_features = vk::PhysicalDeviceDescriptorHeapFeaturesEXT::default().descriptor_heap(true);
        let info = vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_extension_names(&extensions);
        // SAFETY: all feature structs start with empty chains and live through device creation.
        let info = unsafe { info.extend(&mut modern) };
        // SAFETY: the Vulkan 1.2 feature struct extends this device create info.
        let info = unsafe { info.extend(&mut indexing) };
        // SAFETY: the Vulkan 1.4 feature struct extends this device create info.
        let info = unsafe { info.extend(&mut core) };
        // SAFETY: the heap feature struct extends this device create info.
        let info = unsafe { info.extend(&mut heap_features) };
        let info = if present_fences {
            // SAFETY: the queried presentation extension is enabled and its feature struct is live.
            unsafe { info.extend(&mut presentation) }
        } else {
            info
        };
        let info = if fault_reports {
            // SAFETY: the fault reporting extension is enabled and its feature struct is live.
            unsafe { info.extend(&mut fault) }
        } else {
            info
        };
        // SAFETY: the instance and selected device are live, and the requested features are supported.
        let raw = unsafe { instance.create_device(physical, &info, None) }?;
        // SAFETY: queue zero was requested in this family.
        let queue = unsafe { raw.get_device_queue(family, 0) };
        let heap = ext::descriptor_heap::Device::load(instance, &raw);
        let swapchain = surface.map(|_| khr::swapchain::Device::load(instance, &raw));
        let debug = vulkan.debug_labels.then(|| ext::debug_utils::Device::load(instance, &raw));
        let fault = fault_reports.then(|| ext::device_fault::Device::load(instance, &raw));
        tracing::debug!(
            present_fences,
            fault_reports,
            debug_labels = vulkan.debug_labels,
            "Vulkan diagnostics and presentation"
        );
        Ok(Rc::new(Self {
            vulkan,
            physical,
            properties,
            raw,
            queue,
            family,
            max_images,
            heap,
            heap_properties,
            swapchain,
            present_fences,
            debug,
            fault,
            fault_reported: Cell::new(false),
        }))
    }

    pub fn label(&self, cmd: vk::CommandBuffer, name: &str) -> bool {
        let Some(debug) = &self.debug else { return false };
        let Ok(name) = CString::new(name) else { return false };
        let label = vk::DebugUtilsLabelEXT::default().label_name(&name);
        // SAFETY: this enabled extension is recording a region on the live command buffer.
        unsafe {
            debug.cmd_begin_debug_utils_label(cmd, &label);
        }
        true
    }

    pub fn end_label(&self, cmd: vk::CommandBuffer) {
        if let Some(debug) = &self.debug {
            // SAFETY: callers end only a region that was opened by label on this command buffer.
            unsafe {
                debug.cmd_end_debug_utils_label(cmd);
            }
        }
    }

    pub fn report_fault(&self, error: &RenderError) {
        if !matches!(error, RenderError::DeviceLost(_)) || self.fault_reported.replace(true) {
            return;
        }
        let Some(loader) = &self.fault else { return };
        let query = loader.fp().get_device_fault_info_ext;
        let mut counts = vk::DeviceFaultCountsEXT::default();
        // SAFETY: the device is lost and the enabled extension accepts a counts-only query.
        let result = unsafe { query(self.raw.handle(), &raw mut counts, ptr::null_mut()) };
        if result != vk::Result::SUCCESS {
            tracing::warn!(%result, "Vulkan device fault report unavailable");
            return;
        }
        counts.address_info_count = counts.address_info_count.min(32);
        counts.vendor_info_count = counts.vendor_info_count.min(8);
        let mut addresses = vec![vk::DeviceFaultAddressInfoEXT::default(); counts.address_info_count as usize];
        let mut vendors = vec![vk::DeviceFaultVendorInfoEXT::default(); counts.vendor_info_count as usize];
        let mut report = vk::DeviceFaultInfoEXT {
            p_address_infos: addresses.as_mut_ptr(),
            p_vendor_infos: vendors.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: the arrays are live and have the capacities requested in counts.
        let result = unsafe { query(self.raw.handle(), &raw mut counts, &raw mut report) };
        if matches!(result, vk::Result::SUCCESS | vk::Result::INCOMPLETE) {
            let description = report.description_as_c_str().unwrap_or(c"Unknown");
            tracing::error!(
                description = %description.to_string_lossy(),
                addresses = ?&addresses[..counts.address_info_count as usize],
                vendor = ?&vendors[..counts.vendor_info_count as usize],
                "Vulkan device fault"
            );
        } else {
            tracing::warn!(%result, "Could not read Vulkan device fault report");
        }
    }

    pub fn idle(&self) -> Result<(), RenderError> {
        // SAFETY: the device and queue are owned and used on this thread.
        Ok(unsafe { self.raw.device_wait_idle() }?)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: resources hold references, and Gpu waits for its submissions before releasing them.
        unsafe {
            self.raw.destroy_device(None);
        }
    }
}
