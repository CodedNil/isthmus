# Rendering tools

Lares uses procedural SDF geometry. Rust examples provide offline renders,
image differences and CPU transport benchmarks:

```sh
LARES_TIMESTAMP=1782043200 LARES_SHADOW_RESOLUTION=512 cargo run -p lares --release --example capture -- crates/lares/assets/references 1200 900 all
cargo run -p lares --release --example gallery
cargo run -p lares --release --example compare -- reference.webp candidate.png /tmp/error.png
cargo run -p lares --release --example lighting_bench
cargo run -p lares --example bake_lighting
cargo test -p lares -p isthmus --lib
```

[Comparison sheet](../assets/references/comparison.webp) contains photographs
on the left and paired current renders on the right. Each base render is one
1200×1800 image: **evening lights on above daytime lights off**. The gallery
uses the camera list in `examples/support/views.rs`, excluding plan and overhead.
There are 18 photographic references, including three bedroom views and two
views each of the bathroom and ensuite. Dining and kitchen-cabinet have their
own cameras. Hall and the two storage views have no photographs and leave the
left column blank. Plan and overhead retain stacked renders outside the gallery.

Capture always renders both setups. Evening uses 21:00 UTC on the chosen date,
with fixtures at 100%; daylight uses noon with fixtures off. Each setup meters
exposure and bounded camera white balance independently. `LARES_TIMESTAMP`
overrides the daytime timestamp (and sets the date for the evening render);
the optional UTC-hour argument changes the daytime hour. The command above
fixes the date to June 21: evening is 22:00 BST and daytime is 13:00 BST.
Original photographs are preserved. Captures use four AA samples.
Camera metering includes diffuse GI and visible direct sources, with exposure
allowed down to EV100 3 for dim interiors. Bounded diagonal white balance is
applied before tone mapping; it does not change source lumens or cached transport.
For a direct-light diagnostic with exposure held at its normal value, use
`LARES_CAPTURE_GI=off` and a temporary output directory.
Captures use half the logical CPUs, capped at eight.

## Capture cameras

Camera positions, targets and fields of view are defined directly in
`examples/support/views.rs`. Cameras always use world-up, with no roll.
Edit those definitions to adjust the standard captures.

Use `capture -- crates/lares/assets/references 1200 900 matched` to update all
photographed cameras in place, or replace `matched` with a single view name or comma-separated names.
Use `all` for full room coverage, then regenerate the comparison sheet.

### Measured alignment

`cargo run -p lares --example align -- all /tmp/lares-alignment` fits cameras
against architectural points and line segments measured in the reference photos.
Measurements live in `examples/support/measurements.rs`, in original photo pixels
and rendered world metres; they use inner wall faces, including wall thickness.
Furniture, reflections and open door leaves do not constrain the fit. Tile caps
with uncertain heights constrain direction only. Close crops without enough
metric landmarks retain their existing camera position.

The tool writes measured/projected edge overlays and a report with pixel errors,
line angles and a separate reference-roll diagnostic. Capture fits keep roll zero.
`cargo run -p lares --example align -- entrance /tmp/lares-alignment --write`
writes the fitted pose directly into `views.rs`; there are no runtime adjustments.
Inspect the overlays and standard captures before accepting a fit: opening sizes,
missing alcoves and reference tilt can leave residual architectural differences.

## Browser GPU checks

The Rust examples run shared renderer code on the CPU. Their elapsed times
are not GPU frame times. The small JavaScript harness measures the deployed
WebGPU renderer, including timestamps, queue depth, scheduling and accumulation.

```sh
cargo build --release -Zbuild-std=std,panic_abort --lib -p lares --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/lares/assets/web target/wasm32-unknown-unknown/release/lares.wasm
LARES_BENCH_CLOCK=running node crates/lares/tools/run-benchmark.mjs /tmp/lares-clock crates/lares/assets 2560 1600
LARES_BENCH_MOTION=1 node crates/lares/tools/run-benchmark.mjs /tmp/lares-motion crates/lares/assets 2560 1600
```

Requires Node 22+ and Chromium. The runner owns a separate browser profile
and temporary server and stops both on completion, failure, interrupt or a
60-second deadline (`LARES_BENCH_DEADLINE_MS` can extend a cold-start run).
It writes JSON metrics and a converged GPU screenshot.
The JSON includes startup stage timestamps, GPU frame cost and synchronous
browser light-control latency. `LARES_BENCH_MS` controls the
measurement duration (default 10000). Run after CPU captures/builds finish.
The running-clock check fails if settled accumulation resets as time advances.
Optional `LARES_BENCH_TARGET=x,y` pans to a world coordinate and
`LARES_BENCH_ZOOM=17` applies that many inward wheel steps before settling.
For a pile-heavy rug view, use target `-4.25,1.38` and zoom `17`.

## Renderer conventions

One linear lighting unit represents 100 lux incident or 100 cd/m² radiance.
GU10 fixtures use TRÅDFRI colour/white-spectrum 345 lm and a 40° beam, set to
4000 K cool white at 100% in the default scene;
the woven lamp assumes an unidentified 470 lm, 2700 K bulb. Clear-sky daylight uses
Ineichen/Perez with assumed turbidity 3. Solar motion uses date, coordinates
and north bearing. Weather and measured LED spectra are not modelled.
Exterior ground visible through windows uses an analytic 20% diffuse reflectance assumption, including sunlight;
no exterior probes or geometry are baked.

`Scene::set_light_state`, `set_light_states` and `set_light_color` update grouped
fixture output and bounced light, then invalidate accumulated lighting samples.
The shadow atlas depends on geometry and can be reused for brightness/RGB edits.
Fixture changes blend solved, eight-bounce unit responses for the 11 control
groups. The browser exports `set_light_state(entity, brightness)` and
`set_light_color(entity, linear_r, linear_g, linear_b)` from `web/lares.js`;
Home Assistant and UI events can call them without rebaking geometry.
`lighting_bench` reports first and repeated update costs separately from GPU time.

