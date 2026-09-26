{
  lib,
  pkgs,
  rust,
  description,
}:
let
  pname = "cantus";
  nativeBuildInputs = with pkgs; [
    pkg-config
    makeWrapper
    mold
  ];
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
  rustPlatform = pkgs.makeRustPlatform {
    rustc = rust;
    cargo = rust;
  };
in
rustPlatform.buildRustPackage {
  inherit pname;
  cargoDeps = pkgs.symlinkJoin {
    name = "cargo-vendor-dir";
    # build-std also resolves dependencies from the pinned Rust sysroot.
    paths = [
      (rustPlatform.importCargoLock {
        lockFile = ../../Cargo.lock;
        outputHashes = {
          "rustc_codegen_spirv-0.10.0-alpha.1" = "sha256-OL8FIC3YOuH5Xkfee1alGKl6V43jlGaPXS08EOga/W0=";
        };
      })
      (rustPlatform.importCargoLock {
        lockFile = "${rust}/lib/rustlib/src/rust/library/Cargo.lock";
      })
    ];
  };
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../rustfmt.toml
      ../../crates
    ];
  };
  buildAndTestSubdir = "crates/cantus";
  inherit nativeBuildInputs;
  buildInputs = runtimeLibraries;
  RUSTFLAGS = "--remap-path-prefix=${rust}=/rustc";
  postInstall = ''
    wrapProgram "$out/bin/${pname}" \
      --set LD_LIBRARY_PATH "${runtimeLibraryPath}" \
      --prefix PATH : "${lib.makeBinPath runtimeTools}"
  '';
  passthru = { inherit runtimeTools runtimeLibraryPath; };
  meta = {
    inherit description;
    homepage = "https://github.com/CodedNil/cantus";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [ CodedNil ];
    platforms = lib.platforms.linux;
    mainProgram = pname;
  };
}
