# Rendering contracts

Tests check renderer behaviour across ranges of inputs. Authored furniture and
camera placement are reviewed using captures and photographic references.

- `geometry.rs`: accelerated versus exhaustive traversal, valid normals,
  conservative light culling, opaque structure, window apertures, camera-only
  cutaways and indoor probe allocation.
- `lifecycle.rs`: accumulation over advancing time, light controls, discontinuities,
  emitter distributions and camera exposure.
- `../scene/irradiance/transport_tests.rs`: linearity and RGB superposition,
  bounded incremental updates, partition isolation, full versus incremental
  relighting and nonnegative directional reconstruction.
- `../plants.rs`: curved foliage and stem containment across species, rotations
  and wall clearances.
- Numerical tests beside implementations cover photometry, solar motion,
  material filtering, reciprocal BRDFs, tracing, shadow encodings and temporal
  sample reconstruction.

Run `cargo test -p lares -p isthmus --lib`. Geometry or lighting changes also
require native/WASM Clippy, a release WASM build and visual capture review.
See [the rendering tools](../../../tools/README.md) for capture and bounded
browser checks. Browser checks exercise GPU validation, scheduling and
accumulation while the real clock advances.
