use std::{fs, path::Path};

fn hash_sources(path: &Path, hash: &mut u64) {
    let mut paths: Vec<_> =
        fs::read_dir(path).expect("source directory").map(|entry| entry.expect("source entry").path()).collect();
    paths.sort();
    for path in paths {
        if path.file_name().is_some_and(|name| name == "tests" || name.to_string_lossy().ends_with("_tests.rs")) {
            continue;
        }
        if path.is_dir() {
            hash_sources(&path, hash);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            for byte in fs::read(path).expect("source bytes") {
                *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
}

fn main() {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    // Camera, UI and server edits must not invalidate baked transport.
    for path in [
        "src/home.rs",
        "src/render/layout.rs",
        "src/render/acceleration.rs",
        "src/render/plants.rs",
        "src/render/photometry.rs",
        "src/render/shadows.rs",
        "src/render/scene.rs",
        "../isthmus/src/math.rs",
    ] {
        for byte in fs::read(path).expect("geometry source") {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash_sources(Path::new("src/render/scene"), &mut hash);
    println!("cargo:rustc-env=LARES_SCENE_VERSION={hash}");
    isthmus_build::build("src/lib.rs").expect("failed to build shaders");
}
