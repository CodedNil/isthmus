default: cantus

cantus:
    cargo run -p isthmus-build --features compiler --bin isthmus -- build crates/cantus
    cargo run -p cantus --features generate-nix
