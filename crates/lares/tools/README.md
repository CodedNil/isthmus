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
`bake_lighting` runs the same check explicitly. The lighting asset contains lossless receiver geometry and compact solved
fixture/daylight responses. Twelve signed diffuse-light coefficients share one
scale in 16 bytes (at most 0.4% peak-coefficient quantization error). Sky and
exterior ground are solved once; bounced sunlight interpolates a world-space
65-direction hemisphere table. The sun's actual direction, direct illumination,
SDF geometry, materials and primary shadows are unchanged. Eight diffuse bounces
are solved offline for every basis field. Angular interpolation can soften or
shift bounced-sun patches; it does not interpolate direct sun shadows.
The runtime holds no ray-hit bank, volume transport stencils or iterative bounce
buffers. Startup and daylight changes only blend solved fields, as fixture
controls already do. Geometry is decoded in bounded byte-plane chunks.
The separate visibility asset retains the full 512-pixel SDF atlas.
The browser fetches both in parallel with WASM loading. It no longer compiles a
shadow-bake shader or traces the atlas during startup. GPU lamp visibility and
CPU captures use the same packed raw depths and percentage-closer visibility filter.

Runtime GI has one final irradiance field: solved fixture responses plus the
interpolated eight-bounce daylight solution. Daylight changes publish a complete
field atomically without tracing or spatial iterations.
The temporary first-bounce fixture bank exists only while authoring a new bake.
Receiver-side filtering rejects samples behind opaque surfaces, including when
surface charts fall back to the volume at tile edges and wall junctions.

Static SDF visibility maps filter fixture shadows. Hybrid diffuse GI uses 10 cm floor samples and 20 cm
samples projected onto the SDF surfaces of other architecture, thin furniture shelves and the lounge footstool,
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

Short-range ambient contact visibility averages four fixed cosine-weighted SDF rays in every
lighting sample against solid furnishings in a static 40 cm cell index.
The same hemisphere average is available immediately after camera movement. Rays are limited to
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

Interactive rendering separates primary intersections, surface visibility,
material/GI evaluation and material-family finishing. Two reusable RGBA16Float
targets hold packed intermediate data; an alias lets primary intersections reuse
the material target before its final contents are needed. Surface normals retain
four tetrahedral SDF samples, with a uniform loop count to prevent drivers from
inlining four copies of the complete furniture distance evaluator.

Materials include procedural normal gradients, roughness, metallic Fresnel,
sheen and clear coat. The rug uses one procedural SDF yarn field across its
entire pile, with varied lean/height, a gently curved backing and broad nap
variation. There is no separate edge fringe. Interactive glossy reflections use
the same locally occluded GI radiance estimate as diffuse shading, avoiding
another copy of the house tracer in the material shaders. There is no invented
window lobe or brightness-dependent second occlusion factor; diffuse, metal and
clear-coat indirect light scale linearly with the solved field. This inexpensive
isotropic approximation does not reproduce detailed reflected objects. Offline reference rendering retains
traced stone and tile reflections. The BÖJA lamp is 40 cm high and 20 cm wide; the STARKVIND table is
55 cm high and 54 cm wide, matching IKEA's product dimensions.

Progressive rendering converges all native pixels to four AA samples. Large
views interleave columns over eight updates. Motion follows the display refresh at native resolution; all four samples converge
when motion stops. Resizing targets discards incompatible history. A small
edge filter softens initial samples and disappears at convergence. Interactive
exposure/white balance are metered once at startup and locked for the session:
panning and light controls never change the display transform. Offline photo
captures retain independent view metering. Settled views use an 8 Hz ceiling and
20% GPU duty budget. Hidden views stop submitting and queue depth is limited to one. See [rendering contracts](../src/render/tests/README.md)
for the automated checks.

