default: cantus

cantus:
    cargo run -p cantus -- --generate-nix-options
    cargo run -p cantus

cantusweb: (bindgen "cantus")
    python3 -m http.server 8000 --directory crates/cantus/assets

paries:
    cargo run -p paries

# Bakes lighting ahead of serving (about 15 s); `just lares` does the same on first start if it is stale.
bake:
    cargo run --release -p lares -- --bake

lares: (bindgen "lares")
    gzip -kf crates/lares/assets/web/lares_bg.wasm crates/lares/assets/web/lares.js
    cargo run -p lares

[private]
bindgen crate:
    cargo build --release -Zbuild-std=std,panic_abort --lib --target wasm32-unknown-unknown -p {{ crate }}
    wasm-bindgen --target web --out-dir crates/{{ crate }}/assets/web target/wasm32-unknown-unknown/release/{{ crate }}.wasm
