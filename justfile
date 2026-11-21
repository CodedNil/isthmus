default: cantus

cantus:
    cargo run -p cantus -- --generate-nix-options
    cargo run -p cantus

paries:
    cargo run -p paries

# Bakes lighting ahead of time. Lares also refreshes stale assets at startup.
bake:
    cargo run --release -p lares -- --bake

lares:
    cargo run -p lares
