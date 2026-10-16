//! `--window-rect x,y,w,h`: put the sim window where the ground station's layout wants it.
//!
//! The rectangle is the window frame in logical points (global desktop, y down), which is what
//! macOS and GPUI use. Bevy positions windows in physical pixels of the window's current
//! display, so this converts with the live scale factor and re-applies a few times: the scale
//! can change once the window lands on a different display.
//!
//! `--exit-with-parent`: quit when the process that started us (the ground station) goes away.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowPosition};
use groundstation_shared::layout::Rect;

/// The frame rectangle and the title bar height (measured by the ground station from AppKit;
/// the rectangle includes it, Bevy's resolution doesn't).
#[derive(Resource)]
pub struct WindowRect(pub Rect, pub f32);

pub fn place_window(rect: Res<WindowRect>, mut windows: Query<&mut Window, With<PrimaryWindow>>, mut frame: Local<u32>) {
    *frame += 1;
    // Frames 2, 12, 22, 32: early, then again after any display/scale change settles.
    if *frame > 32 || *frame % 10 != 2 {
        return;
    }
    let Ok(mut w) = windows.single_mut() else { return };
    let r = rect.0;
    let scale = w.scale_factor();
    w.position = WindowPosition::At(IVec2::new((r.x * scale).round() as i32, (r.y * scale).round() as i32));
    let (lw, lh) = (r.w, (r.h - rect.1).max(100.0));
    if (w.resolution.width() - lw).abs() > 0.5 || (w.resolution.height() - lh).abs() > 0.5 {
        w.resolution.set(lw, lh);
    }
}

/// Exits when our parent process dies (we get re-parented), so a sim started by the ground
/// station doesn't outlive it.
#[cfg(unix)]
pub fn exit_with_parent() {
    let parent = std::os::unix::process::parent_id();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if std::os::unix::process::parent_id() != parent {
                println!("parent process gone, exiting");
                std::process::exit(0);
            }
        }
    });
}

#[cfg(not(unix))]
pub fn exit_with_parent() {}
