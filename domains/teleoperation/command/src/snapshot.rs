//! A picture of the window as the system draws it, for the screenshots of `--expect --docs`
//! (macOS `screencapture`, as the console takes its own; it needs the window on screen and the
//! Screen Recording permission). Elsewhere there is none, and the docs say "Screenshot to come".

/// The window as a JPEG, or `None` when the system can't take it.
pub fn window_jpeg(window: &gpui::Window) -> Option<Vec<u8>> {
    let number = window_number(window)?;
    let path = std::env::temp_dir().join(format!("sim001_snapshot_{}_{number}.jpg", std::process::id()));
    let ok = std::process::Command::new("screencapture")
        .args(["-x", "-o", "-t", "jpg", "-l", &number.to_string()])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let bytes = if ok { std::fs::read(&path).ok() } else { None };
    let _ = std::fs::remove_file(&path);
    bytes.filter(|b| !b.is_empty())
}

/// The system's number for the window, for `screencapture -l`.
#[cfg(target_os = "macos")]
fn window_number(window: &gpui::Window) -> Option<isize> {
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: GPUI's AppKit handle points at its live NSView, and we're on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    view.window().map(|w| w.windowNumber())
}

#[cfg(not(target_os = "macos"))]
fn window_number(_window: &gpui::Window) -> Option<isize> {
    None
}
