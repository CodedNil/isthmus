# Rendering contracts

Tests check renderer behaviour across ranges of inputs. Authored furniture and
camera placement are reviewed using captures and photographic references.

- `geometry.rs`: accelerated versus exhaustive traversal, valid normals,
  correct hit-face normals near edges, conservative light culling, opaque structure, window apertures, camera-only
  cutaways and indoor probe allocation.
- `lifecycle.rs`: accumulation over advancing time, light controls, discontinuities,
  emitter distributions and camera exposure.
- `../scene/irradiance/transport_tests.rs`: linearity and RGB superposition,
  surface cosine-gather normalization, bounded incremental updates, partition isolation, full versus incremental
  relighting and nonnegative directional reconstruction. Brightness linearity covers both surface
  and volume transport.
- `../plants.rs`: curved foliage and stem containment across species, rotations
  and wall clearances.
- Numerical tests beside implementations cover photometry, solar motion,
  material filtering, reciprocal BRDFs/fibre masking, procedural SDF pruning
  versus exhaustive unions, tracing, shadow encodings and temporal
  sample reconstruction, coplanar construction-joint continuity, missing-site continuity and visibility-aware receiver interpolation. Projected receiver tests also put an extreme irradiance discontinuity across a partition to detect interpolation leaks.

Run tests for the affected contracts; `cargo test -p lares -p isthmus --lib`
runs the complete suite when warranted. Material changes use the material and
BRDF tests plus visual captures; transport changes use the relevant transport
contracts. Compile shader changes for WASM before updating the web assets.
See [the rendering tools](../../../tools/README.md) for capture and bounded
browser checks. Browser checks exercise GPU validation, scheduling and
accumulation while the real clock advances.

`../scene/contacts.rs` checks bounded local contact shading and compares its
spatial candidate index with exhaustive SDF queries across rotations and cell
boundaries. Indirect denoising is geometry constrained and must preserve
partition isolation and full/incremental relighting agreement.
