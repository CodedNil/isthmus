{
  lib,
  pkgs,
  rust,
}:
let
  rustPlatform = pkgs.makeRustPlatform {
    rustc = rust;
    cargo = rust;
  };
in
rustPlatform.buildRustPackage {
  pname = "cantus";
  version = (lib.importTOML ./Cargo.toml).package.version;

  cargoDeps = pkgs.symlinkJoin {
    name = "cargo-vendor-dir";
    # Rust-GPU builds core using the pinned sysroot's lockfile.
    paths = [
      (rustPlatform.importCargoLock {
        lockFile = ../../Cargo.lock;
        outputHashes."ash-0.38.0+1.4.352" = "sha256-tq/WozqxpZHRAG8dtVtAYADM058f5hfMsyQsU5EwIGY=";
      })
      (rustPlatform.importCargoLock {
        lockFile = "${rust}/lib/rustlib/src/rust/library/Cargo.lock";
      })
    ];
  };

  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../LICENSE
      ../../crates
    ];
  };
  buildAndTestSubdir = "crates/cantus";

  nativeBuildInputs = with pkgs; [
    makeWrapper
    pkg-config
  ];
  buildInputs = with pkgs; [
    aws-lc
    libxkbcommon
    wayland
  ];
  nativeCheckInputs = [ pkgs.cacert ];

  # Nixpkgs' target flags take precedence over Cargo's build flags.
  RUSTFLAGS = (lib.importTOML ../../.cargo/config.toml).build.rustflags ++ [
    # Keep compiler source paths from pulling the toolchain into the runtime closure.
    "--remap-path-prefix=${rust}=/rustc"
  ];

  postFixup = ''
    patchelf --add-rpath "${lib.makeLibraryPath [ pkgs.vulkan-loader ]}" "$out/bin/$pname"
    wrapProgram "$out/bin/$pname" --prefix PATH : "${
      lib.makeBinPath [
        pkgs.pipewire
        pkgs.wireplumber
      ]
    }"
  '';

  meta = {
    description = "Interactive music widget for Wayland";
    homepage = "https://github.com/CodedNil/cantus";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [ CodedNil ];
    platforms = lib.platforms.linux;
    mainProgram = "cantus";
  };
}
