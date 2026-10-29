//! Explicitly refresh static lighting assets; unchanged geometry is a fast no-op.
use lares::bake::ensure_assets;
use std::io;

fn main() -> Result<(), io::Error> {
    ensure_assets()
}