The browser reports its selected adapter and rejects CPU software fallbacks
before scene preparation. SwiftShader is not a hardware GPU measurement; a
GPU driver/library mismatch must be resolved before benchmarking hardware.
`LARES_BENCH_STARTUP_MS` extends the readiness deadline independently of
`LARES_BENCH_DEADLINE_MS`, which bounds the entire browser run. Device loss
fails the benchmark instead of counting a failed submission as a rendered frame.

`cargo run -p lares --example startup_bench` separates CPU startup costs from
GPU compilation and reports retained transport storage. The native authoring
solver still traces full-quality transport when producing new lighting assets.

## Live Shelly presence

The native server continuously receives `ws://192.168.0.133/rpc` and relays
snapshots to client WebUIs at `/api/presence`. It keeps one upstream connection
open independently of browser viewers and renews `Presence.LiveTrack` every
40 seconds. This same server-side feed is available for future presence-driven
lighting. Current integration does not control lights or change sensor settings.
Five seconds without fresh tracking clears stale estimates, with reconnection.

Coordinates and the alignment live in Rust. After tilting the sensor, desk
`(1.86, 4.88)`, kitchen-centre `(-2.95, 2.45)`, and entrance-corridor intersection
`(-0.69, 6.07)` measurements define a three-point affine transform. The desk
anchor is 40 cm toward the keyboard from the modelled chair. The kitchen anchor
is its authored room centre; the hall anchor is the centre of the corridor where
it meets the open lounge, as identified in the reference crop. Coefficients are
solved at compile time. Measurement medians and target definitions are saved in
`presence-calibration.json`; there are no alignment controls or browser-local
calibration, and the live service does not write position history to disk.

Ghosts are translucent 3D SDF shells with a world-space uncertainty ring, without
an inferred pose or facing direction. The scene stores surface height in its
sample alpha channel, allowing the small ghost SDF pass to respect walls and
furniture without retracing the house. Ghost updates preserve scene AA history;
only the resolve/ghost pass needs to run at interactive rate. Camera gestures
keep a fixed full-resolution sample grid, anchor movement to the original grab
point, and overlap the next display refresh with GPU completion. The WebSocket
client is Rust/WASM, with no JavaScript/CSS/SVG presence drawing or added UI.

`LARES_PORT=8130 cargo run -p lares` starts an isolated server for testing.
`LARES_BENCH_PRESENCE=1` checks live world-space ghosts with a native server.
Cold driver shader compilation remains slower than cached loads. Startup
reports CPU submission separately from waiting for the first GPU frame, so a
quick scene decode must not be reported as a quick cold first view.

`LARES_BENCH_COMPILE=1` reports queue-completion times after each pipeline is
created. These are cumulative timings because pipeline compilation is queued;
adjacent differences help locate expensive stages. Use
`__GL_SHADER_DISK_CACHE=0` with a fresh browser profile for NVIDIA cold tests.
The benchmark server serves precompressed assets, matching the application.
After the visibility split and uniform normal loop, two fresh-profile Firefox
159 headless runs reached ready in 4.18 and 4.59 seconds with NVIDIA's shader
disk cache disabled and direct presentation enabled. Chromium cold loads took
5.2–5.8 seconds; its 2560×1600 camera-motion test sustained 59.9 FPS with
4.73 ms p95 GPU time and one frame in flight. Headless startup measurements do
not validate the desktop compositor's presentation behaviour.

