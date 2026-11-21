# Vulkan candidates for Isthmus

## Rendering

- **Try local reads on a tile-based GPU.** One Lares room sample matched pixel for pixel on the RTX 4090 and Lavapipe. Warm runs showed no clear speedup. Check whether keeping visibility and material data on chip helps on mobile hardware. [Vulkan local reads](https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_dynamic_rendering_local_read.html)
- **Revisit GPU addresses for frame buffers when Rust-GPU supports them directly.** A prototype passed addresses and explicit word counts to real Cantus and Lares shaders on the RTX 4090 and Lavapipe. It needed a SPIR-V rewrite and showed no clear speedup in Cantus. [Buffer array lengths](https://docs.vulkan.org/guide/latest/buffer_array_length.html)

## Make Isthmus smaller

- **Keep one way to write a shader draw.** Try a single declaration that names the shader, declares its typed captures, and provides its geometry and paint code. Generate the host draw, SPIR-V entry points, and capture layout from that declaration. Aim to retire the `Shader` and `Shaped` builders, runtime name lookup, and the build scanner that reconstructs the method chain. Keep inline Rust, reusable helpers, and the choice of cheap CPU bounds or a shape evaluated on both CPU and GPU.
- **Make a pass the unit of recording.** Let a frame record an ordered list of named passes, each with a target and its own draws. Allow two passes to write the same target in sequence. Lares could reuse its material image for geometry without creating a second surface handle. Use that same pass list for submission, validation, and timing.
- **Give each render target one identity.** An offscreen target could provide a sampleable view of the same resource instead of returning a surface handle and a separate image identity. Keep one creation path for offscreen targets with a default or explicit format. Window setup can stay in the platform adapter while both target kinds use the same pass API.
- **Keep the draw arena private.** Give custom captures a small interface for sending a value, a slice, or reusable data. Let Isthmus choose storage, offsets, and reuse internally. Test this with text outlines, which currently need direct arena access, before hiding `Arena`, `Record`, and the packed-word details from ordinary drawing code.
- **Generate one description per shader.** Use the same generated description for pipeline creation, capture layout, build-time resource checks, and profiling names. Keep Vulkan objects, raw handles, and generated shader entry points out of the everyday API, with a small explicit extension interface for integrations that need them.

Try the draw declaration on Cantus text and the pass API on Lares’s deferred renderer. Count the public types and methods removed, check that each old implementation has been retired, and compare pixels and frame timings before migrating the rest.

## Prevent runtime errors

- **Generate typed shader handles.** Keep readable shader names for logs and captures. Have the build generate handles that Rust can check, so a typo or the wrong capture types fail at compile time.
- **Give render targets a format and a role.** Carry the format in the handle where it is known. Distinguish sampleable images from presentation targets, and prevent a pass from sampling its own output through the pass API.
- **Validate pass inputs up front.** Check that a pass uses images from its renderer and that its inputs have been produced before recording GPU commands. Keep previous-frame inputs explicit for effects that need them.
- **Make invalid sizes hard to construct.** Use checked image sizes and buffer ranges. Reject unsupported sizes and resource counts before allocating or submitting a frame.
- **Tie resource lifetime to submission.** Let the recorded submission own what the GPU still uses. Keep retired surfaces and resources alive until their completion signal, and invalidate stale handles during recovery.

## Shader tooling for later

- **Check shader resources at build time.** Key the report by shader name and source location. Read draw record bytes, bytes per slice element, and texture capture counts from the generated layout. Flag large fixed local arrays in SPIR-V. Fail the build for proven budget violations. When a slice length is only known at runtime, show the cost per element and allow an explicit maximum that the host checks before upload.
- **Check driver compilation without rendering a frame.** On a selected GPU and driver, compile the actual pipelines and query register and spill statistics when available. Mark missing figures as unavailable. The driver can put locals in registers, optimize them away, or spill them into scratch memory. [Vulkan pipeline statistics](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetPipelineExecutableStatisticsKHR.html)
- **Capture actual frame usage when needed.** Show draw, slice, and shared data uploads, what got reused, which textures were referenced, the largest allocations, and the current GPU memory budget. [Vulkan memory budgets](https://docs.vulkan.org/spec/latest/chapters/memory.html)
- **Make workgroup memory explicit.** If Isthmus adds compute shaders, give shared workgroup memory its own type. The shader author decides when the group synchronizes. Show the bytes each group shares and the device limit. [Vulkan device limits](https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceLimits.html)
- **Inspect values at a pixel.** In a debug capture, click a pixel to see UV, coverage, material ID, or lighting. Show a value across the image as a heatmap to spot problems such as a lighting seam.
- **Preview changes live.** Rebuild the changed shader, keep the last working version visible, and show compiler errors beside the source.
- **Add editor hovers last.** Reuse the CLI report beside rust-analyzer’s type hover. A capture in `.with(...)` could show `sampled texture · 4 B texture index per draw instance`. Hover over the shader name to see the total draw record size and open the full report.
