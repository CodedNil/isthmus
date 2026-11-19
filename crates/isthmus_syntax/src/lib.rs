//! Parsing and code generation for `ShaderData`, `program!` and shader draws.
//!
//! The proc-macro crate expands `ShaderData` and `program!` on the host. The build script reads shader draws
//! out of the source and expands their GPU side into a standalone shader crate, so both share this implementation.
#![expect(clippy::wildcard_enum_match_arm, reason = "syn's syntax enums are non-exhaustive")]

pub mod derive;
pub mod program;
pub mod shader;
