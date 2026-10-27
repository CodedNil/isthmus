# Rendering tools

Lares uses procedural SDF geometry. Rust examples provide offline renders,
image differences and CPU transport benchmarks:

```sh
LARES_LIGHTS=both LARES_TIMESTAMP=1782043200 LARES_SHADOW_RESOLUTION=512 cargo run -p lares --release --example capture -- crates/lares/assets/references 1200 900 all
cargo run -p lares --release --example gallery
cargo run -p lares --release --example compare -- reference.webp candidate.png /tmp/error.png
cargo run -p lares --release --example lighting_bench
cargo test -p lares -p isthmus --lib
```

[Lights-on comparison](../assets/references/comparison-lights-on.webp) and
[lights-off comparison](../assets/references/comparison-lights-off.webp)
contain photographs on the left and current renders on the right. Rows:
entrance, bookshelf, kitchen-fridge, kitchen-window, kitchen-cooker, lounge,
desk, hall, bedroom, bathroom, ensuite, boiler-room, spare-room, storage-top,
storage-bottom. Missing photographs leave a blank left column. Plan and
overhead are excluded from comparisons.

`LARES_LIGHTS=both` produces each view with all fixtures at 100% and then off,
using the same per-view exposure for both, at a fixed June 21 noon clock.
Daylight remains enabled in both sets. Off images have a `-lights-off` suffix.
Original photographs are preserved. Captures use four AA samples.
Captures use half the logical CPUs, capped at eight.

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
60-second deadline. It writes JSON metrics and a converged GPU screenshot.
Startup compilation and baking are excluded. `LARES_BENCH_MS` controls the
measurement duration (default 10000). Run after CPU captures/builds finish.
The running-clock check fails if settled accumulation resets as time advances.

## Renderer conventions

One linear lighting unit represents 100 lux incident or 100 cd/m² radiance.
GU10 fixtures use TRÅDFRI colour/white-spectrum 345 lm, 2700 K and a 40° beam;
the woven lamp assumes an unidentified 470 lm bulb. Clear-sky daylight uses
Ineichen/Perez with assumed turbidity 3. Solar motion uses date, coordinates
and north bearing. Weather and measured LED spectra are not modelled.
Exterior ground visible through windows uses an analytic 20% diffuse reflectance assumption, including sunlight;
no exterior probes or geometry are baked.

Static SDF visibility maps filter fixture shadows. Indoor-only transport uses
33 cm probes, 128 directions and eight diffuse bounces. Geometry changes rebuild
these caches; brightness/colour controls recompose source transport. Materials
include procedural normal gradients, roughness, metallic Fresnel, sheen and
clear coat. Indirect specular is an analytic approximation scaled by local GI.

Progressive rendering converges all native pixels to four AA samples. Large
views interleave columns over eight updates. Interaction targets 30 updates/s;
settled views use an 8 Hz ceiling and 20% GPU duty budget. Hidden views stop
submitting and queue depth is limited to one. See [rendering contracts](../src/render/tests/README.md)
for the automated checks.
