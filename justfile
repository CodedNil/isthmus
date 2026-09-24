default: cantus

cantus:
    cargo run -p cantus -- --generate-nix-options
    cargo run -p cantus

cantusweb:
    cargo build --release -Zbuild-std=std,panic_abort --lib -p cantus --target wasm32-unknown-unknown
    wasm-bindgen --target web --out-dir crates/cantus/assets/web target/wasm32-unknown-unknown/release/cantus.wasm
    python3 -m http.server 8000 --directory crates/cantus/assets

paries:
    cargo run -p paries

lares:
    cargo build --release -Zbuild-std=std,panic_abort --lib -p lares --target wasm32-unknown-unknown
    wasm-bindgen --target web --out-dir crates/lares/assets/web target/wasm32-unknown-unknown/release/lares.wasm
    cargo run -p lares
