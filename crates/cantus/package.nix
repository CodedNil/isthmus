{
  lib,
  pkgs,
  rust,
  description,
}:
let
  runtimeTools = with pkgs; [
    pipewire
    wireplumber
  ];
  runtimeLibraryPath = "${lib.makeLibraryPath [ pkgs.vulkan-loader ]}:/run/opengl-driver/lib";
  rustPlatform = pkgs.makeRustPlatform {
    rustc = rust;
    cargo = rust;
  };
in
rustPlatform.buildRustPackage {
  pname = "cantus";
  cargoDeps = pkgs.symlinkJoin {
    name = "cargo-vendor-dir";
    # build-std also resolves dependencies from the pinned Rust sysroot.
    paths = [
      (rustPlatform.importCargoLock {
        lockFile = ../../Cargo.lock;
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
  RUSTFLAGS = "--remap-path-prefix=${rust}=/rustc";
  postInstall = ''
    wrapProgram "$out/bin/$pname" \
      --set LD_LIBRARY_PATH "${runtimeLibraryPath}" \
      --set-default TZDIR "${pkgs.tzdata}/share/zoneinfo" \
      --prefix PATH : "${lib.makeBinPath runtimeTools}"
  '';
  passthru = { inherit runtimeTools runtimeLibraryPath; };
  meta = {
    inherit description;
    homepage = "https://github.com/CodedNil/cantus";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [ CodedNil ];
    platforms = lib.platforms.linux;
    mainProgram = "cantus";
  };
}
