use isthmus_build::ShaderBuild;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"));
    let workspace = manifest.join("../..");
    rerun_for_sources(&manifest.join("src"));
    ShaderBuild {
        name: "paries",
        source: manifest.join("src/sdf.rs"),
        isthmus: manifest.join("../isthmus"),
        workspace,
        output: PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set")).join("isthmus.spv"),
    }
    .build()
    .expect("failed to build Paries shader");
}

fn rerun_for_sources(path: &Path) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("shader source directory is readable") {
            rerun_for_sources(&entry.expect("shader source entry is readable").path());
        }
    } else {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