The native server checks `assets/baked/lighting.bin` and `visibility.bin` before
serving. Unchanged geometry is a fast header check; camera, UI and server edits
do not invalidate these assets. Geometry/SDF/transport edits rebuild them once.
`bake_lighting` runs the same check explicitly. The assets contain losslessly
compressed final fixture responses and the full 512-pixel SDF visibility atlas.
The browser fetches both in parallel with WASM loading. It no longer compiles a
shadow-bake shader or traces the atlas during startup. GPU lamp visibility and
CPU captures use the same packed depth moments and filter.

Runtime GI has one final irradiance field: solved fixture responses plus the
current eight-bounce daylight solution. Static interpolation weights are cached;
the daylight solver runs in bounded batches and publishes only complete results.
The temporary first-bounce fixture bank exists only while authoring a new bake.
Receiver-side filtering rejects samples behind opaque surfaces, including when
surface charts fall back to the volume at tile edges and wall junctions.

Static SDF visibility maps filter fixture shadows. Hybrid diffuse GI uses 20 cm
samples projected onto the SDF surfaces of architecture and the lounge footstool,
backed by a 50 cm indoor directional volume for foliage, small
objects, layered sofa cushions and cabinet fronts and gaps in projected coverage. There are no authored UVs or meshes.
Surface samples retain the cosine integral and its normal derivative in their
existing directional fields, so floor grain and bevel normals also shade under
bounced light. The bake uses 256 directions and eight finite diffuse bounces. Surface-to-surface
transport queries the projected caches; unresolved receivers use the volume.
Architectural samples share a world-aligned lattice and overlap across coplanar
construction joints. Position-based ray rotations keep these overlapping samples consistent and avoid coherent window-sampling bands. Planar samples integrate their footprint instead of stretching tiny occluders
across a whole texel. Footprint offsets and filter connections require an unobstructed SDF segment, so neither can cross a thin partition. Geometry-compatible surface filtering reduces source
variance. The final bounced field receives two geometry-constrained filtering passes.
Projection weights blend continuously across rounded surfaces. Both projected and volume reconstruction enforce architectural clearance bounds, preventing interpolation across partitions. Volume queries additionally use receiver-normal weights and six-cone depth moments,
with a continuous cubic fallback where nearby sites are missing.

Short-range ambient contact visibility traces one cosine-weighted SDF ray per
existing lighting sample against solid furnishings in a static 40 cm cell index.
The four-sample sequence covers the receiver hemisphere. Rays are limited to
12 cm and 16 conservative steps, with direct intersections for boxes; unresolved grazing rays stay visible. Actual intersections, rather than conservative SDF distance deficits,
control contact darkening. Architectural walls are excluded because GI already
integrates their occlusion. It affects indirect light only. Fine foliage keeps
its material visibility; rug contacts use the shared upward nap normal.
This remains a bounded contact approximation, not unbiased path tracing.

Only actual indoor ray hits allocate cached transport paths; the fixed surface
interpolation links are cached too, avoiding repeated projection work during
relighting. Covered surface samples
and exterior faces do not trace or relight. Geometry changes rebuild these
caches; brightness/colour controls recompose linear source transport. The Rust
`lighting_bench` reports sample counts, GPU cache storage and update costs.
This is a static, projected surface cache with a volumetric fallback, rather
than a full camera-driven surfel system. Deep concavities, layered fronts and fine geometry
still rely on the background field.

This design draws on visibility-aware reconstruction in
[NVIDIA's DDGI](https://research.nvidia.com/publication/2019-05_dynamic-diffuse-global-illumination-ray-traced-irradiance-fields)
and the complementary surface/probe caches discussed in
[Frostbite's 2024 GIBS presentation](https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf).
A full GIBS implementation would also need view-driven surfel spawning,
coverage management and dynamic acceleration structures; replacing a grid alone
does not provide those capabilities. The projected cache suits this static SDF
house while retaining bounded CPU daylight updates and cheap GPU gathers.

Shader compilation uses assertion-free numeric clamps for validated bounds,
and the shader extractor lowers integer range loops to counters. This preserves
CPU Rust behaviour while preventing iterator/clamp panic paths from forcing
Rust-GPU to duplicate large SDF dispatch functions. The compiler lowering is
checked against native Rust across nested loops, labels, continues, side effects
and inclusive integer limits.

Materials include procedural normal gradients, roughness, metallic Fresnel,
sheen and clear coat. The rug uses one procedural SDF yarn field across its
entire pile, with varied lean/height, a gently curved backing and broad nap
variation. There is no separate edge fringe. Granite and glazed kitchen tiles
trace one rough reflection ray into the SDF scene, using mean reflected finishes
and cached GI; other materials use an analytic reflection environment scaled by
local GI. The BÖJA lamp is 40 cm high and 20 cm wide; the STARKVIND table is
55 cm high and 54 cm wide, matching IKEA's product dimensions.

Progressive rendering converges all native pixels to four AA samples. Large
views interleave columns over eight updates. Motion targets 30 updates/s with
an adaptive ray-traced resolution; native resolution and all four samples return
when motion stops. Resizing targets discards incompatible history. A small
edge filter softens initial samples and disappears at convergence. Interactive
exposure/white balance are metered once at startup and locked for the session:
panning and light controls never change the display transform. Offline photo
captures retain independent view metering. Settled views use an 8 Hz ceiling and
20% GPU duty budget. Hidden views stop submitting and queue depth is limited to one. See [rendering contracts](../src/render/tests/README.md)
for the automated checks.
