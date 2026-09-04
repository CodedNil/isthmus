use std::{
    env, fs,
    path::{Path, PathBuf},
};

use isthmus_build::ShaderBuild;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let source = manifest.join("src/render/mod.rs");
    let isthmus = manifest.join("../isthmus");
    let workspace = manifest.join("../..");

    rerun_for_sources(&manifest.join("src/render"));
    rerun_for_sources(&isthmus.join("src"));
    rerun_for_sources(&manifest.join("../isthmus_macros/src"));
    println!("cargo:rerun-if-changed={}", manifest.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", manifest.join("../Cargo.lock").display());

    println!("cargo:rerun-if-env-changed=CANTUS_SHADER_SPV");
    let output = out_dir.join("isthmus.spv");
    if let Some(shader) = env::var_os("CANTUS_SHADER_SPV") {
        if !output.exists() {
            fs::copy(shader, &output).expect("failed to copy Cantus shader");
        }
        return;
    }

    ShaderBuild {
        name: String::from("cantus"),
        source,
        isthmus,
        workspace,
        output,
    }
    .build()
    .expect("failed to build Cantus shader");
}

fn rerun_for_sources(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}
