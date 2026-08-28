default: cantus

cantus:
    cargo run -p cantus --features generate-nix

cantusweb:
    cargo build --release -Zbuild-std=std,panic_abort --lib -p cantus --target wasm32-unknown-unknown
    cargo install --locked --version 0.2.127 wasm-bindgen-cli --root target/wasm-bindgen-cli
    target/wasm-bindgen-cli/bin/wasm-bindgen --target web --out-dir assets/web target/wasm32-unknown-unknown/release/cantus.wasm
    python3 -m http.server 8000 --directory assets/web

paries:
    cargo run -p paries
