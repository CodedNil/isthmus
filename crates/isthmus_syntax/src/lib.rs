//! Parsing and code generation for `ShaderData`, `program!` and `shader!`.
//!
//! The proc-macro crate expands these on the host, and the build script expands the GPU side
//! of the same declarations into a standalone shader crate, so both share this implementation.
#![expect(clippy::wildcard_enum_match_arm, reason = "syn's syntax enums are non-exhaustive")]

pub mod derive;
pub mod program;
pub mod shader;
