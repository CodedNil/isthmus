//! Incremental, lossless lighting bake; UI and camera edits reuse existing assets.
use crate::{
    home,
    render::{
        geometry::acceleration::Scene,
        light::{
            shadows::{RESOLUTION, ShadowAtlas},
            sun::SunSettings,
        },
    },
};
use flate2::read::GzDecoder;
use std::{
    env, fs,
    fs::File,
    io,
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};
use tracing::info;

fn current(path: &Path, magic: [u8; 8], signature: u64) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut header = [0; 16];
    GzDecoder::new(file).read_exact(&mut header).is_ok()
        && header[..8] == magic
        && header[8..] == signature.to_le_bytes()
}

/// Where the baked lighting lives: `LARES_BAKED` if set, such as a service's state directory, else the source tree.
pub fn baked_dir() -> PathBuf {
    env::var_os("LARES_BAKED")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/baked"), PathBuf::from)
}

pub fn ensure_assets() -> Result<(), io::Error> {
    let started = Instant::now();
    let root = baked_dir();
    let visibility_path = root.join("visibility.bin");
    let lighting_path = root.join("lighting.bin");
    let rooms = home::rooms();
    let signature = Scene::geometry_signature(&rooms);
    let visibility_current = current(&visibility_path, *b"LARESV04", signature);
    let lighting_current = current(&lighting_path, *b"LARESL05", signature);
    if visibility_current && lighting_current {
        info!("lighting assets are current ({:?}); no rebake", started.elapsed());
        return Ok(());
    }
    let scene_started = Instant::now();
    let mut scene = Scene::new_at(&rooms, 1_782_043_200.0, SunSettings::default());
    info!("scene built in {:?}", scene_started.elapsed());
    fs::create_dir_all(&root)?;
    if !lighting_current {
        let controls_started = Instant::now();
        let bytes = scene.encode_light_controls()?;
        info!("light controls encoded in {:?}", controls_started.elapsed());
        fs::write(&lighting_path, &bytes)?;
        info!("{}: {} bytes", lighting_path.display(), bytes.len());
    }
    if !visibility_current {
        let atlas_started = Instant::now();
        let atlas = ShadowAtlas::bake_cpu(&scene, RESOLUTION);
        info!("shadow atlas traced in {:?}", atlas_started.elapsed());
        let bytes = atlas.encode(&scene)?;
        fs::write(&visibility_path, &bytes)?;
        info!("{}: {} bytes", visibility_path.display(), bytes.len());
    }
    info!("lighting assets baked in {:?}", started.elapsed());
    Ok(())
}
