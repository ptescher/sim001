//! The ground under the map: the top-down picture of the valley that the 3D view exports
//! (`towers_view --export-map`, the format of `sim001 --export-map` that the ground station's
//! tactical display draws). It is made from the same trees, lake and lanes as the 3D view's world, in
//! metres north and east of the field's centre (SITL's home), so everything the app draws lies on
//! the same ground as the game, the cameras and the ground station.

use glam::Vec2;
use gpui::RenderImage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Ground {
    pub image: Arc<RenderImage>,
    /// The picture is a square this wide (m)...
    pub size_m: f32,
    /// ...and its middle is here (north, east of the field's centre, m).
    pub centre: Vec2,
}

/// Where the picture is exported to, and read from.
pub fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

impl Ground {
    pub fn load(dir: &Path) -> Option<Ground> {
        let meta = std::fs::read_to_string(dir.join("terrain_map.txt")).ok()?;
        let get = |k: &str| -> Option<f32> {
            meta.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(key, _)| key.trim() == k)
                .and_then(|(_, v)| v.trim().parse().ok())
        };
        let mut pixels = image::open(dir.join("terrain_map.png")).ok()?.into_rgba8();
        // GPUI's image atlas is BGRA.
        for p in pixels.pixels_mut() {
            p.0.swap(0, 2);
        }
        Some(Ground {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
            size_m: get("size_m")?,
            centre: Vec2::new(get("centre_north")?, get("centre_east")?),
        })
    }
}
