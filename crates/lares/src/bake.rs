//! Incremental, lossless lighting bake; UI and camera edits reuse existing assets.
use crate::{
    home,
    render::{
        Scene, SunSettings,
        shadows::{RESOLUTION, ShadowAtlas},
    },
};
use flate2::read::GzDecoder;
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::Instant,
};

fn current(path: &Path, magic: [u8; 8], signature: u64) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut header = [0; 16];
    GzDecoder::new(file).read_exact(&mut header).is_ok()
        && header[..8] == magic
        && header[8..] == signature.to_le_bytes()
}

pub fn ensure_assets() -> Result<(), io::Error> {
    let started = Instant::now();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/baked");
    let visibility_path = root.join("visibility.bin");
    let lighting_path = root.join("lighting.bin");
    let rooms = home::rooms();
    let signature = Scene::geometry_signature(&rooms);
    let visibility_current = current(&visibility_path, *b"LARESV02", signature);
    let lighting_current = current(&lighting_path, *b"LARESL01", signature);
    if visibility_current && lighting_current {
        println!("Lighting assets are current ({:?}); no rebake.", started.elapsed());
        return Ok(());
    }
    let scene = Scene::new_at(&rooms, 1_782_043_200.0, SunSettings::default());
    fs::create_dir_all(&root)?;
    if !lighting_current {
        let bytes = scene.encode_light_controls()?;
        fs::write(&lighting_path, &bytes)?;
        println!("{}: {} bytes", lighting_path.display(), bytes.len());
    }
    if !visibility_current {
        let atlas = ShadowAtlas::bake_cpu(&scene, RESOLUTION);
        let bytes = atlas.encode(&scene)?;
        fs::write(&visibility_path, &bytes)?;
        println!("{}: {} bytes", visibility_path.display(), bytes.len());
    }
    println!("Lighting assets baked in {:?}.", started.elapsed());
    Ok(())
}
