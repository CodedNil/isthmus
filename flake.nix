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
      inherit (nixpkgs) lib;
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      pname = "cantus";
      runtimeLibraries = with pkgs; [
        wayland
        vulkan-loader
        libxkbcommon
      ];
      runtimeTools = with pkgs; [
        pipewire
        wireplumber
      ];
      runtimeLibraryPath = "${lib.makeLibraryPath runtimeLibraries}:/run/opengl-driver/lib";
      rustToolchain = pkgs.rust-bin.nightly."2026-05-22";
      rust = rustToolchain.default.override {
        extensions = [
          "clippy"
          "rustfmt"
          "rust-src"
          "rustc-dev"
          "llvm-tools"
        ];
      };
      rustPlatform = pkgs.makeRustPlatform {
        rustc = rust;
        cargo = rust;
      };
    in
    {
      packages.${system} =
        let
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "rustc_codegen_spirv-0.10.0-alpha.1" = "sha256-M3/puV8CnGDp4I4C/F4lrH/Dfbs6Lj4T4j4vwdBMzrU=";
            };
          };
          version = (lib.importTOML ./crates/cantus/Cargo.toml).package.version;
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./rustfmt.toml
              ./crates
              ./assets/NotoSans-Variable.ttf
            ];
          };
          sysrootVendorPatch = ''
            for crate in ${rust}/lib/rustlib/src/rust/library/vendor/*; do
              name="''${crate##*/}"
              if [ ! -e "$cargoDepsCopy/$name" ]; then
                cp -r "$crate" "$cargoDepsCopy/"
              fi
            done
          '';
          cantusShader = rustPlatform.buildRustPackage {
            pname = "cantus-shader";
            inherit src cargoLock version;
            postPatch = sysrootVendorPatch;
            doCheck = false;
            dontCargoInstall = true;
            buildPhase = ''
              runHook preBuild
              cargo run --release --offline \
                --manifest-path crates/isthmus_build/Cargo.toml \
                --bin shader-build -- \
                cantus \
                "$PWD/crates/cantus/src/render/mod.rs" \
                "$PWD/crates/isthmus" \
                "$PWD" \
                "$PWD/isthmus.spv"
              runHook postBuild
            '';
            installPhase = ''
              install -Dm644 isthmus.spv "$out/isthmus.spv"
              install -Dm644 isthmus.manifest.rs "$out/isthmus.manifest.rs"
            '';
            nativeBuildInputs = with pkgs; [
              pkg-config
              mold
            ];
          };
        in
        rec {
          default = cantus;
          "cantus-shader" = cantusShader;
          cantus = rustPlatform.buildRustPackage {
            inherit
              pname
              src
              cargoLock
              version
              ;
            buildAndTestSubdir = "crates/cantus";
            CANTUS_SHADER_SPV = "${cantusShader}/isthmus.spv";
            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
              mold
            ];
            buildInputs = runtimeLibraries;
            postInstall = ''
              wrapProgram "$out/bin/${pname}" \
                --set LD_LIBRARY_PATH "${runtimeLibraryPath}" \
                --prefix PATH : "${lib.makeBinPath (runtimeTools)}"
            '';
            meta = {
              inherit description;
              homepage = "https://github.com/CodedNil/cantus";
              license = lib.licenses.mit;
              maintainers = with lib.maintainers; [ CodedNil ];
              platforms = lib.platforms.linux;
              mainProgram = pname;
            };
          };
        };

      devShells.${system}.default = pkgs.mkShell {
        name = pname;
        packages = with pkgs; [
          rust
          mold
          pkg-config
          just
          nixfmt
          spirv-tools
          pipewire
          wireplumber
        ];
        buildInputs = runtimeLibraries;
        LD_LIBRARY_PATH = runtimeLibraryPath;
      };

      formatter.${system} = pkgs.nixfmt;

      homeManagerModules = {
        default = self.homeManagerModules.cantus;
        cantus =
          {
            config,
            lib,
            pkgs,
            ...
          }:
          let
            cfg = config.programs.cantus;
            settingsFormat = pkgs.formats.toml { };
            settingsOptions = import ./generated-options.nix { inherit lib; };
          in
          {
            options.programs.cantus = {
              enable = lib.mkEnableOption description;

              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${system}.cantus;
                defaultText = lib.literalExpression "inputs.${pname}.packages.${system}.${pname}";
                description = "Cantus package to install.";
              };

              autoStart = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = "Whether to start the Cantus widget automatically.";
              };

              settings = lib.mkOption {
                type = lib.types.nullOr (
                  lib.types.submodule {
                    options = settingsOptions;
                  }
                );
                default = null;
                description = "Settings written as TOML to `~/.config/cantus/cantus.toml`.";
                example = lib.mapAttrs (_: option: option.default) settingsOptions;
              };
            };

            config = lib.mkIf cfg.enable {
              home.packages = [ cfg.package ];

              xdg.configFile = lib.optionalAttrs (cfg.settings != null) {
                "cantus/cantus.toml".source = settingsFormat.generate "cantus.toml" (
                  lib.filterAttrs (_: value: value != null) cfg.settings
                );
              };

              systemd.user.services.cantus = lib.mkIf cfg.autoStart {
                Unit = {
                  Description = description;
                  After = [ config.wayland.systemd.target ];
                  X-Restart-Triggers = lib.optional (
                    cfg.settings != null
                  ) config.xdg.configFile."cantus/cantus.toml".source;
                };

                Service = {
                  Type = "simple";
                  ExecStart = "${cfg.package}/bin/${pname}";
                  Restart = "on-failure";
                };

                Install.WantedBy = [ config.wayland.systemd.target ];
              };
            };
          };
      };
    };
}