On affected Linux/NVIDIA Firefox versions, DMABUF presentation can display stale
frames even when the GPU output is correct. The former debug ball reproduced
this independently of sensor input; it has been removed. Disabling
`dom.webgpu.allow-present-without-readback` is a browser-side workaround;
[Mozilla bug 2078553](https://bugzilla.mozilla.org/show_bug.cgi?id=2078553)
tracks the matching presentation regression and its Firefox 160 fix. Shader
optimization reduces startup work but cannot repair browser buffer recycling.



Sun shadows use a continuous SDF cone with the physical 0.53-degree solar disc,
independent of the four camera AA offsets. Expanded BVH bounds retain penumbra
near misses. Candidate walks are bounded to 48 conservative steps and stop at
opaque hits. The existing visibility target stores 8-bit sunlight and 4-bit
ambient contacts, 16-bit octahedral normals and a 15-bit height (under 0.1 mm
steps across 3.2 m), without another render target or shadow sample history.
Penumbra width grows with receiver/blocker separation. This is a bounded SDF
approximation, not an exact integral of the solar disc, and thin grazing geometry
can exceed the walk budget. Fixture visibility uses raw baked depths and receiver-plane-corrected
percentage-closer filtering (twenty search depth taps, up to 84 including the filled-disc penumbra filter); fixture BRDF directions use the same central
source as that atlas rather than shifting between AA updates. Ceiling fittings
follow camera-only ceiling cutaways, while their lighting remains active.
The room compiler removes low trim fragments without a surviving supporting
wall, preventing opening cuts from leaving isolated skirting posts in the lounge.


Fixture source position and lens radius are shared by direct shading, visibility
and baked unit-light transport. Brightness and linear RGB remain runtime inputs:
source changes recompose independently controllable fixture-group responses,
including their baked diffuse bounces. Geometry visibility does not depend on
brightness or colour. Sunlight, sky and fixtures share the same linear shading
and transport conventions; their geometric projections follow source shape.

Shadow atlas version 3 stores unblurred 16-bit depths. Downlights use a downward
hemisphere projection, allocating more directional detail than the former
full-sphere map at the same 512-pixel tile size. Depth comparisons precede
visibility filtering, avoiding variance-shadow light bleeding. Receiver-plane
correction replaces the former slope bias of up to 25 mm with a 1.5 mm guard.
A four-position blocker search covers both sides of silhouettes. A fixed 16-position
filled-disc filter integrates the resulting penumbra in every frame, bounded to
eight projected texels. Uniform search regions skip the disc filter. This remains
a finite-resolution approximation rather than exact area-light integration.

Camera queries, nearest-blocker atlas baking and binary connection checks now
share one geometry traversal. Bake footprint, surface-filter, volume-filter and
bounce-interpolation links use one optical segment-visibility helper. These
connection tests run during authoring, not during smart-light updates. Thin
maple shelves now get projected surface samples instead of being mistaken for
skirting and falling back to the 50 cm volume. Volume reconstruction is still
an approximation for layered furnishings without surface charts.


Exposed folded-linen layers use 5 cm projected samples, with a projection-plane
limit proportional to sample spacing. These dense opaque-layer charts retain
surface reconstruction at rounded rims instead of blending back to the coarse
volume. Deferred shading, offline shading and metering share this reconstruction
policy. Floor charts use a 10 cm world lattice; other architectural faces keep
20 cm spacing. Both retain four-neighbour gathers. The denser fields increase
bake time and storage; runtime source RGB/brightness composition remains linear.
The `spare-plan` and `boiler-plan` capture cameras reproduce overhead diagnostics.


Chart origins and integer dimensions are stored in lighting asset version 5
and copied directly to GPU object records. The shader never infers chart offsets
with floating-point floor/ceil. Division versus reciprocal multiplication can
straddle an integer boundary (the spare room's 2.6 m boundary is one example),
so recomputing the allocation on the GPU can address a different chart row.
Sun visibility uses square-root encoding in its existing eight bits, retaining
fine dark-penumbra values under HDR solar radiance.

For a CPU capture matching the browser benchmark's daylight, fixture states and
locked full-plan meter, use `LARES_TIMESTAMP=1782043200`,
`LARES_CAPTURE_LIGHTS=on` and `LARES_CAPTURE_METER=plan`. Without those options,
photo captures use their documented evening-on/daytime-off setups and separate
view metering. Benchmark JSON now records the read-only `render_diagnostics()`
snapshot of camera, sun and display inputs to make mismatches visible.
