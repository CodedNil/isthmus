rec {
  description = "Ergonomic bridge between the CPU and GPU, write inline shaders";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      # Rust-GPU requires this exact nightly and the compiler development components.
      rust = pkgs.rust-bin.nightly."2026-07-03".default.override {
        extensions = [
          "clippy"
          "rustfmt"
          "rust-src"
          "rustc-dev"
          "llvm-tools"
        ];
      };
      cantus = pkgs.callPackage ./crates/cantus/package.nix {
        inherit rust description;
      };
    in
    {
      packages.${system} = {
        inherit cantus;
        default = cantus;
      };

      devShells.${system}.default = pkgs.mkShell {
        name = "isthmus";
        inputsFrom = [ cantus ];
        packages =
          cantus.runtimeTools
          ++ (with pkgs; [
            rust
            just
            nixfmt
            spirv-tools
            wasm-bindgen-cli
          ]);
        LD_LIBRARY_PATH = cantus.runtimeLibraryPath;
      };

      formatter.${system} = pkgs.nixfmt;

      homeManagerModules = rec {
        default = cantus;
        cantus = import ./crates/cantus/home-manager.nix { inherit self description; };
      };
    };
}
