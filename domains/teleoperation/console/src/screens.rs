//! The displays the station can lay windows out on, in global points (origin at the top-left
//! of the main display, y down) with their usable area below the menu bar and beside the Dock.

use gpui::App;
use teleoperation_shared::layout::{Display, Rect};

#[cfg(target_os = "macos")]
pub fn displays(_cx: &App) -> Vec<Display> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let Some(mtm) = MainThreadMarker::new() else { return vec![] };
    let screens = NSScreen::screens(mtm);
    // AppKit is bottom-left origin; the first screen is the main one (it has the menu bar).
    let main_h = screens.firstObject().map_or(0.0, |s| s.frame().size.height);
    screens
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let v = s.visibleFrame();
            let top = main_h - (v.origin.y + v.size.height);
            Display { rect: Rect::new(v.origin.x as f32, top as f32, v.size.width as f32, v.size.height as f32), main: i == 0 }
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn displays(cx: &App) -> Vec<Display> {
    // GPUI reports display sizes but not their arrangement here: tile on the primary.
    cx.primary_display()
        .map(|d| {
            let b = d.bounds();
            vec![Display { rect: Rect::new(0.0, 0.0, f32::from(b.size.width), f32::from(b.size.height)), main: true }]
        })
        .unwrap_or_default()
}

/// Hides the close / minimize / zoom buttons. GPUI's transparent title bar keeps them, and the
/// station's windows are placed and closed by the station (Cmd+Q), not by hand.
#[cfg(target_os = "macos")]
pub fn hide_window_buttons(window: &gpui::Window) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: GPUI's AppKit handle points at its live NSView, and we're on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    if let Some(win) = view.window() {
        for b in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
            if let Some(button) = win.standardWindowButton(b) {
                button.setHidden(true);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn hide_window_buttons(_window: &gpui::Window) {}
