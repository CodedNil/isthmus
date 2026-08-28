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
      runtimeLibraries =
        pkgs: with pkgs; [
          wayland
          vulkan-loader
          libxkbcommon
        ];
      runtimeTools =
        pkgs: with pkgs; [
          pipewire
          wireplumber
        ];
      nightlyRust =
        pkgs:
        pkgs.rust-bin.nightly."2026-05-22".default.override {
          extensions = [
            "clippy"
            "rustfmt"
            "rust-src"
            "rustc-dev"
            "llvm-tools"
          ];
        };
      nightlyRustPlatform =
        pkgs:
        pkgs.makeRustPlatform {
          rustc = nightlyRust pkgs;
          cargo = nightlyRust pkgs;
        };
    in
    {
      packages.${system} =
        let
          rustPlatform = nightlyRustPlatform pkgs;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "rustc_codegen_spirv-0.10.0-alpha.1" = "sha256-M3/puV8CnGDp4I4C/F4lrH/Dfbs6Lj4T4j4vwdBMzrU=";
              "sysinfo-0.39.6" = "sha256-HgD13E5L5Gtwj1I1mD+vU3ln0dfj61Zeet8LHyUIdkk=";
            };
          };
          src = lib.cleanSource ./.;
          sysrootVendorPatch = ''
            for crate in ${nightlyRust pkgs}/lib/rustlib/src/rust/library/vendor/*; do
              name="$(basename "$crate")"
              if [ ! -e "$cargoDepsCopy/$name" ]; then
                cp -r "$crate" "$cargoDepsCopy/"
              fi
            done
          '';
          cantusShader = rustPlatform.buildRustPackage {
            pname = "cantus-shader";
            version = (lib.importTOML ./crates/cantus/Cargo.toml).package.version;
            inherit src cargoLock;
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
            inherit pname src cargoLock;
            version = (lib.importTOML ./crates/cantus/Cargo.toml).package.version;
            buildAndTestSubdir = "crates/cantus";
            CANTUS_SHADER_SPV = "${cantusShader}/isthmus.spv";
            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
              mold
            ];
            buildInputs = runtimeLibraries pkgs;
            postInstall = ''
              wrapProgram "$out/bin/${pname}" \
                --set LD_LIBRARY_PATH "${lib.makeLibraryPath (runtimeLibraries pkgs)}" \
                --prefix PATH : "${lib.makeBinPath (runtimeTools pkgs)}"
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
          (nightlyRust pkgs)
          mold
          pkg-config
          just
          nixfmt
          pipewire
          wireplumber
        ];
        buildInputs = runtimeLibraries pkgs;
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibraries pkgs);
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
            inherit (lib)
              literalExpression
              mkEnableOption
              mkIf
              mkOption
              optional
              optionalAttrs
              types
              ;

            cfg = config.programs.cantus;
            settingsFormat = pkgs.formats.toml { };
            settingsOptions = import ./generated-options.nix { inherit lib; };
            settingsDefaults = lib.mapAttrs (_: option: option.default) settingsOptions;
          in
          {
            options.programs.cantus = {
              enable = mkEnableOption description;

              package = mkOption {
                type = types.package;
                default = self.packages.${system}.cantus;
                defaultText = literalExpression "inputs.${pname}.packages.${system}.${pname}";
                description = "Cantus package to install.";
              };

              autoStart = mkOption {
                type = types.bool;
                default = true;
                description = "Whether to start the Cantus widget automatically.";
              };

              settings = mkOption {
                type = types.nullOr (
                  types.submodule {
                    options = settingsOptions;
                  }
                );
                default = null;
                description = "Settings written as TOML to `~/.config/cantus/cantus.toml`.";
                example = settingsDefaults;
              };
            };

            config = mkIf cfg.enable {
              home.packages = [ cfg.package ];

              xdg.configFile = optionalAttrs (cfg.settings != null) {
                "cantus/cantus.toml".source = settingsFormat.generate "cantus.toml" (
                  lib.filterAttrs (_: value: value != null) cfg.settings
                );
              };

              systemd.user.services.cantus = mkIf cfg.autoStart {
                Unit = {
                  Description = description;
                  After = [ config.wayland.systemd.target ];
                  X-Restart-Triggers = optional (
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
