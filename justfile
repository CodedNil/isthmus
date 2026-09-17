default: cantus

cantus: nix-options
    cargo run -p cantus

nix-options:
    cargo run -p cantus -- --generate-nix-options

cantusweb: cantusweb-build
    python3 -m http.server 8000 --directory assets/web

cantusweb-build:
    cargo build --release -Zbuild-std=std,panic_abort --lib -p cantus --target wasm32-unknown-unknown
    wasm-bindgen --target web --out-dir assets/web target/wasm32-unknown-unknown/release/cantus.wasm

paries:
    cargo run -p paries
