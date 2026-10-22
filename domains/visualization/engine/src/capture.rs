//! Pictures of the scene from views set by hand: to hold the renderer up against photos of a
//! real field, and to make pictures for camera-based systems to learn from.
//!
//! Once the world has loaded, each view is held still until the ray tracer has settled, then the
//! 3D view (at full resolution, with no HUD) is saved as `<dir>/<name>.png`. The app exits after
//! the last one.

use crate::camera::{PinnedView, RenderScale, View};
use crate::loading::AppState;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use std::path::PathBuf;

#[derive(Resource, Clone)]
pub struct Capture {
    pub dir: PathBuf,
    pub views: Vec<(String, View)>,
    /// How long each view is held before its picture, so the lighting and the anti-aliasing
    /// settle.
    pub settle_secs: f32,
}

#[derive(Resource, Default)]
struct Progress {
    next: usize,
    held: f32,
    /// Time since the last picture was asked for.
    done: f32,
}

pub fn plugin(app: &mut App, capture: Capture) {
    std::fs::create_dir_all(&capture.dir).ok();
    // Old pictures would look like new ones saved.
    for (name, _) in &capture.views {
        std::fs::remove_file(capture.dir.join(format!("{name}.png"))).ok();
    }
    app.insert_resource(capture)
        .init_resource::<Progress>()
        .add_systems(Update, step.run_if(in_state(AppState::Running)));
}

fn step(
    mut commands: Commands,
    time: Res<Time>,
    capture: Res<Capture>,
    mut progress: ResMut<Progress>,
    mut pinned: ResMut<PinnedView>,
    mut scale: ResMut<RenderScale>,
    mut exit: MessageWriter<AppExit>,
) {
    // Full resolution: the pictures are for looking at closely. (SIM001_SCALE renders at a
    // fraction instead, to compare the upscaling with a full-size picture.)
    scale.auto = false;
    scale.scale =
        std::env::var("SIM001_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).map_or(1.0, |s| s.clamp(0.1, 1.0));
    let Some((name, view)) = capture.views.get(progress.next) else {
        // Saving happens a few frames later, on another thread: wait for the last file.
        progress.done += time.delta_secs();
        let saved = capture.views.last().is_none_or(|(name, _)| capture.dir.join(format!("{name}.png")).exists());
        if (saved && progress.done > 1.0) || progress.done > 20.0 {
            exit.write(AppExit::Success);
        }
        return;
    };
    if pinned.0 != Some(*view) {
        pinned.0 = Some(*view);
        progress.held = 0.0;
        return;
    }
    progress.held += time.delta_secs();
    if progress.held >= capture.settle_secs {
        let path = capture.dir.join(format!("{name}.png"));
        info!("capture {}", path.display());
        commands.spawn(Screenshot::image(scale.image.clone())).observe(save_to_disk(path));
        progress.next += 1;
    }
}
